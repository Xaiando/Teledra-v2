use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, Sink};
use std::collections::VecDeque;
use std::fmt;
use std::io::{self, BufRead, Read, Write};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const STDERR_CAPTURE_BYTES: usize = 32 * 1024;
const ERROR_DIAGNOSTIC_BYTES: usize = 6 * 1024;
const STDOUT_TAIL_PREVIEW_BYTES: usize = 256;
const PCM_CHANNEL_DEPTH: usize = 2;
const PLAYBACK_DRAIN_MS: u64 = 120;

#[derive(Clone, Copy, Debug)]
struct ProtocolLimits {
    min_sample_rate: u32,
    max_sample_rate: u32,
    max_frame_seconds: u64,
    max_total_seconds: u64,
}

impl Default for ProtocolLimits {
    fn default() -> Self {
        Self {
            min_sample_rate: 8_000,
            max_sample_rate: 192_000,
            max_frame_seconds: 90,
            max_total_seconds: 600,
        }
    }
}

#[derive(Debug, PartialEq)]
enum PcmProtocolError {
    TruncatedSampleRate {
        received: usize,
    },
    InvalidSampleRate {
        value: i32,
        min: u32,
        max: u32,
    },
    TruncatedFrameSize {
        received: usize,
        total_samples: u64,
    },
    InvalidFrameSize {
        value: i32,
    },
    FrameTooLarge {
        samples: u64,
        max_samples: u64,
    },
    TotalTooLarge {
        samples: u64,
        max_samples: u64,
    },
    TruncatedFrameData {
        expected_bytes: usize,
        received_bytes: usize,
    },
    NonFiniteSample {
        index: u64,
    },
    Io {
        stage: &'static str,
        message: String,
    },
}

impl fmt::Display for PcmProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedSampleRate { received } => write!(
                f,
                "truncated PCM protocol: sample-rate header ended after {received}/4 bytes"
            ),
            Self::InvalidSampleRate { value, min, max } => write!(
                f,
                "invalid PCM sample rate {value}; expected {min}..={max} Hz"
            ),
            Self::TruncatedFrameSize {
                received,
                total_samples,
            } => write!(
                f,
                "truncated PCM protocol: missing clean zero end marker after {total_samples} samples (received {received}/4 frame-header bytes)"
            ),
            Self::InvalidFrameSize { value } => {
                write!(f, "invalid negative PCM frame size {value}")
            }
            Self::FrameTooLarge {
                samples,
                max_samples,
            } => write!(
                f,
                "PCM frame has {samples} samples; bounded limit is {max_samples}"
            ),
            Self::TotalTooLarge {
                samples,
                max_samples,
            } => write!(
                f,
                "PCM stream has {samples} samples; bounded total is {max_samples}"
            ),
            Self::TruncatedFrameData {
                expected_bytes,
                received_bytes,
            } => write!(
                f,
                "truncated PCM frame: received {received_bytes}/{expected_bytes} payload bytes"
            ),
            Self::NonFiniteSample { index } => {
                write!(
                    f,
                    "PCM stream contains a non-finite sample at index {index}"
                )
            }
            Self::Io { stage, message } => {
                write!(f, "PCM pipe I/O failed while reading {stage}: {message}")
            }
        }
    }
}

