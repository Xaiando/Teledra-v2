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
    environment: crate::EnvironmentReport,
    paths: crate::AppPaths,
}

impl SomaticBridge {
    pub fn new(environment: crate::EnvironmentReport, paths: &crate::AppPaths) -> Self {
        SomaticBridge {
            state: Arc::new(Mutex::new(SomaticState::new())),
            child: None,
            environment,
            paths: paths.clone(),
        }
    }

    pub fn get_state(&self) -> SomaticState {
        let lock = self.state.lock().unwrap();
        lock.clone()
    }

    pub fn start(&mut self) -> Result<crate::sidecar::SidecarOutcome<()>, String> {
        let ctx = crate::sidecar::RuntimeContext {
            paths: &self.paths,
            environment: &self.environment,
        };
        
        let mut cmd = match crate::sidecar::tokio_python_sidecar_command(&ctx, crate::sidecar::SidecarKind::Somatic) {
            Ok(cmd) => cmd,
            Err(crate::sidecar::SidecarError::Disabled { reason, .. }) => return Ok(crate::sidecar::SidecarOutcome::Disabled { reason }),
            Err(e) => return Err(e.to_string()),
        };

        cmd.stdout(Stdio::piped())
           .stderr(Stdio::piped());
           
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn somatic sidecar: {}", e))?;

        let stdout = child
            .stdout
            .take()
            .ok_or("Failed to capture python stdout")?;
        
        let stderr = child
            .stderr
            .take()
            .ok_or("Failed to capture python stderr")?;

        let state_clone = self.state.clone();
        let state_stderr = self.state.clone();

        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
                            if let Some(status) = parsed.get("status") {
                                if status == "ready" {}
                            } else {
                                let mut lock = state_clone.lock().unwrap();
                                if let Some(face) = parsed.get("face_detected").and_then(|v| v.as_bool()) {
                                    lock.face_detected = face;
                                }
                                if let Some(hands) = parsed.get("hands_detected").and_then(|v| v.as_bool()) {
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
                    Ok(None) | Err(_) => break,
                }
            }
        });

        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            let mut tail = String::new();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        // Keep a bounded 1 KB tail of stderr for diagnostics
                        tail.push_str(&line);
                        tail.push('\n');
                        if tail.len() > 1000 {
                            let trim_at = tail.len() - 1000;
                            tail = tail[trim_at..].to_string();
                        }
                        let mut lock = state_stderr.lock().unwrap();
                        if lock.error.is_none() {
                            lock.error = Some(tail.clone());
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        });

        self.child = Some(child);
        Ok(crate::sidecar::SidecarOutcome::Started(()))
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}
