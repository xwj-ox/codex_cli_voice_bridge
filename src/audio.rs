use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    Device, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedStreamConfigRange,
};
use serde::Serialize;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[derive(Debug, Clone, Serialize)]
pub struct InputDeviceInfo {
    pub index: usize,
    pub name: String,
    pub default_sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
    pub is_default_input: bool,
}

#[derive(Debug, Clone)]
pub struct MicrophoneCaptureOptions {
    pub duration_seconds: f32,
    pub device_selector: Option<String>,
    pub chunk_ms: u32,
    pub chunk_bytes: usize,
    pub stop_signal: Option<Arc<AtomicBool>>,
    pub target_sample_rate: u32,
    pub target_channels: u16,
}

#[derive(Debug)]
pub enum CaptureEvent {
    Chunk { data: Vec<u8>, is_last: bool },
    Error(String),
}

pub struct MicrophoneCaptureStream {
    pub device_name: String,
    pub source_sample_rate: u32,
    pub source_channels: u16,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
    pub direct_target_format: bool,
    receiver: UnboundedReceiver<CaptureEvent>,
    stop_flag: Arc<AtomicBool>,
    _stream: Stream,
}

impl MicrophoneCaptureStream {
    pub async fn recv(&mut self) -> Option<CaptureEvent> {
        self.receiver.recv().await
    }
}

impl Drop for MicrophoneCaptureStream {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

struct PendingPcmState {
    pending_bytes: Vec<u8>,
    resampler: LinearMonoResampler,
    received_any: bool,
}

struct LinearMonoResampler {
    input_rate: u32,
    output_rate: u32,
    position: f64,
    last_sample: Option<f32>,
}

impl LinearMonoResampler {
    fn new(input_rate: u32, output_rate: u32) -> Self {
        Self {
            input_rate,
            output_rate,
            position: 0.0,
            last_sample: None,
        }
    }

    fn process(&mut self, input: &[f32]) -> Vec<i16> {
        if input.is_empty() {
            return Vec::new();
        }

        let mut samples = Vec::with_capacity(
            input.len() + if self.last_sample.is_some() { 1 } else { 0 },
        );
        if let Some(last) = self.last_sample {
            samples.push(last);
        }
        samples.extend_from_slice(input);

        if samples.len() < 2 {
            self.last_sample = samples.last().copied();
            return Vec::new();
        }

        let step = self.input_rate as f64 / self.output_rate as f64;
        let mut output = Vec::new();
        while self.position + 1.0 < samples.len() as f64 {
            let base_index = self.position.floor() as usize;
            let fraction = self.position - base_index as f64;
            let a = samples[base_index];
            let b = samples[base_index + 1];
            let mixed = a + (b - a) * fraction as f32;
            output.push(float_to_i16(mixed));
            self.position += step;
        }

        self.position -= (samples.len() - 1) as f64;
        self.last_sample = samples.last().copied();
        output
    }
}

struct ResolvedCaptureConfig {
    stream_config: StreamConfig,
    sample_format: SampleFormat,
    sample_format_name: String,
    source_sample_rate: u32,
    source_channels: u16,
    direct_target_format: bool,
}

pub fn list_input_devices() -> Result<Vec<InputDeviceInfo>> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok())
        .unwrap_or_default();

    let mut devices = Vec::new();
    for (index, device) in host
        .input_devices()
        .context("Failed to enumerate input devices")?
        .enumerate()
    {
        let name = device.name().unwrap_or_else(|_| "<unavailable>".to_owned());
        let config = device.default_input_config().ok();
        devices.push(InputDeviceInfo {
            index,
            is_default_input: name == default_name,
            default_sample_rate: config.as_ref().map(|cfg| cfg.sample_rate().0).unwrap_or(0),
            channels: config.as_ref().map(|cfg| cfg.channels()).unwrap_or(0),
            sample_format: config
                .as_ref()
                .map(|cfg| format!("{:?}", cfg.sample_format()))
                .unwrap_or_else(|| "unknown".to_owned()),
            name,
        });
    }

    Ok(devices)
}

