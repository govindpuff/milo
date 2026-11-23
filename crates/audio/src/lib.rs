mod capture;
mod playback;
mod resample;

use std::sync::Arc;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, ErrorKind, FromSample, SampleFormat, SizedSample, StreamConfig, StreamInstant,
    SupportedBufferSize, SupportedStreamConfig,
};
use rtrb::{Consumer, Producer, RingBuffer};

pub use capture::Capture;
pub use playback::Playback;

use capture::Tap;
use playback::{PlayShared, Render};

const TARGET_FRAMES_48K: u64 = 256;
const PLAY_RING_SECS: usize = 30;
const TAP_RING_SECS: usize = 2;
const HDR_RING: usize = 4096;

#[derive(Clone, Debug)]
pub struct AudioConfig {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub tts_rate: u32,
}

pub struct AudioGuard {
    _input: cpal::Stream,
    _output: cpal::Stream,
    status: Arc<Status>,
}

impl Drop for AudioGuard {
    fn drop(&mut self) {
        self.status.alive.store(false, Release);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Hdr {
    pub pos: u64,
    pub t_ns: u64,
    pub cb_ns: u64,
    pub frames: u32,
}

pub(crate) struct Status {
    pub alive: AtomicBool,
    pub fatal: AtomicU8,
    pub errors: AtomicU64,
}

impl Status {
    pub fn fatal(&self) -> Option<&'static str> {
        match self.fatal.load(Acquire) {
            0 => None,
            1 => Some("device not available"),
            2 => Some("stream invalidated"),
            _ => Some("audio host unavailable"),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Clock {
    anchor_ns: u64,
    anchor: Instant,
    origin: Instant,
}

impl Clock {
    fn new(now: StreamInstant, origin: Instant) -> Clock {
        Clock { anchor_ns: now.as_nanos() as u64, anchor: Instant::now(), origin }
    }

    pub fn secs(&self, t_ns: u64) -> f64 {
        (t_ns as i128 - self.anchor_ns as i128) as f64 / 1e9 + (self.anchor - self.origin).as_secs_f64()
    }

    pub fn instant(&self, t_ns: u64) -> Instant {
        if t_ns >= self.anchor_ns {
            self.anchor + Duration::from_nanos(t_ns - self.anchor_ns)
        } else {
            self.anchor.checked_sub(Duration::from_nanos(self.anchor_ns - t_ns)).unwrap_or(self.anchor)
        }
    }
}

pub fn list_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let din = host.default_input_device();
    let dout = host.default_output_device();
    let mut out = Vec::new();
    for d in host.devices()? {
        let mut parts = Vec::new();
        if d.supports_input()
            && let Ok(c) = d.default_input_config()
        {
            parts.push(describe("in", &c, din.as_ref() == Some(&d)));
        }
        if d.supports_output()
            && let Ok(c) = d.default_output_config()
        {
            parts.push(describe("out", &c, dout.as_ref() == Some(&d)));
        }
        if !parts.is_empty() {
            out.push(format!("{}: {}", device_name(&d), parts.join("; ")));
        }
    }
    Ok(out)
}

pub fn open(cfg: &AudioConfig) -> Result<(Capture, Playback, AudioGuard)> {
    ensure!(cfg.tts_rate > 0, "tts_rate must be positive");
    let host = cpal::default_host();
    let in_dev = find_device(&host, cfg.input_device.as_deref(), true)?;
    let out_dev = find_device(&host, cfg.output_device.as_deref(), false)?;
    let status = Arc::new(Status { alive: AtomicBool::new(true), fatal: AtomicU8::new(0), errors: AtomicU64::new(0) });
    let origin = Instant::now();

    let out = build_output(&out_dev, &status)?;
    let inp = build_input(&in_dev, &status)?;
    let out_clock = Clock::new(out.stream.now(), origin);
    let in_clock = Clock::new(inp.stream.now(), origin);

    let playback = Playback::new(out.play, out.shared, status.clone(), cfg.tts_rate, out.rate, out_clock)?;
    let refs = Tap::new(out.ref_samples, out.ref_hdrs, out.rate, out_clock)?;
    let mic = Tap::new(inp.samples, inp.hdrs, inp.rate, in_clock)?;
    let capture = Capture::new(mic, refs, status.clone());

    out.stream.play().context("starting output stream")?;
    inp.stream.play().context("starting input stream")?;
    Ok((capture, playback, AudioGuard { _input: inp.stream, _output: out.stream, status }))
}

struct OutputParts {
    stream: cpal::Stream,
    rate: u32,
    play: Producer<f32>,
    shared: Arc<PlayShared>,
    ref_samples: Consumer<f32>,
    ref_hdrs: Consumer<Hdr>,
}

struct InputParts {
    stream: cpal::Stream,
    rate: u32,
    samples: Consumer<f32>,
    hdrs: Consumer<Hdr>,
}

fn build_output(dev: &Device, status: &Arc<Status>) -> Result<OutputParts> {
    let sup = dev.default_output_config().context("default output config")?;
    let (rate, channels, format) = (sup.sample_rate(), sup.channels(), sup.sample_format());
    let mut last_err = None;
    for buffer_size in buffer_candidates(sup.buffer_size(), rate) {
        let config = StreamConfig { channels, sample_rate: rate, buffer_size };
        let (play, play_cons) = RingBuffer::new(rate as usize * PLAY_RING_SECS);
        let (ref_prod, ref_samples) = RingBuffer::new(rate as usize * TAP_RING_SECS);
        let (ref_hdr, ref_hdrs) = RingBuffer::new(HDR_RING);
        let shared = Arc::new(PlayShared::default());
        let render = Render::new(play_cons, ref_prod, ref_hdr, shared.clone(), channels as usize, rate);
        let built = match format {
            SampleFormat::F32 => output_stream::<f32>(dev, config, render, status),
            SampleFormat::I16 => output_stream::<i16>(dev, config, render, status),
            SampleFormat::I32 => output_stream::<i32>(dev, config, render, status),
            f => bail!("unsupported output sample format {f}"),
        };
        match built {
            Ok(stream) => {
                log::info!(
                    "output {:?}: {} Hz, {} ch, {}, buffer {} frames",
                    device_name(dev),
                    rate,
                    channels,
                    format,
                    stream.buffer_size().map_or_else(|_| "?".into(), |b| b.to_string())
                );
                return Ok(OutputParts { stream, rate, play, shared, ref_samples, ref_hdrs });
            }
            Err(e) => {
                log::warn!("output stream with {buffer_size:?} failed: {e}");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.map_or_else(|| anyhow::anyhow!("no output config"), Into::into))
}

fn build_input(dev: &Device, status: &Arc<Status>) -> Result<InputParts> {
    let sup = dev.default_input_config().context("default input config")?;
    let (rate, channels, format) = (sup.sample_rate(), sup.channels(), sup.sample_format());
    let mut last_err = None;
    for buffer_size in buffer_candidates(sup.buffer_size(), rate) {
        let config = StreamConfig { channels, sample_rate: rate, buffer_size };
        let (prod, samples) = RingBuffer::new(rate as usize * TAP_RING_SECS);
        let (hdr, hdrs) = RingBuffer::new(HDR_RING);
        let built = match format {
            SampleFormat::F32 => input_stream::<f32>(dev, config, prod, hdr, status),
            SampleFormat::I16 => input_stream::<i16>(dev, config, prod, hdr, status),
            SampleFormat::I32 => input_stream::<i32>(dev, config, prod, hdr, status),
            f => bail!("unsupported input sample format {f}"),
        };
        match built {
            Ok(stream) => {
                log::info!(
                    "input {:?}: {} Hz, {} ch, {}, buffer {} frames",
                    device_name(dev),
                    rate,
                    channels,
                    format,
                    stream.buffer_size().map_or_else(|_| "?".into(), |b| b.to_string())
                );
                return Ok(InputParts { stream, rate, samples, hdrs });
            }
            Err(e) => {
                log::warn!("input stream with {buffer_size:?} failed: {e}");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.map_or_else(|| anyhow::anyhow!("no input config"), Into::into))
}

fn output_stream<T>(
    dev: &Device,
    config: StreamConfig,
    mut render: Render,
    status: &Arc<Status>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    dev.build_output_stream::<T, _, _>(
        config,
        move |out: &mut [T], info: &cpal::OutputCallbackInfo| render.render(out, info),
        on_error(status.clone()),
        None,
    )
}

fn input_stream<T>(
    dev: &Device,
    config: StreamConfig,
    mut prod: Producer<f32>,
    mut hdr: Producer<Hdr>,
    status: &Arc<Status>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let ch = config.channels.max(1) as usize;
    let mut pos = 0u64;
    dev.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], info: &cpal::InputCallbackInfo| {
            let frames = data.len() / ch;
            if frames == 0 {
                return;
            }
            let ts = info.timestamp();
            if prod.slots() >= frames
                && hdr.slots() >= 1
                && let Ok(mut chunk) = prod.write_chunk(frames)
            {
                let (a, b) = chunk.as_mut_slices();
                let split = a.len() * ch;
                downmix(&data[..split], a, ch);
                downmix(&data[split..frames * ch], b, ch);
                chunk.commit_all();
                let _ = hdr.push(Hdr {
                    pos,
                    t_ns: ts.capture.as_nanos() as u64,
                    cb_ns: ts.callback.as_nanos() as u64,
                    frames: frames as u32,
                });
            }
            pos += frames as u64;
        },
        on_error(status.clone()),
        None,
    )
}

fn downmix<T: Copy>(src: &[T], dst: &mut [f32], ch: usize)
where
    f32: FromSample<T>,
{
    if ch == 1 {
        for (d, &s) in dst.iter_mut().zip(src) {
            *d = f32::from_sample_(s);
        }
    } else {
        let k = 1.0 / ch as f32;
        for (d, frame) in dst.iter_mut().zip(src.chunks_exact(ch)) {
            *d = frame.iter().map(|&s| f32::from_sample_(s)).sum::<f32>() * k;
        }
    }
}

fn on_error(status: Arc<Status>) -> impl FnMut(cpal::Error) + Send + 'static {
    move |e: cpal::Error| match e.kind() {
        ErrorKind::DeviceNotAvailable => status.fatal.store(1, Release),
        ErrorKind::StreamInvalidated => status.fatal.store(2, Release),
        ErrorKind::HostUnavailable => status.fatal.store(3, Release),
        _ => {
            status.errors.fetch_add(1, Relaxed);
        }
    }
}

fn buffer_candidates(range: &SupportedBufferSize, rate: u32) -> [BufferSize; 2] {
    let want = ((rate as u64 * TARGET_FRAMES_48K + 24_000) / 48_000) as u32;
    let fixed = match *range {
        SupportedBufferSize::Range { min, max } if min <= max => want.clamp(min, max),
        _ => want,
    };
    [BufferSize::Fixed(fixed), BufferSize::Default]
}

fn find_device(host: &cpal::Host, name: Option<&str>, input: bool) -> Result<Device> {
    let kind = if input { "input" } else { "output" };
    let Some(name) = name else {
        let dev = if input { host.default_input_device() } else { host.default_output_device() };
        return dev.with_context(|| format!("no default {kind} device"));
    };
    let mut devices: Vec<Device> =
        if input { host.input_devices()?.collect() } else { host.output_devices()?.collect() };
    if let Some(i) = devices.iter().position(|d| device_name(d) == name) {
        return Ok(devices.swap_remove(i));
    }
    let needle = name.to_lowercase();
    let mut hits: Vec<Device> =
        devices.into_iter().filter(|d| device_name(d).to_lowercase().contains(&needle)).collect();
    match hits.len() {
        1 => Ok(hits.remove(0)),
        0 => bail!("no {kind} device matching {name:?}"),
        _ => bail!(
            "{kind} device {name:?} is ambiguous: {}",
            hits.iter().map(device_name).collect::<Vec<_>>().join(", ")
        ),
    }
}

fn device_name(d: &Device) -> String {
    d.description().map(|x| x.name().to_owned()).unwrap_or_else(|_| "unknown".into())
}

fn describe(dir: &str, c: &SupportedStreamConfig, default: bool) -> String {
    let buffer = match c.buffer_size() {
        SupportedBufferSize::Range { min, max } => format!(", buffer {min}-{max}"),
        SupportedBufferSize::Unknown => String::new(),
    };
    format!(
        "{dir}{} {} ch {} Hz {}{buffer}",
        if default { " (default)" } else { "" },
        c.channels(),
        c.sample_rate(),
        c.sample_format()
    )
}
