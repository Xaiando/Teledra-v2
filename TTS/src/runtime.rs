//! Optional, session-long synthesis and playback runtime.
//!
//! The runtime owns both the warm Python worker and the audio device on one
//! background thread. Court/model work therefore never blocks on synthesis,
//! and rendered audio is appended to one Rodio sink so adjacent utterances
//! play continuously.

use std::process::Command;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rodio::buffer::SamplesBuffer;
use rodio::cpal::traits::HostTrait;
use rodio::{DeviceTrait, OutputStream, Sink, Source};
use thiserror::Error;

use crate::voice::Voice;
use crate::worker::{Synthesizer, WarmWorker};
use crate::{SpeechEvent, Utterance, VoiceName};

/// The current utterance and one synthesized look-ahead keep the conversation
/// continuous without letting text generation run minutes ahead of the voice.
const MAX_PENDING: usize = 2;
type RuntimeEvent = (Option<u64>, SpeechEvent);

#[derive(Debug, Error)]
pub enum SpeechRuntimeError {
    #[error("the speech thread could not be started: {0}")]
    Thread(std::io::Error),
}

/// A non-blocking handle to the court's speech thread.
#[derive(Debug)]
pub struct SpeechRuntime {
    commands: Option<SyncSender<SpeechCommand>>,
    events: Receiver<RuntimeEvent>,
    thread: Option<JoinHandle<()>>,
    worker_pid: Arc<AtomicU32>,
    playback: Arc<PlaybackControl>,
}

#[derive(Default)]
struct PlaybackControl {
    generation: AtomicU64,
    pending: Mutex<PendingState>,
    sink: Mutex<Option<Arc<Sink>>>,
}

#[derive(Default)]
struct PendingState {
    generation: u64,
    count: usize,
}

impl std::fmt::Debug for PlaybackControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackControl")
            .field("generation", &self.generation.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl PlaybackControl {
    fn clear(&self) {
        let mut guard = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending.generation = pending.generation.wrapping_add(1);
            pending.count = 0;
            self.generation.store(pending.generation, Ordering::Release);
        }
        if let Some(sink) = guard.take() {
            sink.stop();
        }
    }

    fn admit(self: &Arc<Self>) -> Option<PendingUtterance> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.count >= MAX_PENDING {
            return None;
        }
        pending.count += 1;
        Some(PendingUtterance {
            generation: pending.generation,
            playback: Arc::clone(self),
        })
    }

    fn pending_count(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).count
    }
}

/// Owns one admission from the command queue through synthesis and playback.
/// Dropping obsolete work must never decrement a newer generation's count.
#[derive(Debug)]
struct PendingUtterance {
    generation: u64,
    playback: Arc<PlaybackControl>,
}

impl Drop for PendingUtterance {
    fn drop(&mut self) {
        let mut pending = self
            .playback
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if pending.generation == self.generation {
            pending.count -= 1;
        }
    }
}

impl SpeechRuntime {
    pub fn start(worker_command: Command) -> Result<Self, SpeechRuntimeError> {
        Self::start_on(worker_command, None)
    }

    pub fn start_on(
        worker_command: Command,
        output_device: Option<String>,
    ) -> Result<Self, SpeechRuntimeError> {
        let (command_tx, command_rx) = mpsc::sync_channel(4);
        let (event_tx, event_rx) = mpsc::channel();
        let worker_pid = Arc::new(AtomicU32::new(0));
        let thread_pid = Arc::clone(&worker_pid);
        let playback = Arc::new(PlaybackControl::default());
        let thread_playback = Arc::clone(&playback);
        let thread = thread::Builder::new()
            .name("teledra-speech".to_owned())
            .spawn(move || {
                run(
                    worker_command,
                    output_device,
                    command_rx,
                    event_tx,
                    thread_pid,
                    thread_playback,
                )
            })
            .map_err(SpeechRuntimeError::Thread)?;
        Ok(Self {
            commands: Some(command_tx),
            events: event_rx,
            thread: Some(thread),
            worker_pid,
            playback,
        })
    }

