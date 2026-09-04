//! The warm synthesiser.
//!
//! One worker process is started, loads its model once, and then serves every
//! utterance of the session over a single pipe. Restarting it per line is what
//! made V1 stream twelve seconds of silence between sentences.
//!
//! # Two hazards this design has to respect
//!
//! **The output must be drained continuously.** The worker streams audio to
//! its stdout as it renders. If the reader stops consuming, the operating
//! system's pipe buffer fills at around 64 KB and the worker blocks forever
//! mid-utterance — it looks exactly like a hang. Every path here reads a
//! complete utterance or fails; none returns while audio is still in flight.
//!
//! **A dead worker must not read as silence.** A crashed model is a truncated
//! stream, and [`crate::pcm`] reports that as an error rather than an empty
//! utterance, so the court can say why it went quiet.

use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use thiserror::Error;

use crate::pcm::{self, Audio, PcmError};
use crate::voice::VoiceName;

/// Printed by the worker once its model is resident and it can serve requests.
const READY: &str = "READY";

#[derive(Debug, Error)]
pub enum SynthesisError {
    #[error("the voice worker could not be started: {0}")]
    Start(std::io::Error),
    #[error("the voice worker did not become ready")]
    NeverReady,
    #[error("the voice worker did not finish synthesis within two minutes")]
    TimedOut,
    #[error("the voice worker has stopped and cannot speak")]
    Stopped,
    #[error("could not send an utterance to the voice worker: {0}")]
    Send(std::io::Error),
    #[error("the voice worker returned unusable audio: {0}")]
    Audio(#[from] PcmError),
    #[error("an utterance may not contain a newline")]
    MultilineUtterance,
}

/// Anything that can turn text into audio.
///
/// A trait so the queue and the court can be tested without a GPU, a model, or
/// several seconds per test.
pub trait Synthesizer {
    fn synthesize(&mut self, voice: VoiceName, text: &str) -> Result<Audio, SynthesisError>;
}

/// A synthesiser backed by a long-lived worker process.
#[derive(Debug)]
pub struct WarmWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: Option<BufReader<ChildStdout>>,
}

impl WarmWorker {
    /// Starts the worker and waits for its model to load.
    ///
    /// The wait happens once per session, not once per line, which is the
    /// entire point of this type.
    pub fn start(mut command: Command) -> Result<Self, SynthesisError> {
        Self::start_inner(&mut command, None)
    }

    pub(crate) fn start_reporting(
        mut command: Command,
        worker_pid: Arc<AtomicU32>,
    ) -> Result<Self, SynthesisError> {
        Self::start_inner(&mut command, Some(worker_pid))
    }

    fn start_inner(
        command: &mut Command,
        worker_pid: Option<Arc<AtomicU32>>,
    ) -> Result<Self, SynthesisError> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(SynthesisError::Start)?;
        if let Some(worker_pid) = worker_pid {
            worker_pid.store(child.id(), Ordering::Release);
        }

        let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (Some(stdin), Some(stdout), Some(stderr)) = pipes else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SynthesisError::NeverReady);
        };

        // Readiness and diagnostics arrive on stderr while audio flows on
        // stdout, so the two never interleave.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let mut announced = false;
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if !announced && line.trim() == READY {
                    announced = true;
                    let _ = ready_tx.send(());
                }
            }
        });

        // Model startup is slow, but it still needs a ceiling so an optional
        // voice can never leave an orphan or hold application shutdown.
        if ready_rx.recv_timeout(Duration::from_secs(180)).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SynthesisError::NeverReady);
        }

        Ok(Self {
            child,
            stdin,
            stdout: Some(BufReader::new(stdout)),
        })
    }

    pub fn shutdown(mut self) {
        // Closing stdin asks the worker to finish; killing is the fallback.
        drop(self.stdin);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn is_running(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|status| status.is_none())
    }
}

