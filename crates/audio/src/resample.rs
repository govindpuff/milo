use anyhow::{Result, ensure};

const ZEROS: f64 = 16.0;
const ROLLOFF: f64 = 0.9;
const BETA: f64 = 8.0;
const MAX_PHASES: usize = 8192;

pub(crate) struct Resampler {
    l: usize,
    m: usize,
    taps: usize,
    half: usize,
    table: Vec<f32>,
    buf: Vec<f32>,
    base: usize,
    acc: usize,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Result<Resampler> {
        ensure!(from > 0 && to > 0, "invalid resampling rates {from} -> {to}");
        let g = gcd(from, to);
        let l = (to / g) as usize;
        let m = (from / g) as usize;
        ensure!(l <= MAX_PHASES, "unsupported resampling ratio {from} -> {to}");
        if l == m {
            return Ok(Resampler { l, m, taps: 0, half: 0, table: Vec::new(), buf: Vec::new(), base: 0, acc: 0 });
        }
        let fc = (l as f64 / m as f64).min(1.0) * ROLLOFF;
        let half = (ZEROS / fc).ceil() as usize;
        let taps = 2 * half;
        let norm = bessel_i0(BETA);
        let mut table = vec![0f32; l * taps];
        let mut row = vec![0f64; taps];
        for p in 0..l {
            let frac = p as f64 / l as f64;
            for (i, v) in row.iter_mut().enumerate() {
                let tau = frac - (i as f64 - half as f64 + 1.0);
                let r = tau / half as f64;
                let w = if r.abs() < 1.0 { bessel_i0(BETA * (1.0 - r * r).sqrt()) / norm } else { 0.0 };
                *v = fc * sinc(fc * tau) * w;
            }
            let sum: f64 = row.iter().sum();
            for (dst, v) in table[p * taps..(p + 1) * taps].iter_mut().zip(&row) {
                *dst = (v / sum) as f32;
            }
        }
        let mut rs = Resampler { l, m, taps, half, table, buf: Vec::with_capacity(8192), base: 0, acc: 0 };
        rs.reset();
        Ok(rs)
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.buf.resize(self.half.saturating_sub(1), 0.0);
        self.base = 0;
        self.acc = 0;
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.taps == 0 {
            out.extend_from_slice(input);
            return;
        }
        self.buf.extend_from_slice(input);
        while self.base + self.taps <= self.buf.len() {
            let h = &self.table[self.acc * self.taps..][..self.taps];
            let x = &self.buf[self.base..][..self.taps];
            out.push(dot(x, h));
            self.acc += self.m;
            self.base += self.acc / self.l;
            self.acc %= self.l;
        }
        let keep = self.base.min(self.buf.len());
        self.buf.drain(..keep);
        self.base -= keep;
    }
}

fn dot(x: &[f32], h: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let (xc, xr) = x.as_chunks::<8>();
    let (hc, hr) = h.as_chunks::<8>();
    for (a, b) in xc.iter().zip(hc) {
        for k in 0..8 {
            acc[k] += a[k] * b[k];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (a, b) in xr.iter().zip(hr) {
        s += a * b;
    }
    s
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) }
}

fn bessel_i0(x: f64) -> f64 {
    let y = x * x / 4.0;
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..100 {
        term *= y / (k * k) as f64;
        sum += term;
        if term < sum * 1e-14 {
            break;
        }
    }
    sum
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