    pub fn speak(&self, mut utterance: Utterance) -> bool {
        // The worker protocol is one request per line. Court replies are
        // commonly multi-paragraph, so flatten presentation whitespace here
        // without changing the words the court committed to speak.
        utterance.text = utterance
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if utterance.text.is_empty() || utterance.text.chars().count() > 6000 {
            return false;
        }
        let Some(pending) = self.playback.admit() else {
            return false;
        };
        self.commands.as_ref().is_some_and(|sender| {
            sender
                .try_send(SpeechCommand::Speak(utterance, pending))
                .is_ok()
        })
    }

    pub fn clear(&self) -> bool {
        self.playback.clear();
        self.commands.is_some()
    }

    pub fn try_event(&self) -> Option<SpeechEvent> {
        while let Ok((generation, event)) = self.events.try_recv() {
            if generation.is_none_or(|generation| {
                generation == self.playback.generation.load(Ordering::Acquire)
            }) {
                return Some(event);
            }
        }
        None
    }

    /// All accepted utterances, including queued, synthesizing and playing.
    /// Admission is limited to two, so callers can pause generation at capacity.
    pub fn pending_count(&self) -> usize {
        self.playback.pending_count()
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.playback.clear();
        self.commands.take();
        if let Some(thread) = self.thread.take() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !thread.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if !thread.is_finished() {
                terminate_worker(self.worker_pid.load(Ordering::Acquire));
            }
            let _ = thread.join();
        }
    }
}