pub fn start_microphone_capture(
    options: &MicrophoneCaptureOptions,
) -> Result<MicrophoneCaptureStream> {
    if options.duration_seconds <= 0.0 {
        bail!("--mic-duration must be greater than 0");
    }
    if options.target_channels != 1 {
        bail!("Rust microphone capture currently only supports mono upload");
    }
    if options.target_sample_rate == 0 {
        bail!("target_sample_rate must be greater than 0");
    }

    let (device, device_name) = resolve_input_device(options.device_selector.as_deref())?;
    let resolved = resolve_capture_config(&device, options)
        .context("Failed to resolve microphone capture format")?;
    let source_sample_rate = resolved.source_sample_rate;
    let source_channels = resolved.source_channels;
    let sample_format = resolved.sample_format_name.clone();
    let config = resolved.stream_config.clone();

    let chunk_size = infer_capture_chunk_bytes(
        options.chunk_bytes,
        options.chunk_ms,
        options.target_sample_rate,
        options.target_channels,
    );
    let (tx, rx) = unbounded_channel();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let shared_state = Arc::new(Mutex::new(PendingPcmState {
        pending_bytes: Vec::new(),
        resampler: LinearMonoResampler::new(source_sample_rate, options.target_sample_rate),
        received_any: false,
    }));

    let stream = match resolved.sample_format {
        SampleFormat::I8 => build_input_stream::<i8>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        SampleFormat::I16 => build_input_stream::<i16>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        SampleFormat::I24 => build_input_stream::<cpal::I24>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        SampleFormat::I32 => build_input_stream::<i32>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        SampleFormat::I64 => build_input_stream::<i64>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        SampleFormat::U8 => build_input_stream::<u8>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        SampleFormat::U16 => build_input_stream::<u16>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        SampleFormat::U32 => build_input_stream::<u32>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        SampleFormat::U64 => build_input_stream::<u64>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        SampleFormat::F32 => build_input_stream::<f32>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        SampleFormat::F64 => build_input_stream::<f64>(
            &device,
            &config,
            chunk_size,
            tx.clone(),
            stop_flag.clone(),
            shared_state.clone(),
        )?,
        other => bail!("Unsupported microphone sample format: {other:?}"),
    };

    stream.play().context("Failed to start microphone stream")?;

    let finalize_tx = tx.clone();
    let finalize_state = shared_state.clone();
    let finalize_stop = stop_flag.clone();
    let finalize_channels = options.target_channels;
    let finalize_duration = options.duration_seconds;
    let external_stop = options.stop_signal.clone();
    thread::spawn(move || {
        let started = Instant::now();
        let stop_reason = loop {
            if finalize_stop.load(Ordering::Relaxed) {
                return;
            }
            if external_stop
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
            {
                break "ptt-release";
            }
            if started.elapsed() >= Duration::from_secs_f32(finalize_duration) {
                break "max-duration";
            }
            thread::sleep(Duration::from_millis(10));
        };
        println!(
            "[AUDIO] finalizing microphone capture: reason={}, elapsed_ms={}",
            stop_reason,
            started.elapsed().as_millis()
        );
        finalize_stop.store(true, Ordering::Relaxed);
        let mut state = match finalize_state.lock() {
            Ok(state) => state,
            Err(_) => {
                let _ = finalize_tx.send(CaptureEvent::Error(
                    "Microphone capture state lock poisoned".to_owned(),
                ));
                return;
            }
        };

        if !state.pending_bytes.is_empty() {
            let data = std::mem::take(&mut state.pending_bytes);
            println!("[AUDIO] sending final microphone chunk: bytes={}", data.len());
            let _ = finalize_tx.send(CaptureEvent::Chunk {
                data,
                is_last: true,
            });
            return;
        }

        if state.received_any {
            println!(
                "[AUDIO] no pending bytes at finalize; sending synthetic final chunk to close capture."
            );
            let _ = finalize_tx.send(CaptureEvent::Chunk {
                data: vec![0u8; finalize_channels as usize * 2],
                is_last: true,
            });
            return;
        }

        println!("[AUDIO] no microphone audio captured before finalize.");
        let _ = finalize_tx.send(CaptureEvent::Error(
            "No microphone audio captured. Check device permissions and settings.".to_owned(),
        ));
    });

    Ok(MicrophoneCaptureStream {
        device_name,
        source_sample_rate,
        source_channels,
        sample_rate: options.target_sample_rate,
        channels: options.target_channels,
        sample_format,
        direct_target_format: resolved.direct_target_format,
        receiver: rx,
        stop_flag,
        _stream: stream,
    })
}

fn resolve_capture_config(
    device: &Device,
    options: &MicrophoneCaptureOptions,
) -> Result<ResolvedCaptureConfig> {
    if let Ok(mut supported_configs) = device.supported_input_configs() {
        let mut candidates = supported_configs
            .by_ref()
            .filter(|config| config.channels() == options.target_channels)
            .filter(|config| {
                let min_rate = config.min_sample_rate().0;
                let max_rate = config.max_sample_rate().0;
                options.target_sample_rate >= min_rate && options.target_sample_rate <= max_rate
            })
            .collect::<Vec<_>>();

        candidates.sort_by_key(|config| format_preference_score(config));
        if let Some(best) = candidates.into_iter().next() {
            let config = best.with_sample_rate(cpal::SampleRate(options.target_sample_rate));
            return Ok(ResolvedCaptureConfig {
                stream_config: StreamConfig {
                    channels: config.channels(),
                    sample_rate: config.sample_rate(),
                    buffer_size: cpal::BufferSize::Default,
                },
                sample_format: config.sample_format(),
                sample_format_name: format!("{:?}", config.sample_format()),
                source_sample_rate: config.sample_rate().0,
                source_channels: config.channels(),
                direct_target_format: true,
            });
        }
    }

    let supported = device
        .default_input_config()
        .context("Failed to read default input config")?;
    Ok(ResolvedCaptureConfig {
        stream_config: StreamConfig {
            channels: supported.channels(),
            sample_rate: supported.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        },
        sample_format: supported.sample_format(),
        sample_format_name: format!("{:?}", supported.sample_format()),
        source_sample_rate: supported.sample_rate().0,
        source_channels: supported.channels(),
        direct_target_format: supported.sample_rate().0 == options.target_sample_rate
            && supported.channels() == options.target_channels,
    })
}

