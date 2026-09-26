use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig};
use rubato::{FftFixedIn, Resampler};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub mod wav;

const TARGET_SAMPLE_RATE: u32 = 16000;
const TARGET_CHANNELS: u16 = 1;
const CHUNK_DURATION_MS: u32 = 100;

pub struct AudioCapture {
    stream: Option<cpal::Stream>,
    is_recording: Arc<AtomicBool>,
}

impl AudioCapture {
    pub fn new() -> Self {
        Self {
            stream: None,
            is_recording: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(&mut self, audio_tx: mpsc::Sender<Vec<u8>>) -> Result<()> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No input device available")?;

        info!("Using input device: {}", device.name()?);

        let supported_configs = device.supported_input_configs()?;
        debug!("Supported configs:");
        for config in supported_configs {
            debug!("  {:?}", config);
        }

        // Try to find a config that matches our target
        let config = find_best_config(&device)?;
        info!("Using config: {:?}", config);

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let sample_format = config.sample_format();

        self.is_recording.store(true, Ordering::SeqCst);
        let is_recording = self.is_recording.clone();

        // Calculate samples per chunk based on the actual sample rate
        let samples_per_chunk = (sample_rate * CHUNK_DURATION_MS / 1000) as usize;

        let stream = match sample_format {
            SampleFormat::I8 => build_stream_i8(
                &device,
                &config.into(),
                audio_tx,
                is_recording,
                sample_rate,
                channels,
                samples_per_chunk,
            )?,
            SampleFormat::I16 => build_stream_i16(
                &device,
                &config.into(),
                audio_tx,
                is_recording,
                sample_rate,
                channels,
                samples_per_chunk,
            )?,
            SampleFormat::I32 => build_stream_i32(
                &device,
                &config.into(),
                audio_tx,
                is_recording,
                sample_rate,
                channels,
                samples_per_chunk,
            )?,
            SampleFormat::I64 => build_stream_i64(
                &device,
                &config.into(),
                audio_tx,
                is_recording,
                sample_rate,
                channels,
                samples_per_chunk,
            )?,
            SampleFormat::U8 => build_stream_u8(
                &device,
                &config.into(),
                audio_tx,
                is_recording,
                sample_rate,
                channels,
                samples_per_chunk,
            )?,
            SampleFormat::U16 => build_stream_u16(
                &device,
                &config.into(),
                audio_tx,
                is_recording,
                sample_rate,
                channels,
                samples_per_chunk,
            )?,
            SampleFormat::U32 => build_stream_u32(
                &device,
                &config.into(),
                audio_tx,
                is_recording,
                sample_rate,
                channels,
                samples_per_chunk,
            )?,
            SampleFormat::U64 => build_stream_u64(
                &device,
                &config.into(),
                audio_tx,
                is_recording,
                sample_rate,
                channels,
                samples_per_chunk,
            )?,
            SampleFormat::F32 => build_stream_f32(
                &device,
                &config.into(),
                audio_tx,
                is_recording,
                sample_rate,
                channels,
                samples_per_chunk,
            )?,
            SampleFormat::F64 => build_stream_f64(
                &device,
                &config.into(),
                audio_tx,
                is_recording,
                sample_rate,
                channels,
                samples_per_chunk,
            )?,
            _ => {
                return Err(anyhow::anyhow!(
                    "Unsupported sample format: {:?}",
                    sample_format
                ))
            }
        };

        stream.play()?;
        self.stream = Some(stream);

        info!("Audio capture started");
        Ok(())
    }

    pub fn stop(&mut self) {
        self.is_recording.store(false, Ordering::SeqCst);
        self.stream = None;
        info!("Audio capture stopped");
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }
}

fn sample_format_rank(format: SampleFormat) -> u8 {
    match format {
        SampleFormat::F32 => 9,
        SampleFormat::I32 => 8,
        SampleFormat::F64 => 7,
        SampleFormat::I64 => 6,
        SampleFormat::U32 => 5,
        SampleFormat::U64 => 4,
        SampleFormat::I16 => 3,
        SampleFormat::U16 => 2,
        SampleFormat::I8 | SampleFormat::U8 => 0,
        _ => 1,
    }
}

fn find_best_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig> {
    let mut supported_configs: Vec<_> = device.supported_input_configs()?.collect();
    // Within each tier below, prefer the highest-resolution sample format. Devices
    // (e.g. the PipeWire ALSA "default") may list U8 first, and 8-bit capture erases
    // quiet sounds such as word onsets.
    supported_configs.sort_by_key(|c| std::cmp::Reverse(sample_format_rank(c.sample_format())));

    // First, try to find exact match
    for config in &supported_configs {
        if config.channels() == TARGET_CHANNELS
            && config.min_sample_rate().0 <= TARGET_SAMPLE_RATE
            && config.max_sample_rate().0 >= TARGET_SAMPLE_RATE
        {
            return Ok(config
                .clone()
                .with_sample_rate(SampleRate(TARGET_SAMPLE_RATE)));
        }
    }

    // Try mono with any sample rate
    for config in &supported_configs {
        if config.channels() == TARGET_CHANNELS {
            let sample_rate = if config.min_sample_rate().0 <= TARGET_SAMPLE_RATE
                && config.max_sample_rate().0 >= TARGET_SAMPLE_RATE
            {
                TARGET_SAMPLE_RATE
            } else {
                config.max_sample_rate().0.min(48000)
            };
            return Ok(config.clone().with_sample_rate(SampleRate(sample_rate)));
        }
    }

    // Fall back to stereo
    for config in &supported_configs {
        if config.channels() == 2 {
            let sample_rate = if config.min_sample_rate().0 <= TARGET_SAMPLE_RATE
                && config.max_sample_rate().0 >= TARGET_SAMPLE_RATE
            {
                TARGET_SAMPLE_RATE
            } else {
                config.max_sample_rate().0.min(48000)
            };
            return Ok(config.clone().with_sample_rate(SampleRate(sample_rate)));
        }
    }

    // Use default
    device
        .default_input_config()
        .context("No suitable input config found")
}

// Helper function to process audio data regardless of type
fn process_audio_samples(
    samples_f32: Vec<f32>,
    source_channels: u16,
    buffer: &mut Vec<f32>,
    samples_per_chunk: usize,
    resampler: &mut Option<AudioResampler>,
    audio_tx: &mpsc::Sender<Vec<u8>>,
) {
    // Convert to mono if stereo
    let mono_samples: Vec<f32> = if source_channels == 2 {
        samples_f32
            .chunks(2)
            .map(|chunk| (chunk[0] + chunk.get(1).copied().unwrap_or(0.0)) / 2.0)
            .collect()
    } else if source_channels > 2 {
        samples_f32
            .chunks(source_channels as usize)
            .map(|chunk| chunk.iter().sum::<f32>() / source_channels as f32)
            .collect()
    } else {
        samples_f32
    };

    buffer.extend(mono_samples);

    // Process complete chunks
    while buffer.len() >= samples_per_chunk {
        let chunk: Vec<f32> = buffer.drain(..samples_per_chunk).collect();

        // Resample if necessary
        let resampled = if let Some(resampler) = resampler.as_mut() {
            resampler.resample(&chunk)
        } else {
            chunk
        };

        // Convert to i16 PCM bytes
        let pcm_bytes: Vec<u8> = resampled
            .iter()
            .flat_map(|&sample| {
                let clamped = sample.clamp(-1.0, 1.0);
                let i16_sample = (clamped * 32767.0) as i16;
                i16_sample.to_le_bytes()
            })
            .collect();

        // Send to channel (non-blocking)
        if let Err(e) = audio_tx.try_send(pcm_bytes) {
            warn!("Failed to send audio chunk: {}", e);
        }
    }
}

fn build_stream_i8(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_tx: mpsc::Sender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    samples_per_chunk: usize,
) -> Result<cpal::Stream> {
    let mut buffer: Vec<f32> = Vec::with_capacity(samples_per_chunk * source_channels as usize * 2);
    let mut resampler = AudioResampler::new_if_needed(source_sample_rate, samples_per_chunk)?;

    let err_fn = |err| error!("Audio stream error: {}", err);

    let stream = device.build_input_stream(
        config,
        move |data: &[i8], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            // Convert i8 to f32 (range -128 to 127)
            let samples: Vec<f32> = data.iter().map(|&s| s as f32 / 128.0).collect();

            process_audio_samples(
                samples,
                source_channels,
                &mut buffer,
                samples_per_chunk,
                &mut resampler,
                &audio_tx,
            );
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn build_stream_i16(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_tx: mpsc::Sender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    samples_per_chunk: usize,
) -> Result<cpal::Stream> {
    let mut buffer: Vec<f32> = Vec::with_capacity(samples_per_chunk * source_channels as usize * 2);
    let mut resampler = AudioResampler::new_if_needed(source_sample_rate, samples_per_chunk)?;

    let err_fn = |err| error!("Audio stream error: {}", err);

    let stream = device.build_input_stream(
        config,
        move |data: &[i16], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            // Convert i16 to f32 (range -32768 to 32767)
            let samples: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();

            process_audio_samples(
                samples,
                source_channels,
                &mut buffer,
                samples_per_chunk,
                &mut resampler,
                &audio_tx,
            );
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn build_stream_i32(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_tx: mpsc::Sender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    samples_per_chunk: usize,
) -> Result<cpal::Stream> {
    let mut buffer: Vec<f32> = Vec::with_capacity(samples_per_chunk * source_channels as usize * 2);
    let mut resampler = AudioResampler::new_if_needed(source_sample_rate, samples_per_chunk)?;

    let err_fn = |err| error!("Audio stream error: {}", err);

    let stream = device.build_input_stream(
        config,
        move |data: &[i32], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            // Convert i32 to f32
            let samples: Vec<f32> = data.iter().map(|&s| s as f32 / (i32::MAX as f32)).collect();

            process_audio_samples(
                samples,
                source_channels,
                &mut buffer,
                samples_per_chunk,
                &mut resampler,
                &audio_tx,
            );
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn build_stream_i64(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_tx: mpsc::Sender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    samples_per_chunk: usize,
) -> Result<cpal::Stream> {
    let mut buffer: Vec<f32> = Vec::with_capacity(samples_per_chunk * source_channels as usize * 2);
    let mut resampler = AudioResampler::new_if_needed(source_sample_rate, samples_per_chunk)?;

    let err_fn = |err| error!("Audio stream error: {}", err);

    let stream = device.build_input_stream(
        config,
        move |data: &[i64], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            // Convert i64 to f32
            let samples: Vec<f32> = data
                .iter()
                .map(|&s| (s as f64 / i64::MAX as f64) as f32)
                .collect();

            process_audio_samples(
                samples,
                source_channels,
                &mut buffer,
                samples_per_chunk,
                &mut resampler,
                &audio_tx,
            );
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn build_stream_u8(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_tx: mpsc::Sender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    samples_per_chunk: usize,
) -> Result<cpal::Stream> {
    let mut buffer: Vec<f32> = Vec::with_capacity(samples_per_chunk * source_channels as usize * 2);
    let mut resampler = AudioResampler::new_if_needed(source_sample_rate, samples_per_chunk)?;

    let err_fn = |err| error!("Audio stream error: {}", err);

    let stream = device.build_input_stream(
        config,
        move |data: &[u8], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            // Convert u8 to f32 (range 0 to 255, center at 128)
            let samples: Vec<f32> = data.iter().map(|&s| (s as f32 - 128.0) / 128.0).collect();

            process_audio_samples(
                samples,
                source_channels,
                &mut buffer,
                samples_per_chunk,
                &mut resampler,
                &audio_tx,
            );
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn build_stream_u16(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_tx: mpsc::Sender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    samples_per_chunk: usize,
) -> Result<cpal::Stream> {
    let mut buffer: Vec<f32> = Vec::with_capacity(samples_per_chunk * source_channels as usize * 2);
    let mut resampler = AudioResampler::new_if_needed(source_sample_rate, samples_per_chunk)?;

    let err_fn = |err| error!("Audio stream error: {}", err);

    let stream = device.build_input_stream(
        config,
        move |data: &[u16], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            // Convert u16 to f32 (range 0 to 65535, center at 32768)
            let samples: Vec<f32> = data
                .iter()
                .map(|&s| (s as f32 - 32768.0) / 32768.0)
                .collect();

            process_audio_samples(
                samples,
                source_channels,
                &mut buffer,
                samples_per_chunk,
                &mut resampler,
                &audio_tx,
            );
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn build_stream_u32(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_tx: mpsc::Sender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    samples_per_chunk: usize,
) -> Result<cpal::Stream> {
    let mut buffer: Vec<f32> = Vec::with_capacity(samples_per_chunk * source_channels as usize * 2);
    let mut resampler = AudioResampler::new_if_needed(source_sample_rate, samples_per_chunk)?;

    let err_fn = |err| error!("Audio stream error: {}", err);

    let stream = device.build_input_stream(
        config,
        move |data: &[u32], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            // Convert u32 to f32
            let samples: Vec<f32> = data
                .iter()
                .map(|&s| ((s as f64 - (u32::MAX as f64 / 2.0)) / (u32::MAX as f64 / 2.0)) as f32)
                .collect();

            process_audio_samples(
                samples,
                source_channels,
                &mut buffer,
                samples_per_chunk,
                &mut resampler,
                &audio_tx,
            );
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn build_stream_u64(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_tx: mpsc::Sender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    samples_per_chunk: usize,
) -> Result<cpal::Stream> {
    let mut buffer: Vec<f32> = Vec::with_capacity(samples_per_chunk * source_channels as usize * 2);
    let mut resampler = AudioResampler::new_if_needed(source_sample_rate, samples_per_chunk)?;

    let err_fn = |err| error!("Audio stream error: {}", err);

    let stream = device.build_input_stream(
        config,
        move |data: &[u64], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            // Convert u64 to f32
            let samples: Vec<f32> = data
                .iter()
                .map(|&s| ((s as f64 - (u64::MAX as f64 / 2.0)) / (u64::MAX as f64 / 2.0)) as f32)
                .collect();

            process_audio_samples(
                samples,
                source_channels,
                &mut buffer,
                samples_per_chunk,
                &mut resampler,
                &audio_tx,
            );
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn build_stream_f32(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_tx: mpsc::Sender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    samples_per_chunk: usize,
) -> Result<cpal::Stream> {
    let mut buffer: Vec<f32> = Vec::with_capacity(samples_per_chunk * source_channels as usize * 2);
    let mut resampler = AudioResampler::new_if_needed(source_sample_rate, samples_per_chunk)?;

    let err_fn = |err| error!("Audio stream error: {}", err);

    let stream = device.build_input_stream(
        config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            // f32 samples are already in the correct range (-1.0 to 1.0)
            let samples: Vec<f32> = data.to_vec();

            process_audio_samples(
                samples,
                source_channels,
                &mut buffer,
                samples_per_chunk,
                &mut resampler,
                &audio_tx,
            );
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

fn build_stream_f64(
    device: &cpal::Device,
    config: &StreamConfig,
    audio_tx: mpsc::Sender<Vec<u8>>,
    is_recording: Arc<AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    samples_per_chunk: usize,
) -> Result<cpal::Stream> {
    let mut buffer: Vec<f32> = Vec::with_capacity(samples_per_chunk * source_channels as usize * 2);
    let mut resampler = AudioResampler::new_if_needed(source_sample_rate, samples_per_chunk)?;

    let err_fn = |err| error!("Audio stream error: {}", err);

    let stream = device.build_input_stream(
        config,
        move |data: &[f64], _: &cpal::InputCallbackInfo| {
            if !is_recording.load(Ordering::SeqCst) {
                return;
            }

            // Convert f64 to f32
            let samples: Vec<f32> = data.iter().map(|&s| s as f32).collect();

            process_audio_samples(
                samples,
                source_channels,
                &mut buffer,
                samples_per_chunk,
                &mut resampler,
                &audio_tx,
            );
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

/// Stateful, band-limited resampler. Keeps filter state across chunks so there
/// are no discontinuities at chunk boundaries, and low-pass filters before
/// decimation to avoid aliasing high frequencies into the speech band.
struct AudioResampler {
    inner: FftFixedIn<f32>,
}

impl AudioResampler {
    fn new_if_needed(source_rate: u32, chunk_size: usize) -> Result<Option<Self>> {
        if source_rate == TARGET_SAMPLE_RATE {
            return Ok(None);
        }
        let inner = FftFixedIn::<f32>::new(
            source_rate as usize,
            TARGET_SAMPLE_RATE as usize,
            chunk_size,
            2,
            1,
        )
        .context("Failed to create resampler")?;
        info!("Resampling {} Hz -> {} Hz", source_rate, TARGET_SAMPLE_RATE);
        Ok(Some(Self { inner }))
    }

    fn resample(&mut self, input: &[f32]) -> Vec<f32> {
        match self.inner.process(&[input], None) {
            Ok(mut out) => out.pop().unwrap_or_default(),
            Err(e) => {
                error!("Resampling failed: {}", e);
                Vec::new()
            }
        }
    }
}

impl Default for AudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms_after_resampling(freq: f32, source_rate: u32) -> (f32, Vec<usize>) {
        let chunk = (source_rate * CHUNK_DURATION_MS / 1000) as usize;
        let mut resampler = AudioResampler::new_if_needed(source_rate, chunk).unwrap().unwrap();
        let input: Vec<f32> = (0..chunk * 20)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / source_rate as f32).sin() * 0.5)
            .collect();
        let mut out = Vec::new();
        let mut lens = Vec::new();
        for c in input.chunks(chunk) {
            let r = resampler.resample(c);
            lens.push(r.len());
            out.extend(r);
        }
        // Skip the filter warm-up.
        let tail = &out[out.len() / 2..];
        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        (rms, lens)
    }

    #[test]
    fn resampler_keeps_speech_band_and_chunk_size() {
        for rate in [48000, 44100] {
            let (rms, lens) = rms_after_resampling(1000.0, rate);
            assert!(lens.iter().all(|&l| l == 1600), "{rate}: {lens:?}");
            assert!((rms - 0.5 / 2f32.sqrt()).abs() < 0.02, "{rate}: rms {rms}");
        }
    }

    #[test]
    fn resampler_removes_content_above_nyquist() {
        // Naive decimation would alias 12 kHz down to 4 kHz at full amplitude.
        let (rms, _) = rms_after_resampling(12000.0, 48000);
        assert!(rms < 0.01, "rms {rms}");
    }
}
