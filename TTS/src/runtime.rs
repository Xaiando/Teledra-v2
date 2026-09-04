//! Optional, session-long synthesis and playback runtime.
//!
//! The runtime owns both the warm Python worker and the audio device on one
//! background thread. Court/model work therefore never blocks on synthesis,
//! and rendered audio is appended to one Rodio sink so adjacent utterances
//! play continuously.

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use rodio::buffer::SamplesBuffer;
use rodio::cpal::traits::HostTrait;
use rodio::{DeviceTrait, OutputStream, Sink};
use thiserror::Error;

use crate::voice::Voice;
use crate::worker::{Synthesizer, WarmWorker};
use crate::{SpeechEvent, Utterance};

#[derive(Debug, Error)]
pub enum SpeechRuntimeError {
    #[error("the speech thread could not be started: {0}")]
    Thread(std::io::Error),
}

/// A non-blocking handle to the court's speech thread.
#[derive(Debug)]
pub struct SpeechRuntime {
    commands: Option<Sender<SpeechCommand>>,
    events: Receiver<SpeechEvent>,
    thread: Option<JoinHandle<()>>,
    worker_pid: Arc<AtomicU32>,
}

impl SpeechRuntime {
    pub fn start(worker_command: Command) -> Result<Self, SpeechRuntimeError> {
        Self::start_on(worker_command, None)
    }

    pub fn start_on(
        worker_command: Command,
        output_device: Option<String>,
    ) -> Result<Self, SpeechRuntimeError> {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker_pid = Arc::new(AtomicU32::new(0));
        let thread_pid = Arc::clone(&worker_pid);
        let thread = thread::Builder::new()
            .name("teledra-speech".to_owned())
            .spawn(move || {
                run(
                    worker_command,
                    output_device,
                    command_rx,
                    event_tx,
                    thread_pid,
                )
            })
            .map_err(SpeechRuntimeError::Thread)?;
        Ok(Self {
            commands: Some(command_tx),
            events: event_rx,
            thread: Some(thread),
            worker_pid,
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
        if utterance.text.is_empty() {
            return false;
        }
        self.commands
            .as_ref()
            .is_some_and(|sender| sender.send(SpeechCommand::Speak(utterance)).is_ok())
    }

    pub fn clear(&self) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|sender| sender.send(SpeechCommand::Clear).is_ok())
    }

    pub fn try_event(&self) -> Option<SpeechEvent> {
        self.events.try_recv().ok()
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(SpeechCommand::Shutdown);
        }
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
    events: Sender<SpeechEvent>,
    worker_pid: Arc<AtomicU32>,
) {
    let program = worker_command.get_program().to_owned();
    let args: Vec<_> = worker_command.get_args().map(|a| a.to_owned()).collect();
    let make_command = || {
        let mut cmd = Command::new(&program);
        cmd.args(&args);
        cmd
    };

    let mut worker = match WarmWorker::start_reporting(make_command(), Arc::clone(&worker_pid)) {
        Ok(worker) => worker,
        Err(error) => {
            worker_pid.store(0, Ordering::Release);
            let _ = events.send(SpeechEvent::Failed {
                voice: crate::VoiceName::Queen,
                reason: error.to_string(),
            });
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
            let _ = events.send(SpeechEvent::Failed {
                voice: crate::VoiceName::Queen,
                reason: format!("no audio output is available: {error}"),
            });
            worker.shutdown();
            worker_pid.store(0, Ordering::Release);
            return;
        }
    };
    let mut sink = match Sink::try_new(&stream_handle) {
        Ok(sink) => sink,
        Err(error) => {
            let _ = events.send(SpeechEvent::Failed {
                voice: crate::VoiceName::Queen,
                reason: format!("the audio output could not be opened: {error}"),
            });
            worker.shutdown();
            worker_pid.store(0, Ordering::Release);
            return;
        }
    };

    for command in commands {
        match command {
            SpeechCommand::Clear => {
                sink.stop();
                match Sink::try_new(&stream_handle) {
                    Ok(replacement) => sink = replacement,
                    Err(error) => {
                        let _ = events.send(SpeechEvent::Failed {
                            voice: crate::VoiceName::Queen,
                            reason: format!("the audio output could not be reopened: {error}"),
                        });
                        break;
                    }
                }
            }
            SpeechCommand::Speak(utterance) => {
                let voice = utterance.voice;
                if !worker.is_running() {
                    worker_pid.store(0, Ordering::Release);
                    if let Ok(new_worker) =
                        WarmWorker::start_reporting(make_command(), Arc::clone(&worker_pid))
                    {
                        worker = new_worker;
                    } else {
                        let _ = events.send(SpeechEvent::Failed {
                            voice,
                            reason: "speech worker stopped and could not be restarted".to_owned(),
                        });
                        break;
                    }
                }
                match worker.synthesize(voice, &utterance.text) {
                    Ok(mut audio) => {
                        Voice::new(voice).level(&mut audio.samples);
                        sink.append(SamplesBuffer::new(
                            1,
                            audio.sample_rate as u32,
                            audio.samples,
                        ));
                    }
                    Err(error) => {
                        let _ = events.send(SpeechEvent::Failed {
                            voice,
                            reason: error.to_string(),
                        });
                        if !worker.is_running() {
                            worker_pid.store(0, Ordering::Release);
                            if let Ok(new_worker) =
                                WarmWorker::start_reporting(make_command(), Arc::clone(&worker_pid))
                            {
                                worker = new_worker;
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
            SpeechCommand::Shutdown => break,
        }
    }

    // Session shutdown is deliberately immediate. Waiting for queued audio
    // here can hold the terminal open for minutes after the operator exits.
    sink.stop();
    worker.shutdown();
    worker_pid.store(0, Ordering::Release);
}

#[derive(Debug)]
enum SpeechCommand {
    Speak(Utterance),
    Clear,
    Shutdown,
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