fn format_preference_score(config: &SupportedStreamConfigRange) -> u8 {
    match config.sample_format() {
        SampleFormat::I16 => 0,
        SampleFormat::F32 => 1,
        SampleFormat::U16 => 2,
        _ => 10,
    }
}

fn resolve_input_device(selector: Option<&str>) -> Result<(Device, String)> {
    let host = cpal::default_host();
    let selector = selector.unwrap_or("").trim();

    if selector.is_empty() {
        let device = host
            .default_input_device()
            .context("No default input device available")?;
        let name = device.name().unwrap_or_else(|_| "<unavailable>".to_owned());
        return Ok((device, name));
    }

    let devices = host
        .input_devices()
        .context("Failed to enumerate input devices")?
        .collect::<Vec<_>>();

    if let Ok(index) = selector.parse::<usize>() {
        let device = devices
            .into_iter()
            .nth(index)
            .with_context(|| format!("Input device index out of range: {index}"))?;
        let name = device.name().unwrap_or_else(|_| "<unavailable>".to_owned());
        return Ok((device, name));
    }

    let selector_lower = selector.to_ascii_lowercase();
    for device in devices {
        let name = device.name().unwrap_or_else(|_| "<unavailable>".to_owned());
        if name.to_ascii_lowercase().contains(&selector_lower) {
            return Ok((device, name));
        }
    }

    Err(anyhow!("No input device matched selector: {selector}"))
}

fn infer_capture_chunk_bytes(
    chunk_bytes: usize,
    chunk_ms: u32,
    sample_rate: u32,
    channels: u16,
) -> usize {
    if chunk_bytes > 0 {
        return chunk_bytes;
    }
    let bytes_per_second = sample_rate as usize * channels as usize * 2;
    let inferred = (bytes_per_second * chunk_ms as usize) / 1000;
    inferred.max(channels as usize * 2)
}

fn build_input_stream<T>(
    device: &Device,
    config: &StreamConfig,
    chunk_size: usize,
    tx: UnboundedSender<CaptureEvent>,
    stop_flag: Arc<AtomicBool>,
    shared_state: Arc<Mutex<PendingPcmState>>,
) -> Result<Stream>
where
    T: Sample + SizedSample + Send + Copy + 'static,
    f32: FromSample<T>,
{
    let err_tx = tx.clone();
    let input_channels = config.channels as usize;
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _| {
            if stop_flag.load(Ordering::Relaxed) {
                return;
            }
            let mut state = match shared_state.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            if stop_flag.load(Ordering::Relaxed) {
                return;
            }

            let mono = downmix_to_mono(data, input_channels);
            let resampled = state.resampler.process(&mono);
            if !resampled.is_empty() {
                state.received_any = true;
                for sample in resampled {
                    state.pending_bytes.extend_from_slice(&sample.to_le_bytes());
                }
            }

            while state.pending_bytes.len() >= chunk_size {
                let chunk = state.pending_bytes.drain(..chunk_size).collect::<Vec<_>>();
                let _ = tx.send(CaptureEvent::Chunk {
                    data: chunk,
                    is_last: false,
                });
            }
        },
        move |err| {
            let _ = err_tx.send(CaptureEvent::Error(format!("Microphone stream error: {err}")));
        },
        None,
    )?;
    Ok(stream)
}

fn downmix_to_mono<T>(data: &[T], channels: usize) -> Vec<f32>
where
    T: Sample + Copy,
    f32: FromSample<T>,
{
    if channels <= 1 {
        return data.iter().copied().map(f32::from_sample).collect();
    }

    data.chunks(channels)
        .map(|frame| {
            let sum = frame
                .iter()
                .copied()
                .map(f32::from_sample)
                .fold(0.0f32, |acc, sample| acc + sample);
            sum / frame.len() as f32
        })
        .collect()
}

fn float_to_i16(sample: f32) -> i16 {
    let clamped = sample.clamp(-1.0, 1.0);
    if clamped >= 0.0 {
        (clamped * i16::MAX as f32).round() as i16
    } else {
        (clamped * -(i16::MIN as f32)).round() as i16
    }
}