fn read_exact_count<R: Read>(reader: &mut R, buffer: &mut [u8]) -> io::Result<usize> {
    let mut received = 0;
    while received < buffer.len() {
        match reader.read(&mut buffer[received..]) {
            Ok(0) => break,
            Ok(count) => received += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(received)
}

struct PcmProtocolReader<R: Read> {
    reader: R,
    limits: ProtocolLimits,
    sample_rate: Option<u32>,
    total_samples: u64,
    ended: bool,
}

impl<R: Read> PcmProtocolReader<R> {
    fn new(reader: R, limits: ProtocolLimits) -> Self {
        Self {
            reader,
            limits,
            sample_rate: None,
            total_samples: 0,
            ended: false,
        }
    }

    fn read_sample_rate(&mut self) -> Result<u32, PcmProtocolError> {
        if let Some(sample_rate) = self.sample_rate {
            return Ok(sample_rate);
        }

        let mut bytes = [0_u8; 4];
        let received = read_exact_count(&mut self.reader, &mut bytes).map_err(|error| {
            PcmProtocolError::Io {
                stage: "sample-rate header",
                message: error.to_string(),
            }
        })?;
        if received != bytes.len() {
            return Err(PcmProtocolError::TruncatedSampleRate { received });
        }

        let signed_rate = i32::from_le_bytes(bytes);
        if signed_rate < self.limits.min_sample_rate as i32
            || signed_rate > self.limits.max_sample_rate as i32
        {
            return Err(PcmProtocolError::InvalidSampleRate {
                value: signed_rate,
                min: self.limits.min_sample_rate,
                max: self.limits.max_sample_rate,
            });
        }
        let sample_rate = signed_rate as u32;
        self.sample_rate = Some(sample_rate);
        Ok(sample_rate)
    }

    /// Returns `Ok(None)` only after the explicit zero-sized clean end marker.
    fn next_frame(&mut self) -> Result<Option<Vec<f32>>, PcmProtocolError> {
        if self.ended {
            return Ok(None);
        }
        let sample_rate = self.read_sample_rate()?;

        let mut size_bytes = [0_u8; 4];
        let received = read_exact_count(&mut self.reader, &mut size_bytes).map_err(|error| {
            PcmProtocolError::Io {
                stage: "frame header",
                message: error.to_string(),
            }
        })?;
        if received != size_bytes.len() {
            return Err(PcmProtocolError::TruncatedFrameSize {
                received,
                total_samples: self.total_samples,
            });
        }

        let signed_samples = i32::from_le_bytes(size_bytes);
        if signed_samples == 0 {
            self.ended = true;
            return Ok(None);
        }
        if signed_samples < 0 {
            return Err(PcmProtocolError::InvalidFrameSize {
                value: signed_samples,
            });
        }
        let frame_samples = signed_samples as u64;
        let max_frame_samples =
            u64::from(sample_rate).saturating_mul(self.limits.max_frame_seconds);
        if frame_samples > max_frame_samples {
            return Err(PcmProtocolError::FrameTooLarge {
                samples: frame_samples,
                max_samples: max_frame_samples,
            });
        }

        let new_total = self.total_samples.checked_add(frame_samples).ok_or(
            PcmProtocolError::TotalTooLarge {
                samples: u64::MAX,
                max_samples: u64::from(sample_rate).saturating_mul(self.limits.max_total_seconds),
            },
        )?;
        let max_total_samples =
            u64::from(sample_rate).saturating_mul(self.limits.max_total_seconds);
        if new_total > max_total_samples {
            return Err(PcmProtocolError::TotalTooLarge {
                samples: new_total,
                max_samples: max_total_samples,
            });
        }

        let payload_bytes_u64 =
            frame_samples
                .checked_mul(4)
                .ok_or(PcmProtocolError::FrameTooLarge {
                    samples: frame_samples,
                    max_samples: max_frame_samples,
                })?;
        let payload_bytes =
            usize::try_from(payload_bytes_u64).map_err(|_| PcmProtocolError::FrameTooLarge {
                samples: frame_samples,
                max_samples: max_frame_samples,
            })?;
        let mut bytes = vec![0_u8; payload_bytes];
        let received = read_exact_count(&mut self.reader, &mut bytes).map_err(|error| {
            PcmProtocolError::Io {
                stage: "frame payload",
                message: error.to_string(),
            }
        })?;
        if received != payload_bytes {
            return Err(PcmProtocolError::TruncatedFrameData {
                expected_bytes: payload_bytes,
                received_bytes: received,
            });
        }

        let mut samples = Vec::with_capacity(frame_samples as usize);
        for (frame_index, chunk) in bytes.chunks_exact(4).enumerate() {
            let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if !sample.is_finite() {
                return Err(PcmProtocolError::NonFiniteSample {
                    index: self.total_samples + frame_index as u64,
                });
            }
            samples.push(sample);
        }
        self.total_samples = new_total;
        Ok(Some(samples))
    }

    fn into_inner(self) -> R {
        self.reader
    }
}

enum PcmStreamEvent {
    Header(u32),
    Frame(Vec<f32>),
    End,
    Failed(PcmProtocolError),
}

#[derive(Default)]
struct StdoutTail {
    bytes: u64,
    preview: Vec<u8>,
    drain_error: Option<String>,
}

fn read_pcm_stream(stdout: ChildStdout, sender: mpsc::SyncSender<PcmStreamEvent>) -> StdoutTail {
    let mut parser = PcmProtocolReader::new(stdout, ProtocolLimits::default());
    let sample_rate = match parser.read_sample_rate() {
        Ok(sample_rate) => sample_rate,
        Err(error) => {
            let _ = sender.send(PcmStreamEvent::Failed(error));
            return StdoutTail::default();
        }
    };
    if sender.send(PcmStreamEvent::Header(sample_rate)).is_err() {
        return StdoutTail::default();
    }

    loop {
        match parser.next_frame() {
            Ok(Some(samples)) => {
                if sender.send(PcmStreamEvent::Frame(samples)).is_err() {
                    return StdoutTail::default();
                }
            }
            Ok(None) => {
                if sender.send(PcmStreamEvent::End).is_err() {
                    return StdoutTail::default();
                }
                break;
            }
            Err(error) => {
                let _ = sender.send(PcmStreamEvent::Failed(error));
                return StdoutTail::default();
            }
        }
    }

    // Keep draining until the child exits. This prevents a backend's accidental
    // post-marker print from filling the pipe, while still reporting it as a
    // strict protocol violation after teardown.
    let mut reader = parser.into_inner();
    let mut tail = StdoutTail::default();
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                tail.bytes = tail.bytes.saturating_add(count as u64);
                let remaining = STDOUT_TAIL_PREVIEW_BYTES.saturating_sub(tail.preview.len());
                tail.preview
                    .extend_from_slice(&buffer[..count.min(remaining)]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                tail.drain_error = Some(error.to_string());
                break;
            }
        }
    }
    tail
}

