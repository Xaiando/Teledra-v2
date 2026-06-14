// Audio-input ("ears") subsystem: FFT + RØDE UNIFY loopback listener.
// Constructed in main.rs but not yet wired into the court loop; kept as a
// planned feature, so dead-code warnings are silenced module-wide. Remove
// this allow when start_listening()/get_state() are actually used.
#![allow(dead_code)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    pub fn new(re: f32, im: f32) -> Self {
        Complex { re, im }
    }

    pub fn add(self, other: Self) -> Self {
        Complex::new(self.re + other.re, self.im + other.im)
    }

    pub fn sub(self, other: Self) -> Self {
        Complex::new(self.re - other.re, self.im - other.im)
    }

    pub fn mul(self, other: Self) -> Self {
        Complex::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }

    pub fn norm(self) -> f32 {
        (self.re * self.re + self.im * self.im).sqrt()
    }
}

// Simple Radix-2 Cooley-Tukey FFT (in-place)
pub fn fft(a: &mut [Complex]) {
    let n = a.len();
    if n <= 1 {
        return;
    }

    // Bit reversal permutation
    let mut j = 0;
    for i in 0..n {
        if i < j {
            a.swap(i, j);
        }
        let mut m = n >> 1;
        while m >= 1 && j >= m {
            j -= m;
            m >>= 1;
        }
        j += m;
    }

    // Cooley-Tukey Decimation-in-Time
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * std::f32::consts::PI / (len as f32);
        let wlen = Complex::new(angle.cos(), angle.sin());
        for i in (0..n).step_by(len) {
            let mut w = Complex::new(1.0, 0.0);
            for k in 0..(len / 2) {
                let u = a[i + k];
                let t = a[i + k + len / 2].mul(w);
                a[i + k] = u.add(t);
                a[i + k + len / 2] = u.sub(t);
                w = w.mul(wlen);
            }
        }
        len <<= 1;
    }
}

#[derive(Clone)]
pub struct EarsState {
    pub current_rms: f32,
    pub spectral_flatness: f32,
}

pub struct AudioCortex {
    state: Arc<Mutex<EarsState>>,
    stream: Option<cpal::Stream>,
}

impl AudioCortex {
    pub fn new() -> Self {
        AudioCortex {
            state: Arc::new(Mutex::new(EarsState {
                current_rms: 0.0,
                spectral_flatness: 0.0,
            })),
            stream: None,
        }
    }

    pub fn get_state(&self) -> EarsState {
        let lock = self.state.lock().unwrap();
        lock.clone()
    }

    pub fn start_listening(&mut self) -> Result<(), String> {
        let host = cpal::default_host();

        // Find WASAPI device for Røde Unify loopback or stream input
        let devices = host.devices().map_err(|e| e.to_string())?;
        let mut target_device = None;

        for dev in devices {
            if let Ok(name) = dev.name() {
                if name.contains("Stream Input") && name.contains("RØDE UNIFY") {
                    target_device = Some(dev);
                    break;
                }
            }
        }

        let device = match target_device {
            Some(d) => d,
            None => host
                .default_input_device()
                .ok_or("No default input device found")?,
        };

        let config: cpal::StreamConfig = device
            .default_input_config()
            .map_err(|e| e.to_string())?
            .into();

        let state_clone = self.state.clone();

        let error_callback = |err| eprintln!("an error occurred on stream: {}", err);

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // Calculate RMS
                    let sum_sq: f32 = data.iter().map(|&x| x * x).sum();
                    let rms = (sum_sq / data.len() as f32).sqrt();

                    // Compute spectral flatness if volume is sufficient
                    let mut flatness = 0.0;
                    if rms > 0.001 {
                        // Use a block size of 2048 (or closest power of 2)
                        let fft_len = 2048;
                        if data.len() >= fft_len {
                            let mut fft_buf: Vec<Complex> = data[0..fft_len]
                                .iter()
                                .map(|&x| Complex::new(x, 0.0))
                                .collect();
                            fft(&mut fft_buf);

                            // Take magnitude of first half (RFFT counterpart)
                            let half_len = fft_len / 2;
                            let mut sum_mag = 0.0;
                            let mut log_sum_mag = 0.0;

                            for k in 0..half_len {
                                let mag = fft_buf[k].norm();
                                let val = mag + 1e-10;
                                sum_mag += val;
                                log_sum_mag += val.ln();
                            }

                            let amean = sum_mag / half_len as f32;
                            let gmean = (log_sum_mag / half_len as f32).exp();

                            if amean > 1e-10 {
                                flatness = gmean / amean;
                            }
                        }
                    }

                    // Update state
                    let mut lock = state_clone.lock().unwrap();
                    lock.current_rms = rms;
                    lock.spectral_flatness = flatness;
                },
                error_callback,
                None,
            )
            .map_err(|e| e.to_string())?;

        stream.play().map_err(|e| e.to_string())?;
        self.stream = Some(stream);

        Ok(())
    }

    pub fn stop_listening(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.pause();
        }
        let mut lock = self.state.lock().unwrap();
        lock.current_rms = 0.0;
        lock.spectral_flatness = 0.0;
    }
}
