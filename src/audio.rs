use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub const TARGET_RATE: u32 = 16_000; // Whisper works at 16 kHz mono; smaller upload = faster

/// An open microphone stream. Audio is converted to 16 kHz mono as it arrives, so memory
/// stays small (~2 MB per minute) and pieces can be taken off while recording continues.
/// Dropping it closes the mic.
pub struct Recorder {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    level: Level,
}

/// Loudness of the latest audio chunk (RMS, as f32 bits), for the listening indicator.
pub type Level = Arc<AtomicU32>;

pub fn read_level(level: &AtomicU32) -> f32 {
    f32::from_bits(level.load(Ordering::Relaxed))
}

/// A piece of recording, 16 kHz mono.
pub struct Clip {
    pub samples: Vec<f32>,
}

/// Turns the device's stream (any rate, any channel count) into 16 kHz mono, chunk by
/// chunk, carrying its position across callbacks.
struct Converter {
    channels: usize,
    step: f64, // input samples per output sample
    pos: f64,  // position of the next output sample, relative to `tail`
    tail: Option<f32>,
}

impl Converter {
    fn new(rate: u32, channels: u16) -> Self {
        Self {
            channels: channels.max(1) as usize,
            step: rate as f64 / TARGET_RATE as f64,
            pos: 0.0,
            tail: None,
        }
    }

    fn push(&mut self, interleaved: &[f32], out: &mut Vec<f32>) {
        let mono = interleaved
            .chunks(self.channels)
            .map(|f| f.iter().sum::<f32>() / f.len() as f32);
        let buf: Vec<f32> = self.tail.into_iter().chain(mono).collect();
        if buf.len() < 2 {
            self.tail = buf.last().copied();
            return;
        }
        while self.pos + 1.0 < buf.len() as f64 {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            out.push(buf[i] + (buf[i + 1] - buf[i]) * frac);
            self.pos += self.step;
        }
        // keep the last input sample for interpolation across the next callback
        self.pos -= (buf.len() - 1) as f64;
        self.tail = buf.last().copied();
    }
}

impl Recorder {
    /// Record the chosen microphone (Linux: the sound server routes to it; Windows: by name).
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        #[cfg(windows)]
        let device = crate::mics::chosen_input(&host);
        #[cfg(not(windows))]
        let device = host.default_input_device();
        Self::from_device(&device.context("no microphone found")?, false)
    }

    /// Record from a device. `loopback`: an output device, recording what it plays
    /// (Windows WASAPI loopback).
    pub fn from_device(device: &cpal::Device, loopback: bool) -> Result<Self> {
        let supported = if loopback { device.default_output_config() } else { device.default_input_config() }
            .context("could not read the audio device settings")?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let level: Level = Arc::default();
        let conv = Converter::new(config.sample_rate.0, config.channels);

        let stream = match format {
            SampleFormat::F32 => build::<f32>(device, &config, samples.clone(), level.clone(), conv)?,
            SampleFormat::I16 => build::<i16>(device, &config, samples.clone(), level.clone(), conv)?,
            SampleFormat::U16 => build::<u16>(device, &config, samples.clone(), level.clone(), conv)?,
            SampleFormat::I32 => build::<i32>(device, &config, samples.clone(), level.clone(), conv)?,
            SampleFormat::U8 => build::<u8>(device, &config, samples.clone(), level.clone(), conv)?,
            other => bail!("unsupported microphone sample format {other:?}"),
        };
        stream.play().context("could not start microphone")?;
        Ok(Self { stream, samples, level })
    }

    /// Live loudness, updated with every audio chunk.
    pub fn level(&self) -> Level {
        self.level.clone()
    }

    /// Seconds recorded and not yet taken.
    #[allow(dead_code)]
    pub fn buffered_seconds(&self) -> f32 {
        self.samples.lock().unwrap().len() as f32 / TARGET_RATE as f32
    }

    /// Take a piece off the front while recording continues: once at least `min` seconds
    /// are buffered, cut at the quietest moment (a pause between words or sentences)
    /// after `min`; if there is no real pause by `max` seconds, cut at the quietest
    /// moment anyway. None = not yet.
    pub fn take_piece(&self, min: f32, max: f32) -> Option<Clip> {
        cut_piece(&mut self.samples.lock().unwrap(), min, max)
    }

    pub fn stop(self) -> Clip {
        drop(self.stream);
        Clip {
            samples: std::mem::take(&mut *self.samples.lock().unwrap()),
        }
    }
}

/// Take a piece off the front of a 16 kHz buffer if one is ready (see `take_piece`).
pub fn cut_piece(buf: &mut Vec<f32>, min: f32, max: f32) -> Option<Clip> {
    let cut = find_cut(buf, min, max)?;
    Some(Clip {
        samples: buf.drain(..cut).collect(),
    })
}

