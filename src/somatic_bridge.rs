use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SomaticState {
    pub face_detected: bool,
    pub hands_detected: bool,
    pub shoulder_asymmetry: Option<f32>,
    pub error: Option<String>,
}

impl SomaticState {
    pub fn new() -> Self {
        SomaticState {
            face_detected: false,
            hands_detected: false,
            shoulder_asymmetry: None,
            error: None,
        }
    }
}

pub struct SomaticBridge {
    state: Arc<Mutex<SomaticState>>,
    child: Option<tokio::process::Child>,
}

impl SomaticBridge {
    pub fn new() -> Self {
        SomaticBridge {
            state: Arc::new(Mutex::new(SomaticState::new())),
            child: None,
        }
    }

    pub fn get_state(&self) -> SomaticState {
        let lock = self.state.lock().unwrap();
        lock.clone()
    }

    pub fn start(&mut self) -> Result<(), String> {
        // Path to virtual environment python
        let python_exe = "D:\\Teledra\\.venv\\Scripts\\python.exe";
        let script_path = "D:\\Teledra\\somatic_cortex_stream.py";

        let mut cmd = Command::new(python_exe);
        cmd.arg(script_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn python script: {}", e))?;

        let stdout = child
            .stdout
            .take()
            .ok_or("Failed to capture python stdout")?;
        let state_clone = self.state.clone();

        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                // Parse stdout line by line
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
                    if let Some(status) = parsed.get("status") {
                        if status == "ready" {}
                    } else {
                        // Update state with posture/facial metrics
                        let mut lock = state_clone.lock().unwrap();
                        if let Some(face) = parsed.get("face_detected").and_then(|v| v.as_bool()) {
                            lock.face_detected = face;
                        }
                        if let Some(hands) = parsed.get("hands_detected").and_then(|v| v.as_bool())
                        {
                            lock.hands_detected = hands;
                        }
                        lock.shoulder_asymmetry = parsed
                            .get("shoulder_asymmetry")
                            .and_then(|v| v.as_f64())
                            .map(|f| f as f32);
                        lock.error = parsed
                            .get("error")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                }
            }
        });

        self.child = Some(child);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}
