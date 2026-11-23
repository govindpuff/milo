use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

const SAMPLES: usize = 8 * 16000;
pub const MELS: usize = 80;
pub const FRAMES: usize = 800;
const N_FFT: usize = 400;
const HOP: usize = 160;
const BINS: usize = N_FFT / 2 + 1;

pub struct Mel {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    filters: Vec<f32>,
}

fn hz_to_mel(hz: f64) -> f64 {
    if hz < 1000.0 { 3.0 * hz / 200.0 } else { 15.0 + (hz / 1000.0).ln() * 27.0 / 6.4f64.ln() }
}

fn mel_to_hz(mel: f64) -> f64 {
    if mel < 15.0 { 200.0 * mel / 3.0 } else { 1000.0 * (6.4f64.ln() / 27.0 * (mel - 15.0)).exp() }
}

impl Mel {
    pub fn new() -> Mel {
        let window = (0..N_FFT)
            .map(|n| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / N_FFT as f64).cos()) as f32)
            .collect();
        let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(8000.0));
        let edges: Vec<f64> = (0..MELS + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (MELS + 1) as f64)).collect();
        let mut filters = vec![0.0f32; MELS * BINS];
        for m in 0..MELS {
            let (a, b, c) = (edges[m], edges[m + 1], edges[m + 2]);
            let norm = 2.0 / (c - a);
            for k in 0..BINS {
                let f = 8000.0 * k as f64 / (BINS - 1) as f64;
                let w = ((f - a) / (b - a)).min((c - f) / (c - b)).max(0.0);
                filters[m * BINS + k] = (w * norm) as f32;
            }
        }
        Mel { fft: FftPlanner::new().plan_fft_forward(N_FFT), window, filters }
    }

    pub fn features(&self, audio: &[f32]) -> Vec<f32> {
        let audio = &audio[audio.len().saturating_sub(SAMPLES)..];
        let mut x = vec![0.0f32; SAMPLES];
        x[SAMPLES - audio.len()..].copy_from_slice(audio);
        let mean = x.iter().map(|&v| v as f64).sum::<f64>() / SAMPLES as f64;
        let var = x.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / SAMPLES as f64;
        let scale = 1.0 / (var + 1e-7).sqrt();
        for v in &mut x {
            *v = ((*v as f64 - mean) * scale) as f32;
        }

        let half = N_FFT / 2;
        let mut padded = Vec::with_capacity(SAMPLES + N_FFT);
        padded.extend((1..=half).rev().map(|i| x[i]));
        padded.extend_from_slice(&x);
        padded.extend((1..=half).map(|i| x[SAMPLES - 1 - i]));

        let mut out = vec![0.0f32; MELS * FRAMES];
        let mut buf = vec![Complex32::default(); N_FFT];
        let mut power = [0.0f32; BINS];
        let mut peak = f32::MIN;
        for t in 0..FRAMES {
            let frame = &padded[t * HOP..t * HOP + N_FFT];
            for (b, (&s, &w)) in buf.iter_mut().zip(frame.iter().zip(&self.window)) {
                *b = Complex32::new(s * w, 0.0);
            }
            self.fft.process(&mut buf);
            for (p, c) in power.iter_mut().zip(&buf) {
                *p = c.norm_sqr();
            }
            for m in 0..MELS {
                let row = &self.filters[m * BINS..(m + 1) * BINS];
                let e: f32 = row.iter().zip(&power).map(|(a, b)| a * b).sum();
                let v = e.max(1e-10).log10();
                peak = peak.max(v);
                out[m * FRAMES + t] = v;
            }
        }
        for v in &mut out {
            *v = (v.max(peak - 8.0) + 4.0) / 4.0;
        }
        out
    }
}