#[derive(Default)]
struct StderrBuffer {
    lines: VecDeque<String>,
    bytes: usize,
}

impl StderrBuffer {
    fn push(&mut self, mut line: String) {
        if line.len() > STDERR_CAPTURE_BYTES {
            let mut start = line.len() - STDERR_CAPTURE_BYTES;
            while start < line.len() && !line.is_char_boundary(start) {
                start += 1;
            }
            line = line[start..].to_string();
        }
        self.bytes = self.bytes.saturating_add(line.len() + 1);
        self.lines.push_back(line);
        while self.bytes > STDERR_CAPTURE_BYTES && self.lines.len() > 1 {
            if let Some(removed) = self.lines.pop_front() {
                self.bytes = self.bytes.saturating_sub(removed.len() + 1);
            }
        }
    }

    fn tail(&self, max_bytes: usize) -> String {
        let joined = self.lines.iter().cloned().collect::<Vec<_>>().join("\n");
        if joined.len() <= max_bytes {
            return joined;
        }
        let mut start = joined.len() - max_bytes;
        while start < joined.len() && !joined.is_char_boundary(start) {
            start += 1;
        }
        format!("...{}", &joined[start..])
    }
}

fn read_stderr(
    stderr: ChildStderr,
    capture: Arc<Mutex<StderrBuffer>>,
    progress: Arc<dyn Fn(String) + Send + Sync>,
) {
    let reader = io::BufReader::new(stderr);
    for line in reader.lines() {
        match line {
            Ok(line) => {
                let line = line.trim_end().to_string();
                lock_unpoison(&capture).push(line.clone());
                if let Some(message) = line.strip_prefix("STATUS:") {
                    progress(message.to_string());
                } else if let Some(message) = line.strip_prefix("PROGRESS:") {
                    progress(message.to_string());
                }
            }
            Err(error) => {
                lock_unpoison(&capture).push(format!("stderr reader failed before EOF: {error}"));
                break;
            }
        }
    }
}

