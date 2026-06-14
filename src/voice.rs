use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, Sink};
use std::io::Read;
use std::process::Command;
use std::sync::{Arc, Mutex};

pub struct PlaybackController {
    pub child: std::process::Child,
    pub sink: Arc<Sink>,
}

impl Drop for PlaybackController {
    fn drop(&mut self) {
        // Reap the TTS child when the controller is cleared or replaced.
        // Without this, finished generate_voice.py processes were never
        // wait()ed and lingered as zombie handles. Only kill if it is still
        // alive after the audio stream has drained.
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
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
        // 1. (REMOVED) We no longer terminate ongoing playback; queuing in main.rs ensures sequential audio.

        let python_exe = "D:\\Teledra\\.venv\\Scripts\\python.exe";
        let script_path = "D:\\Teledra\\generate_voice.py";

        let mut cmd = Command::new(python_exe);
        cmd.arg(script_path)
            .arg(text)
            .arg(&self.voice_name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn tts child process: {}", e))?;

        let stdout = child.stdout.take().ok_or("Failed to capture tts stdout")?;
        let stderr = child.stderr.take().ok_or("Failed to capture tts stderr")?;

        let progress_cb = std::sync::Arc::new(on_progress);
        let progress_cb_clone = std::sync::Arc::clone(&progress_cb);

        std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stderr);
            for line in reader.lines() {
                if let Ok(text_line) = line {
                    if text_line.starts_with("STATUS:") {
                        let msg = text_line.trim_start_matches("STATUS:").to_string();
                        progress_cb_clone(msg);
                    } else if text_line.starts_with("PROGRESS:") {
                        let msg = text_line.trim_start_matches("PROGRESS:").to_string();
                        progress_cb_clone(msg);
                    }
                }
            }
        });

        // Play the audio using rodio (keep stream alive locally in generate_and_play stack frame)
        let (_stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| format!("Failed to open output audio stream: {}", e))?;

        let sink = Arc::new(
            Sink::try_new(&stream_handle)
                .map_err(|e| format!("Failed to create audio sink: {}", e))?,
        );

        // 2. Register new active playback controller IMMEDIATELY so she is not considered silent during startup
        {
            let mut guard = active_playback.lock().unwrap();
            *guard = Some(PlaybackController {
                child,
                sink: Arc::clone(&sink),
            });
        }

        // Run the main generation and playback loop
        let result = self.play_loop(stdout, &sink);

        // 3. Clear our controller from active playback if it hasn't been replaced
        {
            let mut guard = active_playback.lock().unwrap();
            let is_ours = if let Some(ref controller) = *guard {
                Arc::ptr_eq(&controller.sink, &sink)
            } else {
                false
            };
            if is_ours {
                *guard = None;
            }
        }

        result
    }

    fn play_loop(&self, mut stdout: std::process::ChildStdout, sink: &Sink) -> Result<(), String> {
        let playback_gain = self.playback_gain();

        // Read sample rate (first 4 bytes)
        let mut rate_buf = [0u8; 4];
        stdout
            .read_exact(&mut rate_buf)
            .map_err(|e| format!("Failed to read sample rate: {}", e))?;

        let sample_rate = i32::from_le_bytes(rate_buf) as u32;

        loop {
            let mut size_buf = [0u8; 4];
            if let Err(_) = stdout.read_exact(&mut size_buf) {
                return Err("Cancelled".to_string()); // Premature EOF: Process was killed
            }
            let num_samples = i32::from_le_bytes(size_buf) as usize;
            if num_samples == 0 {
                break; // EOF marker
            }

            let mut pcm_bytes = vec![0u8; num_samples * 4];
            if let Err(_) = stdout.read_exact(&mut pcm_bytes) {
                return Err("Cancelled".to_string());
            }

            let samples: Vec<f32> = pcm_bytes
                .chunks_exact(4)
                .map(|c| {
                    let sample = f32::from_le_bytes([c[0], c[1], c[2], c[3]]) * playback_gain;
                    sample.clamp(-0.98, 0.98)
                })
                .collect();

            let buffer = SamplesBuffer::new(1, sample_rate, samples);
            sink.append(buffer);
        }

        sink.sleep_until_end();
        // Give Windows audio a small drain window after the sink reports empty.
        // Without this, the last syllable can be clipped on some devices.
        std::thread::sleep(std::time::Duration::from_millis(350));
        Ok(())
    }

    pub fn voice_name(&self) -> &str {
        &self.voice_name
    }

    fn playback_gain(&self) -> f32 {
        match self.voice_name.as_str() {
            "organist" => 3.25,
            "artist" => 3.25,
            "scribe" => 2.55,
            "archivist" => 2.45,
            "alchemist" => 2.45,
            "orator" => 2.45,
            "diplomat" | "envoy" => 2.6,
            "treasurer" => 2.5,
            "queen" | "teledra" | "energetic" => 1.22,
            _ => 1.55,
        }
    }
}