impl Drop for SpeechRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    worker_command: Command,
    output_device: Option<String>,
    commands: Receiver<SpeechCommand>,
    events: Sender<RuntimeEvent>,
    worker_pid: Arc<AtomicU32>,
    playback: Arc<PlaybackControl>,
) {
    let program = worker_command.get_program().to_owned();
    let args: Vec<_> = worker_command.get_args().map(|a| a.to_owned()).collect();
    let environment: Vec<_> = worker_command
        .get_envs()
        .map(|(key, value)| (key.to_owned(), value.map(|v| v.to_owned())))
        .collect();
    let directory = worker_command.get_current_dir().map(|path| path.to_owned());
    let make_command = || {
        let mut cmd = Command::new(&program);
        cmd.args(&args);
        for (key, value) in &environment {
            match value {
                Some(value) => {
                    cmd.env(key, value);
                }
                None => {
                    cmd.env_remove(key);
                }
            }
        }
        if let Some(directory) = &directory {
            cmd.current_dir(directory);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        cmd
    };

    let mut worker = match WarmWorker::start_reporting(make_command(), Arc::clone(&worker_pid)) {
        Ok(worker) => worker,
        Err(error) => {
            worker_pid.store(0, Ordering::Release);
            let _ = events.send((
                None,
                SpeechEvent::Failed {
                    id: None,
                    voice: crate::VoiceName::Queen,
                    reason: error.to_string(),
                },
            ));
            return;
        }
    };
    let stream = output_device
        .as_deref()
        .map(open_named_output)
        .unwrap_or_else(OutputStream::try_default);
    let (_stream, stream_handle) = match stream {
        Ok(stream) => stream,
        Err(error) => {
            let _ = events.send((
                None,
                SpeechEvent::Failed {
                    id: None,
                    voice: crate::VoiceName::Queen,
                    reason: format!("no audio output is available: {error}"),
                },
            ));
            worker.shutdown();
            worker_pid.store(0, Ordering::Release);
            return;
        }
    };
    let sink = match Sink::try_new(&stream_handle) {
        Ok(sink) => Arc::new(sink),
        Err(error) => {
            let _ = events.send((
                None,
                SpeechEvent::Failed {
                    id: None,
                    voice: crate::VoiceName::Queen,
                    reason: format!("the audio output could not be opened: {error}"),
                },
            ));
            worker.shutdown();
            worker_pid.store(0, Ordering::Release);
            return;
        }
    };
    *playback.sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);

    let mut terminal_failure = None;
    for command in commands {
        match command {
            SpeechCommand::Speak(utterance, pending) => {
                let generation = pending.generation;
                if generation != playback.generation.load(Ordering::Acquire) {
                    continue;
                }
                let voice = utterance.voice;
                let fail = |reason: String| {
                    send_event(
                        &events,
                        &playback,
                        generation,
                        SpeechEvent::Failed {
                            id: Some(utterance.id),
                            voice,
                            reason,
                        },
                    );
                };
                if !worker.is_running() {
                    worker_pid.store(0, Ordering::Release);
                    if let Ok(new_worker) =
                        WarmWorker::start_reporting(make_command(), Arc::clone(&worker_pid))
                    {
                        worker = new_worker;
                    } else {
                        terminal_failure =
                            Some("speech worker stopped and could not be restarted".to_owned());
                        break;
                    }
                }
                match worker.synthesize(voice, &utterance.text) {
                    Ok(mut audio) => {
                        if audio.is_silent() {
                            fail("The speech worker returned no audio".to_owned());
                            continue;
                        }
                        // Interruption may have happened while the model was
                        // synthesizing. Old audio must never reappear afterward.
                        let mut guard = playback.sink.lock().unwrap_or_else(|e| e.into_inner());
                        if generation != playback.generation.load(Ordering::Acquire) {
                            continue;
                        }
                        if guard.is_none() {
                            match Sink::try_new(&stream_handle) {
                                Ok(sink) => *guard = Some(Arc::new(sink)),
                                Err(error) => {
                                    terminal_failure = Some(error.to_string());
                                    break;
                                }
                            }
                        }
                        let sink = guard.as_ref().expect("playback sink was created");
                        Voice::new(voice)
                            .level_at_rate(&mut audio.samples, audio.sample_rate as u32);
                        sink.append(PlaybackSource::new(
                            utterance.id,
                            voice,
                            audio,
                            pending,
                            events.clone(),
                        ));
                    }
                    Err(error) => {
                        fail(error.to_string());
                        if !worker.is_running() {
                            worker_pid.store(0, Ordering::Release);
                            if let Ok(new_worker) =
                                WarmWorker::start_reporting(make_command(), Arc::clone(&worker_pid))
                            {
                                worker = new_worker;
                            } else {
                                terminal_failure = Some(
                                    "speech worker stopped and could not be restarted".to_owned(),
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // Session shutdown is deliberately immediate. Waiting for queued audio
    // here can hold the terminal open for minutes after the operator exits.
    end_playback(&playback, &events, terminal_failure);
    worker.shutdown();
    worker_pid.store(0, Ordering::Release);
}

fn end_playback(
    playback: &PlaybackControl,
    events: &Sender<RuntimeEvent>,
    terminal_failure: Option<String>,
) {
    playback.clear();
    if let Some(reason) = terminal_failure {
        // Clearing playback invalidates all individual callbacks. A terminal
        // failure is runtime-wide and must still reach the caller afterward.
        let _ = events.send((
            None,
            SpeechEvent::Failed {
                id: None,
                voice: VoiceName::Queen,
                reason,
            },
        ));
    }
}

#[derive(Debug)]
enum SpeechCommand {
    Speak(Utterance, PendingUtterance),
}

fn send_event(
    events: &Sender<RuntimeEvent>,
    playback: &PlaybackControl,
    generation: u64,
    event: SpeechEvent,
) {
    if generation == playback.generation.load(Ordering::Acquire) {
        let _ = events.send((Some(generation), event));
    }
}

/// Emits caption timing from samples consumed by Rodio, not synthesis time or
/// time spent waiting behind the preceding utterance in the sink.
struct PlaybackSource {
    samples: SamplesBuffer<f32>,
    id: u64,
    voice: VoiceName,
    seconds: f64,
    consumed: u64,
    progress_interval: u64,
    pending: Option<PendingUtterance>,
    events: Sender<RuntimeEvent>,
}

impl PlaybackSource {
    fn new(
        id: u64,
        voice: VoiceName,
        audio: crate::Audio,
        pending: PendingUtterance,
        events: Sender<RuntimeEvent>,
    ) -> Self {
        Self {
            seconds: audio.duration_seconds(),
            progress_interval: (audio.sample_rate as u64 / 10).max(1),
            samples: SamplesBuffer::new(1, audio.sample_rate as u32, audio.samples),
            id,
            voice,
            consumed: 0,
            pending: Some(pending),
            events,
        }
    }

    fn emit(&self, event: SpeechEvent) {
        if let Some(pending) = &self.pending {
            send_event(&self.events, &pending.playback, pending.generation, event);
        }
    }
}

impl Iterator for PlaybackSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let pending = self.pending.as_ref()?;
        if pending.generation != pending.playback.generation.load(Ordering::Acquire) {
            self.pending.take();
            return None;
        }
        let sample = self.samples.next();
        if sample.is_some() {
            if self.consumed == 0 {
                self.emit(SpeechEvent::Started {
                    id: self.id,
                    voice: self.voice,
                    seconds: self.seconds,
                });
            }
            self.consumed += 1;
            if self.consumed.is_multiple_of(self.progress_interval) {
                self.emit(SpeechEvent::Progress {
                    id: self.id,
                    voice: self.voice,
                    elapsed: self.consumed as f64 / f64::from(self.samples.sample_rate()),
                    seconds: self.seconds,
                });
            }
        } else {
            self.emit(SpeechEvent::Finished {
                id: self.id,
                voice: self.voice,
                seconds: self.seconds,
            });
            self.pending.take();
        }
        sample
    }
}

impl Source for PlaybackSource {
    fn current_frame_len(&self) -> Option<usize> {
        self.samples.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.samples.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.samples.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.samples.total_duration()
    }
}

fn open_named_output(
    name: &str,
) -> Result<(OutputStream, rodio::OutputStreamHandle), rodio::StreamError> {
    let wanted = name.trim().to_ascii_lowercase();
    let device = rodio::cpal::default_host()
        .output_devices()
        .ok()
        .and_then(|mut devices| {
            devices.find(|device| {
                device
                    .name()
                    .is_ok_and(|candidate| candidate.to_ascii_lowercase().contains(&wanted))
            })
        });
    match device {
        Some(device) => OutputStream::try_from_device(&device),
        None => Err(rodio::StreamError::NoDevice),
    }
}

pub fn output_devices() -> Vec<String> {
    rodio::cpal::default_host()
        .output_devices()
        .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
        .unwrap_or_default()
}

#[cfg(windows)]
fn terminate_worker(pid: u32) {
    use std::os::windows::process::CommandExt;
    if pid == 0 {
        return;
    }
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = Command::new("taskkill.exe")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(not(windows))]
fn terminate_worker(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires installed Python and an audio device; plays only silence"]
    fn operational_speech_interrupt_restart_and_shutdown() {
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let directory = workspace.join("target/astra-verification").join(format!(
            "speech-runtime-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let mut command = Command::new(workspace.join(".venv/Scripts/python.exe"));
        command
            .arg(workspace.join("TTS/worker/probe_runtime.py"))
            .env("TELEDRA_PROBE_DIR", &directory)
            .env("TELEDRA_PROBE_MARKER", "preserved")
            .current_dir(&directory);
        let mut runtime = SpeechRuntime::start(command).unwrap();
        let wait = |predicate: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(12);
            while !predicate() {
                assert!(
                    Instant::now() < deadline,
                    "speech operational check timed out"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&|| runtime.playback.sink.lock().unwrap().is_some());
        // Queue two short silent sources on the real device. Their captions
        // must follow sample consumption in sequence, not both fire at append.
        let (timing_tx, timing_events) = mpsc::channel();
        let before = Instant::now();
        for id in [100, 101] {
            let source = PlaybackSource::new(
                id,
                VoiceName::Queen,
                crate::Audio {
                    sample_rate: 48_000,
                    samples: vec![0.0; 19_200],
                },
                runtime.playback.admit().unwrap(),
                timing_tx.clone(),
            );
            runtime
                .playback
                .sink
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .append(source);
        }
        assert_eq!(runtime.pending_count(), 2);
        assert!(runtime.playback.admit().is_none());
        for id in [100, 101] {
            let mut started = false;
            let mut progress = 0.0;
            loop {
                assert!(before.elapsed() < Duration::from_secs(3));
                match timing_events.try_recv().ok().map(|(_, event)| event) {
                    Some(SpeechEvent::Started {
                        id: actual,
                        seconds,
                        ..
                    }) => {
                        assert_eq!(actual, id);
                        assert_eq!(seconds, 0.4);
                        assert!(!started);
                        started = true;
                    }
                    Some(SpeechEvent::Progress { elapsed, .. }) => {
                        assert!(started);
                        assert!(elapsed > progress);
                        progress = elapsed;
                    }
                    Some(SpeechEvent::Finished {
                        id: actual,
                        seconds,
                        ..
                    }) => {
                        assert_eq!(actual, id);
                        assert_eq!(seconds, 0.4);
                        assert!(progress >= 0.3);
                        let minimum = if id == 100 { 250 } else { 650 };
                        assert!(before.elapsed() >= Duration::from_millis(minimum));
                        break;
                    }
                    Some(event) => panic!("unexpected playback event: {event:?}"),
                    None => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        }
        assert_eq!(runtime.pending_count(), 0);
        assert!(runtime.speak(Utterance::new(crate::VoiceName::Queen, "empty")));
        wait(
            &|| matches!(runtime.try_event(), Some(SpeechEvent::Failed { reason, .. }) if reason.contains("no audio")),
        );
        assert!(runtime.speak(Utterance::new(crate::VoiceName::Queen, "slow")));
        wait(&|| directory.join("slow.started").exists());
        let first_sink = runtime
            .playback
            .sink
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        runtime.clear();
        assert!(runtime.speak(Utterance::new(crate::VoiceName::Queen, "current")));
        wait(&|| directory.join("current.started").exists());
        wait(&|| {
            runtime
                .playback
                .sink
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|sink| sink.len() == 1)
        });
        assert!(first_sink.empty(), "interrupted sink still contains speech");
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(
            runtime
                .playback
                .sink
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .len(),
            1,
            "in-flight speech reappeared after interruption"
        );
        let first_pid = runtime.worker_pid.load(Ordering::Acquire);
        assert!(runtime.speak(Utterance::new(crate::VoiceName::Queen, "crash")));
        wait(&|| {
            runtime.worker_pid.load(Ordering::Acquire) != first_pid
                && std::fs::read_to_string(directory.join("starts.log"))
                    .is_ok_and(|s| s.lines().count() == 2)
        });
        assert!(runtime.speak(Utterance::new(crate::VoiceName::Queen, "restarted")));
        wait(&|| directory.join("restarted.started").exists());
        wait(&|| {
            runtime
                .playback
                .sink
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|sink| sink.len() == 2)
        });
        assert_eq!(runtime.pending_count(), 2);
        assert!(!runtime.speak(Utterance::new(crate::VoiceName::Queen, "beyond-lookahead")));
        runtime.clear();
        assert!(runtime.speak(Utterance::new(crate::VoiceName::Queen, "corrupt")));
        wait(&|| {
            std::fs::read_to_string(directory.join("starts.log"))
                .is_ok_and(|s| s.lines().count() == 3)
        });
        assert!(runtime.speak(Utterance::new(crate::VoiceName::Queen, "recovered")));
        wait(&|| {
            runtime
                .playback
                .sink
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|sink| sink.len() == 1)
        });
        let started = Instant::now();
        runtime.stop();
        assert!(started.elapsed() < Duration::from_secs(4));
        assert_eq!(runtime.worker_pid.load(Ordering::Acquire), 0);
    }

    #[test]
    fn a_full_speech_queue_never_blocks_the_court_and_clear_invalidates_old_work() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (_event_tx, events) = mpsc::channel();
        let runtime = SpeechRuntime {
            commands: Some(tx),
            events,
            thread: None,
            worker_pid: Arc::new(AtomicU32::new(0)),
            playback: Arc::new(PlaybackControl::default()),
        };
        assert!(runtime.speak(Utterance::new(crate::VoiceName::Queen, "First line")));
        assert!(!runtime.speak(Utterance::new(crate::VoiceName::Queen, "Queue full")));
        runtime.clear();
        let SpeechCommand::Speak(_, old_pending) = rx.try_recv().unwrap();
        assert_ne!(
            old_pending.generation,
            runtime.playback.generation.load(Ordering::Acquire)
        );
        assert_eq!(runtime.pending_count(), 0);
        assert!(runtime.speak(Utterance::new(crate::VoiceName::Queen, "New line")));
        let SpeechCommand::Speak(_, pending) = rx.try_recv().unwrap();
        assert_eq!(
            pending.generation,
            runtime.playback.generation.load(Ordering::Acquire)
        );
        drop(old_pending);
        assert_eq!(runtime.pending_count(), 1);
        drop(pending);
        assert_eq!(runtime.pending_count(), 0);
    }

    fn test_runtime(
        capacity: usize,
    ) -> (SpeechRuntime, Receiver<SpeechCommand>, Sender<RuntimeEvent>) {
        let (tx, commands) = mpsc::sync_channel(capacity);
        let (events, rx) = mpsc::channel();
        let runtime = SpeechRuntime {
            commands: Some(tx),
            events: rx,
            thread: None,
            worker_pid: Arc::new(AtomicU32::new(0)),
            playback: Arc::new(PlaybackControl::default()),
        };
        (runtime, commands, events)
    }

    fn test_source(
        commands: &Receiver<SpeechCommand>,
        events: &Sender<RuntimeEvent>,
    ) -> PlaybackSource {
        let SpeechCommand::Speak(utterance, pending) = commands.try_recv().unwrap();
        PlaybackSource::new(
            utterance.id,
            utterance.voice,
            crate::Audio {
                sample_rate: 48_000,
                samples: vec![0.0; 12_000],
            },
            pending,
            events.clone(),
        )
    }

    #[test]
    fn captions_start_on_consumption_and_follow_pcm_duration() {
        let (runtime, commands, events) = test_runtime(4);
        assert!(runtime.speak(Utterance::new(VoiceName::Queen, "First").with_id(42)));
        assert!(runtime.speak(Utterance::new(VoiceName::Organist, "Second").with_id(43)));
        let mut first = test_source(&commands, &events);
        let mut second = test_source(&commands, &events);
        assert_eq!(runtime.pending_count(), 2);
        assert!(!runtime.speak(Utterance::new(VoiceName::Artist, "Too far ahead")));
        assert!(
            runtime.try_event().is_none(),
            "preparing PCM is not playback"
        );

        assert_eq!(first.next(), Some(0.0));
        assert_eq!(
            runtime.try_event(),
            Some(SpeechEvent::Started {
                id: 42,
                voice: VoiceName::Queen,
                seconds: 0.25,
            })
        );
        assert!(runtime.try_event().is_none());
        for _ in 1..4_800 {
            assert_eq!(first.next(), Some(0.0));
        }
        assert_eq!(
            runtime.try_event(),
            Some(SpeechEvent::Progress {
                id: 42,
                voice: VoiceName::Queen,
                elapsed: 0.1,
                seconds: 0.25,
            })
        );
        assert_eq!(first.by_ref().count(), 7_200);
        assert_eq!(
            runtime.try_event(),
            Some(SpeechEvent::Progress {
                id: 42,
                voice: VoiceName::Queen,
                elapsed: 0.2,
                seconds: 0.25,
            })
        );
        assert_eq!(
            runtime.try_event(),
            Some(SpeechEvent::Finished {
                id: 42,
                voice: VoiceName::Queen,
                seconds: 0.25,
            })
        );
        assert_eq!(first.next(), None);
        assert!(runtime.try_event().is_none(), "completion is emitted once");
        assert_eq!(runtime.pending_count(), 1);
        assert_eq!(second.next(), Some(0.0));
        assert_eq!(
            runtime.try_event(),
            Some(SpeechEvent::Started {
                id: 43,
                voice: VoiceName::Organist,
                seconds: 0.25,
            })
        );
        drop(second);
        assert_eq!(runtime.pending_count(), 0);
        assert!(
            runtime.try_event().is_none(),
            "dropping partial audio is not a finish"
        );
    }

    #[test]
    fn clear_suppresses_buffered_events_and_stale_sources_without_stealing_new_capacity() {
        let (runtime, commands, events) = test_runtime(4);
        assert!(runtime.speak(Utterance::new(VoiceName::Queen, "Obsolete").with_id(1)));
        let mut obsolete = test_source(&commands, &events);
        assert_eq!(obsolete.next(), Some(0.0));
        runtime.clear();
        assert!(
            runtime.try_event().is_none(),
            "old start was buffered before clear"
        );
        assert!(runtime.speak(Utterance::new(VoiceName::Queen, "Current").with_id(2)));
        assert_eq!(obsolete.next(), None, "old audio stops immediately");
        assert_eq!(runtime.pending_count(), 1);
        assert!(runtime.try_event().is_none());
        drop(commands.try_recv().unwrap());
        assert_eq!(runtime.pending_count(), 0);
    }

    #[test]
    fn failed_admission_and_worker_disconnect_release_their_capacity() {
        let (runtime, commands, _) = test_runtime(1);
        assert!(!runtime.speak(Utterance::new(VoiceName::Queen, " \n ")));
        assert!(runtime.speak(Utterance::new(VoiceName::Queen, "Accepted")));
        assert!(!runtime.speak(Utterance::new(VoiceName::Queen, "Channel full")));
        assert_eq!(runtime.pending_count(), 1);
        drop(commands);
        assert_eq!(runtime.pending_count(), 0);
        assert!(!runtime.speak(Utterance::new(VoiceName::Queen, "Worker gone")));
        assert_eq!(runtime.pending_count(), 0);
    }

    #[test]
    fn fatal_shutdown_reports_all_pending_captions_after_invalidating_playback() {
        let (runtime, commands, events) = test_runtime(4);
        assert!(runtime.speak(Utterance::new(VoiceName::Queen, "Playing").with_id(1)));
        assert!(runtime.speak(Utterance::new(VoiceName::Organist, "Look ahead").with_id(2)));
        let mut playing = test_source(&commands, &events);
        assert_eq!(playing.next(), Some(0.0));
        end_playback(
            &runtime.playback,
            &events,
            Some("worker restart failed".to_owned()),
        );
        assert_eq!(runtime.pending_count(), 0);
        assert_eq!(
            runtime.try_event(),
            Some(SpeechEvent::Failed {
                id: None,
                voice: VoiceName::Queen,
                reason: "worker restart failed".to_owned(),
            })
        );
        assert_eq!(playing.next(), None);
        drop(commands);
        assert!(runtime.try_event().is_none());
        assert_eq!(runtime.pending_count(), 0);

        end_playback(&runtime.playback, &events, None);
        assert!(
            runtime.try_event().is_none(),
            "normal shutdown is not a failure"
        );
    }
}