/// Where to cut a piece (sample index), see `take_piece`.
fn find_cut(buf: &[f32], min: f32, max: f32) -> Option<usize> {
    const WIN: usize = (TARGET_RATE / 50) as usize; // 20 ms
    const SPAN: usize = 15; // 15 windows = 300 ms of quiet
    let rate = TARGET_RATE as f32;
    if (buf.len() as f32) < min * rate {
        return None;
    }
    let rms: Vec<f32> = buf
        .chunks(WIN)
        .map(|w| (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt())
        .collect();
    let loudest = rms.iter().copied().fold(0.0f32, f32::max);
    let first = (min * rate) as usize / WIN;
    if rms.len() < first + SPAN {
        return None;
    }
    // quietest 300 ms stretch between `min` and `max` (a long kept recording is cut piece
    // by piece, never searched as a whole)
    let last = rms.len().min(((max * rate) as usize / WIN).max(first + SPAN));
    let (mut best_at, mut best) = (first, f32::MAX);
    for start in first..=last - SPAN {
        let avg = rms[start..start + SPAN].iter().sum::<f32>() / SPAN as f32;
        if avg < best {
            best = avg;
            best_at = start;
        }
    }
    let is_pause = best < (loudest * 0.12).max(0.003);
    let forced = buf.len() as f32 >= max * rate;
    (is_pause || forced).then_some((best_at + SPAN / 2) * WIN)
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buf: Arc<Mutex<Vec<f32>>>,
    level: Level,
    mut conv: Converter,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut scratch: Vec<f32> = Vec::new();
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            scratch.clear();
            scratch.extend(data.iter().map(|s| s.to_sample::<f32>()));
            if !scratch.is_empty() {
                let rms = (scratch.iter().map(|s| s * s).sum::<f32>() / scratch.len() as f32).sqrt();
                level.store(rms.to_bits(), Ordering::Relaxed);
            }
            if let Ok(mut b) = buf.lock() {
                conv.push(&scratch, &mut b);
            }
        },
        |err| eprintln!("microphone error: {err}"),
        None,
    )?;
    Ok(stream)
}