fn lock_unpoison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn terminate_owned_child(child: &mut Child) {
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn terminate_and_reap(
    shared_child: &Arc<Mutex<Option<Child>>>,
) -> Result<Option<ExitStatus>, String> {
    let child = lock_unpoison(shared_child).take();
    let Some(mut child) = child else {
        return Ok(None);
    };

    match child.try_wait() {
        Ok(Some(status)) => return Ok(Some(status)),
        Ok(None) => {
            let _ = child.kill();
        }
        Err(error) => {
            let _ = child.kill();
            return child.wait().map(Some).map_err(|wait_error| {
                format!("failed to inspect TTS child ({error}) and then reap it ({wait_error})")
            });
        }
    }
    child
        .wait()
        .map(Some)
        .map_err(|error| format!("failed to reap TTS child: {error}"))
}

fn wait_for_child_exit(
    shared_child: &Arc<Mutex<Option<Child>>>,
    cancelled: &AtomicBool,
    timeout: Duration,
) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Cancelled".to_string());
        }
        let status = {
            let mut guard = lock_unpoison(shared_child);
            let child = guard
                .as_mut()
                .ok_or_else(|| "TTS child handle disappeared before exit".to_string())?;
            child
                .try_wait()
                .map_err(|error| format!("failed to inspect TTS child status: {error}"))?
        };
        if let Some(status) = status {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "TTS child did not exit within {:.1}s after its clean end marker",
                timeout.as_secs_f32()
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn describe_exit(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "termination by signal or forced process stop".to_string(),
    }
}

fn configured_timeout(name: &str, default_seconds: u64) -> Duration {
    let seconds = std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| (1..=3_600).contains(value))
        .unwrap_or(default_seconds);
    Duration::from_secs(seconds)
}

#[derive(Clone, Copy)]
struct PlaybackTimeouts {
    startup: Duration,
    frame: Duration,
    total: Duration,
    child_exit: Duration,
}

impl PlaybackTimeouts {
    fn from_env() -> Self {
        Self {
            startup: configured_timeout("TELEDRA_TTS_STARTUP_TIMEOUT_SECS", 120),
            frame: configured_timeout("TELEDRA_TTS_FRAME_TIMEOUT_SECS", 180),
            total: configured_timeout("TELEDRA_TTS_TOTAL_TIMEOUT_SECS", 900),
            child_exit: configured_timeout("TELEDRA_TTS_EXIT_TIMEOUT_SECS", 15),
        }
    }
}

pub struct PlaybackController {
    child: Arc<Mutex<Option<Child>>>,
    sink: Arc<Mutex<Option<Arc<Sink>>>>,
    cancelled: Arc<AtomicBool>,
    token: Arc<()>,
}

impl Drop for PlaybackController {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let sink = lock_unpoison(&self.sink).take();
        if let Some(sink) = sink {
            sink.stop();
        }
        let _ = terminate_and_reap(&self.child);
    }
}

pub struct VoiceEngine {
    voice_name: String,
}

impl VoiceEngine {
    pub fn new(voice_name: &str) -> Self {
        VoiceEngine {
            voice_name: voice_name.to_string(),
        }
    }

    pub fn set_voice(&mut self, voice_name: &str) {
        self.voice_name = voice_name.to_string();
    }

    pub fn generate_and_play(
        &self,
        text: &str,
        active_playback: Arc<Mutex<Option<PlaybackController>>>,
        on_progress: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<(), String> {
        let workspace_root = std::env::var("TELEDRA_ROOT").unwrap_or_else(|_| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".to_string())
        });
        let python_exe = format!("{}\\.venv\\Scripts\\python.exe", workspace_root);
        let script_path = format!("{}\\generate_voice.py", workspace_root);

        let use_resident = std::env::var("TELEDRA_TTS_RESIDENT")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);