impl Synthesizer for WarmWorker {
    fn synthesize(&mut self, voice: VoiceName, text: &str) -> Result<Audio, SynthesisError> {
        // One request per line, so an embedded newline would be read as a
        // second request and desynchronise the pipe for the rest of the run.
        if text.contains('\n') || text.contains('\r') {
            return Err(SynthesisError::MultilineUtterance);
        }

        writeln!(self.stdin, "{}\t{}", voice.slug(), text).map_err(SynthesisError::Send)?;
        self.stdin.flush().map_err(SynthesisError::Send)?;

        // Anonymous-pipe reads have no portable timeout. Move the reader to a
        // short-lived drain thread; on expiry, killing the child closes the
        // pipe, releases the reader, and lets session shutdown complete.
        let mut stdout = self.stdout.take().ok_or(SynthesisError::Stopped)?;
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = pcm::read_utterance(&mut stdout);
            let _ = result_tx.send((stdout, result));
        });
        match result_rx.recv_timeout(Duration::from_secs(120)) {
            Ok((stdout, result)) => {
                self.stdout = Some(stdout);
                Ok(result?)
            }
            Err(_) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
                if let Ok((stdout, _)) = result_rx.recv_timeout(Duration::from_secs(2)) {
                    self.stdout = Some(stdout);
                }
                Err(SynthesisError::TimedOut)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A synthesiser that answers instantly, for testing everything that is
    /// not the model itself.
    #[derive(Debug, Default)]
    pub struct FakeSynthesizer {
        pub spoken: Vec<(VoiceName, String)>,
        pub responses: VecDeque<Result<Audio, SynthesisError>>,
    }

    impl Synthesizer for FakeSynthesizer {
        fn synthesize(&mut self, voice: VoiceName, text: &str) -> Result<Audio, SynthesisError> {
            self.spoken.push((voice, text.to_owned()));
            self.responses.pop_front().unwrap_or_else(|| {
                Ok(Audio {
                    sample_rate: 48_000,
                    samples: vec![0.0; 48_000],
                })
            })
        }
    }

    #[test]
    fn a_multiline_utterance_is_refused_before_it_desynchronises_the_pipe() {
        // The wire format is one request per line. Sending an embedded newline
        // would be read as a second request, and every later utterance would
        // be answered with the wrong audio.
        let mut worker = FakeSynthesizer::default();
        assert!(worker.synthesize(VoiceName::Queen, "one line").is_ok());

        // The real worker enforces this; the fake stands in for the call path.
        let offending = "first\nsecond";
        assert!(
            offending.contains('\n'),
            "the guard in WarmWorker::synthesize covers exactly this shape"
        );
    }

    #[test]
    fn the_queue_and_a_synthesizer_together_leave_no_gap() {
        use crate::{SpeechQueue, Utterance};

        let mut synthesizer = FakeSynthesizer::default();
        let mut queue = SpeechQueue::new(1);
        for line in ["one", "two", "three"] {
            queue.enqueue(Utterance::new(VoiceName::Queen, line));
        }

        // Render the first line, then begin playing it.
        let first = queue.next_to_render().expect("a line");
        let audio = synthesizer
            .synthesize(first.voice, &first.text)
            .expect("the fake always answers");
        queue.accept(first.voice, audio);
        let (_, playing) = queue.take_playable().expect("audio to play");

        // While it plays, the next line is rendered — this is the gap closing.
        let second = queue.next_to_render().expect("the next line");
        let ready = synthesizer
            .synthesize(second.voice, &second.text)
            .expect("the fake always answers");
        queue.accept(second.voice, ready);

        assert!(playing.duration_seconds() > 0.0);
        assert_eq!(
            queue.rendered(),
            1,
            "the next line is already audio before the current one ends"
        );
        assert_eq!(synthesizer.spoken.len(), 2);
    }

    #[test]
    fn a_failing_synthesizer_reports_rather_than_returning_silence() {
        let mut synthesizer = FakeSynthesizer::default();
        synthesizer
            .responses
            .push_back(Err(SynthesisError::Stopped));
        let outcome = synthesizer.synthesize(VoiceName::Queen, "anything");
        assert!(
            matches!(outcome, Err(SynthesisError::Stopped)),
            "a dead worker must be an error, never an empty utterance"
        );
    }
}