impl Clip {
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / TARGET_RATE as f32
    }

    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// Loudness of each 20 ms window.
    fn window_rms(&self) -> Vec<f32> {
        self.samples
            .chunks((TARGET_RATE / 50) as usize)
            .map(|w| (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt())
            .collect()
    }

    /// Level that separates sound from the room's background noise, or None if the
    /// clip has no clear sound at all (flat noise: fan, hum, silence).
    fn sound_threshold(rms: &[f32]) -> Option<f32> {
        if rms.is_empty() {
            return None;
        }
        let mut sorted = rms.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let floor = sorted[sorted.len() / 10]; // background noise level
        let top = sorted[sorted.len() * 95 / 100];
        if top < (floor * 3.0).max(0.008) {
            return None; // nothing stands out from the background
        }
        Some((floor * 2.5).max(top * 0.15).max(0.005))
    }

    /// Seconds that sound like speech: stretches of at least 100 ms clearly louder than
    /// the background. Clicks, taps and a door (a few ms) don't count; flat noise doesn't
    /// either.
    pub fn speech_seconds(&self) -> f32 {
        let rms = self.window_rms();
        let Some(thr) = Self::sound_threshold(&rms) else { return 0.0 };
        let mut total = 0;
        let mut run = 0;
        for r in rms.iter().chain(std::iter::once(&0.0)) {
            if *r > thr {
                run += 1;
            } else {
                if run >= 5 {
                    total += run;
                }
                run = 0;
            }
        }
        total as f32 * 0.02
    }

    /// Is there enough speech to be worth sending?
    pub fn has_speech(&self) -> bool {
        self.speech_seconds() >= 0.4
    }

    /// Shorten pauses longer than 1 s inside the clip to 0.6 s: nobody pays for silence,
    /// and Whisper has less silence to invent text from.
    pub fn shorten_pauses(&mut self) {
        const WIN: usize = (TARGET_RATE / 50) as usize;
        const KEEP: usize = 15; // windows kept on each side of a pause (0.3 s)
        let rms = self.window_rms();
        let Some(thr) = Self::sound_threshold(&rms) else { return };
        let mut out = Vec::with_capacity(self.samples.len());
        let mut i = 0;
        while i < rms.len() {
            if rms[i] > thr {
                let end = ((i + 1) * WIN).min(self.samples.len());
                out.extend_from_slice(&self.samples[i * WIN..end]);
                i += 1;
                continue;
            }
            let start = i;
            while i < rms.len() && rms[i] <= thr {
                i += 1;
            }
            let (a, b) = (start * WIN, (i * WIN).min(self.samples.len()));
            if i - start > 50 {
                out.extend_from_slice(&self.samples[a..a + KEEP * WIN]);
                out.extend_from_slice(&self.samples[b - KEEP * WIN..b]);
            } else {
                out.extend_from_slice(&self.samples[a..b]);
            }
        }
        self.samples = out;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converter_keeps_length_and_tone() {
        // 1 s of 48 kHz stereo 440 Hz, fed in uneven callback sizes
        let input: Vec<f32> = (0..48_000)
            .flat_map(|i| {
                let v = (i as f32 / 48_000.0 * 440.0 * std::f32::consts::TAU).sin() * 0.5;
                [v, v]
            })
            .collect();
        let mut conv = Converter::new(48_000, 2);
        let mut out = Vec::new();
        for chunk in input.chunks(2 * 441) {
            conv.push(chunk, &mut out);
        }
        assert!((out.len() as i64 - 16_000).abs() <= 2, "{}", out.len());
        let expect = |i: usize| (i as f32 / 16_000.0 * 440.0 * std::f32::consts::TAU).sin() * 0.5;
        let worst = (0..out.len()).map(|i| (out[i] - expect(i)).abs()).fold(0.0, f32::max);
        assert!(worst < 0.02, "max error {worst}");
    }

    fn noise(n: usize, amp: f32) -> impl Iterator<Item = f32> {
        let mut x: u32 = 12345;
        (0..n).map(move |_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            ((x >> 16) as f32 / 65_535.0 - 0.5) * 2.0 * amp
        })
    }

    #[test]
    fn speech_detector() {
        let rate = TARGET_RATE as usize;
        let tone = |n: usize| (0..n).map(|i| (i as f32 / 16_000.0 * 220.0 * std::f32::consts::TAU).sin() * 0.2);
        // fan noise only: not speech
        let fan = Clip { samples: noise(20 * rate, 0.02).collect() };
        assert_eq!(fan.speech_seconds(), 0.0);
        assert!(!fan.has_speech());
        // silence only
        assert!(!Clip { samples: vec![0.0; 20 * rate] }.has_speech());
        // noise with 10 key clicks of 20 ms: not speech
        let mut clicks: Vec<f32> = noise(20 * rate, 0.005).collect();
        for k in 0..10 {
            let at = k * 2 * rate;
            for (j, v) in tone(rate / 50).enumerate() {
                clicks[at + j] = v * 3.0;
            }
        }
        assert!(!Clip { samples: clicks }.has_speech());
        // noise with 2 s of "voice": speech
        let mut talk: Vec<f32> = noise(20 * rate, 0.005).collect();
        for (j, v) in tone(2 * rate).enumerate() {
            talk[5 * rate + j] += v;
        }
        let c = Clip { samples: talk };
        assert!(c.has_speech());
        assert!((1.8..2.2).contains(&c.speech_seconds()), "{}", c.speech_seconds());
    }

    #[test]
    fn long_pauses_are_shortened() {
        let rate = TARGET_RATE as usize;
        let tone = |n: usize| (0..n).map(|i| (i as f32 / 16_000.0 * 220.0 * std::f32::consts::TAU).sin() * 0.2);
        // 2 s voice, 8 s pause, 2 s voice, 0.5 s pause, 2 s voice
        let mut s: Vec<f32> = tone(2 * rate).collect();
        s.extend(noise(8 * rate, 0.003));
        s.extend(tone(2 * rate));
        s.extend(noise(rate / 2, 0.003));
        s.extend(tone(2 * rate));
        let mut c = Clip { samples: s };
        c.shorten_pauses();
        let secs = c.seconds();
        // 6 s voice + 0.6 s (shortened) + 0.5 s (short pause kept)
        assert!((6.9..7.3).contains(&secs), "{secs}");
    }

    #[test]
    fn cuts_at_the_pause() {
        let rate = TARGET_RATE as usize;
        let speech = |n: usize| (0..n).map(|i| ((i % 40) as f32 / 40.0 - 0.5) * 0.6);
        // 25 s speech, 0.6 s silence, 10 s speech
        let mut buf: Vec<f32> = speech(25 * rate).collect();
        buf.extend(std::iter::repeat_n(0.0, rate * 6 / 10));
        buf.extend(speech(10 * rate));
        let cut = find_cut(&buf, 20.0, 45.0).unwrap() as f32 / rate as f32;
        assert!((25.0..25.6).contains(&cut), "cut at {cut}");
        // too short yet
        assert!(find_cut(&buf[..15 * rate], 20.0, 45.0).is_none());
        // no pause and below max: wait
        let talk: Vec<f32> = speech(30 * rate).collect();
        assert!(find_cut(&talk, 20.0, 45.0).is_none());
        // no pause but at max: forced cut
        let talk: Vec<f32> = speech(46 * rate).collect();
        assert!(find_cut(&talk, 20.0, 45.0).is_some());
        // a long recording: the cut stays within `max`, even if a later pause is quieter
        let mut long: Vec<f32> = speech(30 * rate).collect();
        long.extend(std::iter::repeat_n(0.002, rate / 2)); // a soft pause at 30 s
        long.extend(speech(60 * rate));
        long.extend(std::iter::repeat_n(0.0, rate)); // a silent pause at 90.5 s
        long.extend(speech(10 * rate));
        let cut = find_cut(&long, 20.0, 45.0).unwrap() as f32 / rate as f32;
        assert!((30.0..30.6).contains(&cut), "cut at {cut}");
    }
}
