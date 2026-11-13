use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use std::sync::{Arc, Mutex};

const TARGET_RATE: u32 = 16_000; // Whisper works at 16 kHz mono; smaller upload = faster

/// An open microphone stream. Dropping it closes the mic.
pub struct Recorder {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    channels: u16,
}

/// Finished recording, already mixed down to 16 kHz mono.
pub struct Clip {
    pub samples: Vec<f32>,
}

impl Recorder {
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no microphone found")?;
        let supported = device
            .default_input_config()
            .context("could not read microphone settings")?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let samples = Arc::new(Mutex::new(Vec::new()));

        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, samples.clone())?,
            SampleFormat::I16 => build::<i16>(&device, &config, samples.clone())?,
            SampleFormat::U16 => build::<u16>(&device, &config, samples.clone())?,
            SampleFormat::I32 => build::<i32>(&device, &config, samples.clone())?,
            SampleFormat::U8 => build::<u8>(&device, &config, samples.clone())?,
            other => bail!("unsupported microphone sample format {other:?}"),
        };
        stream.play().context("could not start microphone")?;

        Ok(Self {
            stream,
            samples,
            rate: config.sample_rate.0,
            channels: config.channels,
        })
    }

    pub fn stop(self) -> Clip {
        drop(self.stream);
        let raw = std::mem::take(&mut *self.samples.lock().unwrap());
        let mono = downmix(&raw, self.channels);
        Clip {
            samples: resample(&mono, self.rate, TARGET_RATE),
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buf: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if let Ok(mut b) = buf.lock() {
                b.extend(data.iter().map(|s| s.to_sample::<f32>()));
            }
        },
        |err| eprintln!("microphone error: {err}"),
        None,
    )?;
    Ok(stream)
}

fn downmix(raw: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if ch == 1 {
        return raw.to_vec();
    }
    raw.chunks(ch)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let out_len = (input.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * step;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx.min(input.len() - 1)];
            let b = input[(idx + 1).min(input.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

impl Clip {
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / TARGET_RATE as f32
    }

    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// Cut the quiet parts before the first and after the last loud moment, keeping a
    /// little padding. Less to upload, and less silence for Whisper to invent words from.
    pub fn trim_silence(&mut self) {
        const WIN: usize = (TARGET_RATE / 50) as usize; // 20 ms windows
        const PAD: usize = (TARGET_RATE / 4) as usize; // keep 250 ms around speech
        let rms: Vec<f32> = self
            .samples
            .chunks(WIN)
            .map(|w| (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt())
            .collect();
        let loudest = rms.iter().copied().fold(0.0f32, f32::max);
        let threshold = (loudest * 0.1).max(0.004);
        let (Some(first), Some(last)) = (
            rms.iter().position(|&r| r > threshold),
            rms.iter().rposition(|&r| r > threshold),
        ) else {
            return;
        };
        let start = (first * WIN).saturating_sub(PAD);
        let end = ((last + 1) * WIN + PAD).min(self.samples.len());
        self.samples = self.samples[start..end].to_vec();
    }

    /// 16-bit PCM WAV file in memory.
    pub fn to_wav(&self) -> Vec<u8> {
        let data_len = (self.samples.len() * 2) as u32;
        let mut w = Vec::with_capacity(44 + data_len as usize);
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        w.extend_from_slice(&1u16.to_le_bytes()); // PCM
        w.extend_from_slice(&1u16.to_le_bytes()); // mono
        w.extend_from_slice(&TARGET_RATE.to_le_bytes());
        w.extend_from_slice(&(TARGET_RATE * 2).to_le_bytes()); // byte rate
        w.extend_from_slice(&2u16.to_le_bytes()); // block align
        w.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        for s in &self.samples {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            w.extend_from_slice(&v.to_le_bytes());
        }
        w
    }
}

/// Play a short quiet tone on the default speaker without blocking the caller.
pub fn beep(freq: f32, ms: u64) {
    std::thread::spawn(move || {
        if let Err(e) = play_tone(freq, ms) {
            eprintln!("beep failed: {e:#}");
        }
    });
}

fn play_tone(freq: f32, ms: u64) -> Result<()> {
    let device = cpal::default_host()
        .default_output_device()
        .context("no speaker found")?;
    let supported = device.default_output_config()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => tone::<f32>(&device, &config, freq, ms)?,
        SampleFormat::I16 => tone::<i16>(&device, &config, freq, ms)?,
        SampleFormat::U16 => tone::<u16>(&device, &config, freq, ms)?,
        SampleFormat::I32 => tone::<i32>(&device, &config, freq, ms)?,
        other => bail!("unsupported speaker sample format {other:?}"),
    };
    stream.play()?;
    std::thread::sleep(std::time::Duration::from_millis(ms + 100));
    Ok(())
}

fn tone<T>(device: &cpal::Device, config: &cpal::StreamConfig, freq: f32, ms: u64) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let rate = config.sample_rate.0 as f32;
    let channels = config.channels.max(1) as usize;
    let total = (rate * ms as f32 / 1000.0) as usize;
    let fade = rate * 0.005; // 5 ms fade in/out avoids clicks
    let mut n = 0usize;
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                let v = if n < total {
                    let env = (n.min(total - n) as f32 / fade).min(1.0);
                    (n as f32 / rate * freq * std::f32::consts::TAU).sin() * 0.12 * env
                } else {
                    0.0
                };
                n += 1;
                for s in frame.iter_mut() {
                    *s = T::from_sample_(v);
                }
            }
        },
        |err| eprintln!("speaker error: {err}"),
        None,
    )?;
    Ok(stream)
}