        let mut cmd = Command::new(python_exe);
        cmd.arg(script_path);
        if use_resident {
            // Exercise the warm-resident path in generate_voice.py (--resident mode).
            // The Python side now keeps the LuxTTS model loaded for the life of the process
            // and serves requests via `voice<TAB>text` on stdin.
            // Current behavior: still one process per utterance (latency unchanged).
            // True warm resident (keep child alive across utterances) is the follow-up micro-step.
            cmd.arg("--resident").stdin(std::process::Stdio::piped());
        } else {
            cmd.arg(text).arg(&self.voice_name);
        }
        // Model loaders use tqdm/Hugging Face progress bars during a cold
        // cache fill. They are diagnostics, not court UI, and must never
        // paint over ratatui's alternate-screen art panel.
        cmd.env("HF_HUB_DISABLE_PROGRESS_BARS", "1")
            .env("TQDM_DISABLE", "1")
            .env("TRANSFORMERS_VERBOSITY", "error")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd
            .spawn()
            .map_err(|error| format!("failed to spawn TTS child process: {error}"))?;

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_owned_child(&mut child);
                return Err("failed to capture TTS stdout".to_string());
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_owned_child(&mut child);
                return Err("failed to capture TTS stderr".to_string());
            }
        };

        // For resident launches we send the request over stdin instead of CLI args.
        if use_resident {
            if let Some(mut stdin) = child.stdin.take() {
                // The Python resident loop uses `sys.stdin.readline()`, which splits on `\n`.
                // If `text` contains internal newlines, the TTS worker will read only the
                // first line, treating subsequent lines as malformed commands (crashing the
                // worker without propagating the error to Rust). We must replace `\n` here.
                let single_line_text = text.replace('\n', " ");
                let request = format!("{}\t{}\n", self.voice_name, single_line_text);
                let _ = stdin.write_all(request.as_bytes());
                let _ = stdin.flush();
            }
        }

        let shared_child = Arc::new(Mutex::new(Some(child)));
        let shared_sink = Arc::new(Mutex::new(None));
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = Arc::new(());
        let controller = PlaybackController {
            child: Arc::clone(&shared_child),
            sink: Arc::clone(&shared_sink),
            cancelled: Arc::clone(&cancelled),
            token: Arc::clone(&token),
        };

        // Mark the entire startup phase as active before model loading begins.
        // Drop a replaced controller outside the mutex so process teardown can
        // never freeze the event loop while it tries to inspect this state.
        let previous = {
            let mut guard = lock_unpoison(&active_playback);
            guard.replace(controller)
        };
        drop(previous);

        let progress: Arc<dyn Fn(String) + Send + Sync> = Arc::new(on_progress);
        let stderr_capture = Arc::new(Mutex::new(StderrBuffer::default()));
        let stderr_thread = {
            let capture = Arc::clone(&stderr_capture);
            let progress = Arc::clone(&progress);
            std::thread::spawn(move || read_stderr(stderr, capture, progress))
        };
        let (pcm_sender, pcm_receiver) = mpsc::sync_channel(PCM_CHANNEL_DEPTH);
        let stdout_thread: JoinHandle<StdoutTail> =
            std::thread::spawn(move || read_pcm_stream(stdout, pcm_sender));

        let mut result = self.play_protocol(
            pcm_receiver,
            Arc::clone(&shared_child),
            Arc::clone(&shared_sink),
            Arc::clone(&cancelled),
            PlaybackTimeouts::from_env(),
        );

        // Whether playback succeeded or failed, own and reap the process before
        // joining its pipe readers. Killing closes blocked reads deterministically.
        let cleanup = terminate_and_reap(&shared_child);
        let stdout_tail = stdout_thread.join();
        let stderr_join = stderr_thread.join();

        let was_cancelled = matches!(&result, Err(message) if message == "Cancelled");
        if !was_cancelled {
            match cleanup {
                Ok(Some(status)) if !status.success() => {
                    if result.is_ok() {
                        result = Err(format!(
                            "TTS child reported a clean PCM stream but ended with {}",
                            describe_exit(status)
                        ));
                    } else if let Err(message) = &mut result
                        && !message.contains("exit code")
                    {
                        message.push_str(&format!("; child ended with {}", describe_exit(status)));
                    }
                }
                Ok(_) => {}
                Err(cleanup_error) => {
                    if let Err(message) = &mut result {
                        message.push_str(&format!("; {cleanup_error}"));
                    } else {
                        result = Err(cleanup_error);
                    }
                }
            }

            match stdout_tail {
                Ok(tail) => {
                    if let Some(drain_error) = tail.drain_error {
                        let message = format!("failed while draining TTS stdout: {drain_error}");
                        if let Err(existing) = &mut result {
                            existing.push_str(&format!("; {message}"));
                        } else {
                            result = Err(message);
                        }
                    }
                    if tail.bytes > 0 {
                        let preview = String::from_utf8_lossy(&tail.preview);
                        let message = format!(
                            "TTS protocol contained {} unexpected byte(s) after the clean end marker: {:?}",
                            tail.bytes, preview
                        );
                        if let Err(existing) = &mut result {
                            existing.push_str(&format!("; {message}"));
                        } else {
                            result = Err(message);
                        }
                    }
                }
                Err(_) => {
                    let message = "TTS stdout reader thread panicked".to_string();
                    if let Err(existing) = &mut result {
                        existing.push_str(&format!("; {message}"));
                    } else {
                        result = Err(message);
                    }
                }
            }

            if stderr_join.is_err() {
                if let Err(message) = &mut result {
                    message.push_str("; TTS stderr reader thread panicked");
                } else {
                    result = Err("TTS stderr reader thread panicked".to_string());
                }
            }

            if let Err(message) = &mut result {
                let diagnostics = lock_unpoison(&stderr_capture).tail(ERROR_DIAGNOSTIC_BYTES);
                if !diagnostics.is_empty() {
                    message.push_str("\nTTS diagnostics:\n");
                    message.push_str(&diagnostics);
                }
            }
        }

        // Clear only our own controller; a newer playback may already have
        // replaced it. Drop outside the lock to avoid waiting under the mutex.
        let ours = {
            let mut guard = lock_unpoison(&active_playback);
            let is_ours = guard
                .as_ref()
                .map(|controller| Arc::ptr_eq(&controller.token, &token))
                .unwrap_or(false);
            if is_ours { guard.take() } else { None }
        };
        drop(ours);

        result
    }

    fn play_protocol(
        &self,
        receiver: mpsc::Receiver<PcmStreamEvent>,
        shared_child: Arc<Mutex<Option<Child>>>,
        shared_sink: Arc<Mutex<Option<Arc<Sink>>>>,
        cancelled: Arc<AtomicBool>,
        timeouts: PlaybackTimeouts,
    ) -> Result<(), String> {
        let playback_gain = self.playback_gain();
        let started = Instant::now();
        let overall_deadline = started + timeouts.total;
        let mut last_protocol_event = started;
        let mut sample_rate = None;
        let mut sink: Option<Arc<Sink>> = None;
        let mut _output_stream: Option<OutputStream> = None;

        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err("Cancelled".to_string());
            }
            let stage_timeout = if sample_rate.is_none() {
                timeouts.startup
            } else {
                timeouts.frame
            };
            let stage_deadline = last_protocol_event + stage_timeout;
            let deadline = stage_deadline.min(overall_deadline);
            let now = Instant::now();
            if now >= deadline {
                let stage = if sample_rate.is_none() {
                    "startup/sample-rate header"
                } else {
                    "next PCM frame"
                };
                return Err(format!(
                    "TTS {stage} timed out after {:.1}s",
                    stage_timeout.as_secs_f32()
                ));
            }

            let event = match receiver.recv_timeout(deadline.saturating_duration_since(now)) {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let stage = if sample_rate.is_none() {
                        "startup/sample-rate header"
                    } else {
                        "next PCM frame"
                    };
                    return Err(format!(
                        "TTS {stage} timed out after {:.1}s",
                        stage_timeout.as_secs_f32()
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if cancelled.load(Ordering::Acquire) {
                        return Err("Cancelled".to_string());
                    }
                    return Err("TTS PCM reader stopped before a clean zero end marker".to_string());
                }
            };
            last_protocol_event = Instant::now();

            match event {
                PcmStreamEvent::Header(rate) => {
                    if sample_rate.replace(rate).is_some() {
                        return Err(
                            "TTS protocol sent more than one sample-rate header".to_string()
                        );
                    }
                    let (stream, stream_handle) = OutputStream::try_default()
                        .map_err(|error| format!("failed to open output audio stream: {error}"))?;
                    let new_sink = Arc::new(
                        Sink::try_new(&stream_handle)
                            .map_err(|error| format!("failed to create audio sink: {error}"))?,
                    );
                    if cancelled.load(Ordering::Acquire) {
                        new_sink.stop();
                        return Err("Cancelled".to_string());
                    }
                    *lock_unpoison(&shared_sink) = Some(Arc::clone(&new_sink));
                    _output_stream = Some(stream);
                    sink = Some(new_sink);
                }
                PcmStreamEvent::Frame(raw_samples) => {
                    let rate = sample_rate.ok_or_else(|| {
                        "TTS protocol delivered PCM before its sample-rate header".to_string()
                    })?;
                    let sink = sink.as_ref().ok_or_else(|| {
                        "TTS audio sink was unavailable after its sample-rate header".to_string()
                    })?;
                    let samples: Vec<f32> = raw_samples
                        .into_iter()
                        .map(|sample| (sample * playback_gain).clamp(-0.98, 0.98))
                        .collect();
                    sink.append(SamplesBuffer::new(1, rate, samples));
                }
                PcmStreamEvent::End => {
                    if sample_rate.is_none() {
                        return Err("TTS protocol ended before its sample-rate header".to_string());
                    }
                    break;
                }
                PcmStreamEvent::Failed(error) => {
                    if cancelled.load(Ordering::Acquire) {
                        return Err("Cancelled".to_string());
                    }
                    return Err(format!("invalid TTS PCM protocol: {error}"));
                }
            }
        }

        let remaining = overall_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!(
                "TTS session exceeded its {:.1}s total deadline",
                timeouts.total.as_secs_f32()
            ));
        }
        let status = wait_for_child_exit(
            &shared_child,
            &cancelled,
            timeouts.child_exit.min(remaining),
        )?;
        if !status.success() {
            return Err(format!("TTS child ended with {}", describe_exit(status)));
        }

        if let Some(sink) = sink {
            while !sink.empty() {
                if cancelled.load(Ordering::Acquire) {
                    sink.stop();
                    return Err("Cancelled".to_string());
                }
                if Instant::now() >= overall_deadline {
                    sink.stop();
                    return Err(format!(
                        "TTS playback exceeded its {:.1}s total deadline",
                        timeouts.total.as_secs_f32()
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            // Keep the Windows endpoint alive briefly after the bounded physical
            // silence from Python; this replaces the old 350ms blind sleep.
            std::thread::sleep(Duration::from_millis(PLAYBACK_DRAIN_MS));
        }
        Ok(())
    }

    pub fn voice_name(&self) -> &str {
        &self.voice_name
    }

    fn playback_gain(&self) -> f32 {
        match self.voice_name.as_str() {
            // Operator 2026-07-13: Organist and Alchemist were still far too
            // quiet at 3.25/2.45 (their reference voices are inherently soft);
            // the sample clamp at +-0.98 bounds any clipping from the boost.
            "organist" => 5.5,
            "artist" => 3.25,
            "scribe" => 2.55,
            "archivist" => 2.45,
            "alchemist" => 4.5,
            "orator" => 2.45,
            "diplomat" | "envoy" => 2.6,
            "treasurer" => 2.5,
            "wizard" => 2.45,
            "queen" | "teledra" | "energetic" => 1.22,
            _ => 1.55,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct FragmentedReader {
        inner: Cursor<Vec<u8>>,
        max_read: usize,
    }

    impl Read for FragmentedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let limit = buffer.len().min(self.max_read);
            self.inner.read(&mut buffer[..limit])
        }
    }

    fn push_frame(bytes: &mut Vec<u8>, samples: &[f32]) {
        bytes.extend_from_slice(&(samples.len() as i32).to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }

    fn parser_for(bytes: Vec<u8>) -> PcmProtocolReader<Cursor<Vec<u8>>> {
        PcmProtocolReader::new(Cursor::new(bytes), ProtocolLimits::default())
    }

    #[test]
    fn valid_protocol_requires_explicit_clean_end_marker() {
        let mut bytes = 48_000_i32.to_le_bytes().to_vec();
        push_frame(&mut bytes, &[0.0, 0.25, -0.5]);
        bytes.extend_from_slice(&0_i32.to_le_bytes());

        let mut parser = parser_for(bytes);
        assert_eq!(parser.read_sample_rate().unwrap(), 48_000);
        assert_eq!(parser.next_frame().unwrap(), Some(vec![0.0, 0.25, -0.5]));
        assert_eq!(parser.next_frame().unwrap(), None);
        assert_eq!(parser.next_frame().unwrap(), None);
    }

    #[test]
    fn fragmented_pipe_reads_are_reassembled() {
        let mut bytes = 48_000_i32.to_le_bytes().to_vec();
        push_frame(&mut bytes, &[0.1, -0.2]);
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        let reader = FragmentedReader {
            inner: Cursor::new(bytes),
            max_read: 1,
        };
        let mut parser = PcmProtocolReader::new(reader, ProtocolLimits::default());
        assert_eq!(parser.read_sample_rate().unwrap(), 48_000);
        assert_eq!(parser.next_frame().unwrap(), Some(vec![0.1, -0.2]));
        assert_eq!(parser.next_frame().unwrap(), None);
    }

    #[test]
    fn truncated_sample_rate_is_not_a_cancellation() {
        let mut parser = parser_for(vec![0x80, 0xbb]);
        assert_eq!(
            parser.read_sample_rate().unwrap_err(),
            PcmProtocolError::TruncatedSampleRate { received: 2 }
        );
    }

    #[test]
    fn physical_eof_without_zero_marker_is_truncated() {
        let bytes = 48_000_i32.to_le_bytes().to_vec();
        let mut parser = parser_for(bytes);
        parser.read_sample_rate().unwrap();
        assert_eq!(
            parser.next_frame().unwrap_err(),
            PcmProtocolError::TruncatedFrameSize {
                received: 0,
                total_samples: 0,
            }
        );
    }

    #[test]
    fn truncated_frame_payload_reports_exact_byte_count() {
        let mut bytes = 48_000_i32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&2_i32.to_le_bytes());
        bytes.extend_from_slice(&0.5_f32.to_le_bytes());
        let mut parser = parser_for(bytes);
        parser.read_sample_rate().unwrap();
        assert_eq!(
            parser.next_frame().unwrap_err(),
            PcmProtocolError::TruncatedFrameData {
                expected_bytes: 8,
                received_bytes: 4,
            }
        );
    }

    #[test]
    fn sample_rate_and_negative_frame_bounds_are_enforced() {
        let mut parser = parser_for(7_999_i32.to_le_bytes().to_vec());
        assert_eq!(
            parser.read_sample_rate().unwrap_err(),
            PcmProtocolError::InvalidSampleRate {
                value: 7_999,
                min: 8_000,
                max: 192_000,
            }
        );

        let mut bytes = 48_000_i32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&(-1_i32).to_le_bytes());
        let mut parser = parser_for(bytes);
        parser.read_sample_rate().unwrap();
        assert_eq!(
            parser.next_frame().unwrap_err(),
            PcmProtocolError::InvalidFrameSize { value: -1 }
        );
    }

    #[test]
    fn oversized_frame_is_rejected_before_allocation() {
        let limits = ProtocolLimits {
            max_frame_seconds: 1,
            ..ProtocolLimits::default()
        };
        let mut bytes = 8_000_i32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&8_001_i32.to_le_bytes());
        let mut parser = PcmProtocolReader::new(Cursor::new(bytes), limits);
        parser.read_sample_rate().unwrap();
        assert_eq!(
            parser.next_frame().unwrap_err(),
            PcmProtocolError::FrameTooLarge {
                samples: 8_001,
                max_samples: 8_000,
            }
        );
    }

    #[test]
    fn cumulative_audio_limit_is_enforced() {
        let limits = ProtocolLimits {
            max_frame_seconds: 1,
            max_total_seconds: 1,
            ..ProtocolLimits::default()
        };
        let mut bytes = 8_000_i32.to_le_bytes().to_vec();
        push_frame(&mut bytes, &vec![0.0; 5_000]);
        push_frame(&mut bytes, &vec![0.0; 4_000]);
        let mut parser = PcmProtocolReader::new(Cursor::new(bytes), limits);
        parser.read_sample_rate().unwrap();
        assert_eq!(parser.next_frame().unwrap().unwrap().len(), 5_000);
        assert_eq!(
            parser.next_frame().unwrap_err(),
            PcmProtocolError::TotalTooLarge {
                samples: 9_000,
                max_samples: 8_000,
            }
        );
    }

    #[test]
    fn nan_and_infinity_are_rejected() {
        for bad_sample in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut bytes = 48_000_i32.to_le_bytes().to_vec();
            push_frame(&mut bytes, &[0.0, bad_sample]);
            let mut parser = parser_for(bytes);
            parser.read_sample_rate().unwrap();
            assert_eq!(
                parser.next_frame().unwrap_err(),
                PcmProtocolError::NonFiniteSample { index: 1 }
            );
        }
    }
}
