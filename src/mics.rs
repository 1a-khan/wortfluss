//! Choosing the microphone (Linux).
//!
//! The sound server (PipeWire, or PulseAudio) knows every input with a readable name
//! ("Digital Microphone", "Headset Microphone", a Bluetooth headset, a USB mic), while
//! ALSA only shows cryptic names like `hw:CARD=sofhdadsp,DEV=6`. So we list the inputs
//! over the PulseAudio protocol, which PipeWire also speaks, and record from the chosen
//! one by naming it in `PIPEWIRE_NODE` / `PULSE_SOURCE`, which the ALSA plugins that
//! vtt records through honour. If the chosen mic is unplugged, the sound server falls
//! back to the default one.
//! (A Windows port lists devices with their names directly through cpal/WASAPI.)

use anyhow::{anyhow, Context, Result};
use pulseaudio::protocol;
use std::ffi::CString;
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Mic {
    /// Sound-server name, stored in config.toml (stable across reboots).
    pub name: String,
    /// Readable name, e.g. "Digital Microphone".
    pub description: String,
    pub is_default: bool,
    pub muted: bool,
}

type Sock = BufReader<UnixStream>;

fn perr(e: protocol::ProtocolError) -> anyhow::Error {
    anyhow!("sound server: {e}")
}

/// Connect and log in to the sound server. Returns the socket and protocol version.
fn connect() -> Result<(Sock, u16)> {
    let path = pulseaudio::socket_path_from_env().context("no sound server found (PipeWire/PulseAudio)")?;
    let stream = UnixStream::connect(&path).with_context(|| format!("cannot reach the sound server at {}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut sock = BufReader::new(stream);
    let cookie = pulseaudio::cookie_path_from_env()
        .and_then(|p| std::fs::read(p).ok())
        .unwrap_or_default();
    let auth = protocol::AuthParams {
        version: protocol::MAX_VERSION,
        supports_shm: false,
        supports_memfd: false,
        cookie,
    };
    protocol::write_command_message(sock.get_mut(), 0, &protocol::Command::Auth(auth), protocol::MAX_VERSION)
        .map_err(perr)?;
    let (_, reply) = protocol::read_reply_message::<protocol::AuthReply>(&mut sock, protocol::MAX_VERSION).map_err(perr)?;
    let version = protocol::MAX_VERSION.min(reply.version);

    let mut props = protocol::Props::new();
    props.set(protocol::Prop::ApplicationName, CString::new("vtt").unwrap());
    protocol::write_command_message(sock.get_mut(), 1, &protocol::Command::SetClientName(props), version).map_err(perr)?;
    protocol::read_reply_message::<protocol::SetClientNameReply>(&mut sock, version).map_err(perr)?;
    Ok((sock, version))
}

/// All inputs except "Monitor of ..." (recordings of what the speakers play).
pub fn list() -> Result<Vec<Mic>> {
    let (mut sock, version) = connect()?;
    let err = perr;
    protocol::write_command_message(sock.get_mut(), 2, &protocol::Command::GetServerInfo, version).map_err(err)?;
    let (_, server) = protocol::read_reply_message::<protocol::ServerInfo>(&mut sock, version).map_err(err)?;
    let default = server
        .default_source_name
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    protocol::write_command_message(sock.get_mut(), 3, &protocol::Command::GetSourceInfoList, version).map_err(err)?;
    let (_, sources) = protocol::read_reply_message::<protocol::SourceInfoList>(&mut sock, version).map_err(err)?;

    Ok(sources
        .into_iter()
        .filter(|s| s.monitor_of_sink_name.is_none() && s.monitor_of_sink_index.is_none())
        .map(|s| {
            let name = s.name.to_string_lossy().into_owned();
            Mic {
                description: s
                    .description
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_else(|| name.clone()),
                is_default: name == default,
                muted: s.muted,
                name,
            }
        })
        .collect::<Vec<_>>())
    .map(readable)
}

/// "Alder Lake PCH-P High Definition Audio Controller Digital Microphone" ->
/// "Digital Microphone (Alder Lake PCH-P High Definition Audio Controller)": when several
/// inputs start with the same sound-card name, put the part that differs first.
fn readable(mut mics: Vec<Mic>) -> Vec<Mic> {
    if mics.len() < 2 {
        return mics;
    }
    let words: Vec<Vec<&str>> = mics.iter().map(|m| m.description.split(' ').collect()).collect();
    let mut common = 0;
    while words.iter().all(|w| w.len() > common + 1 && w[common] == words[0][common]) {
        common += 1;
    }
    if common < 2 {
        return mics;
    }
    let card = words[0][..common].join(" ");
    let short: Vec<String> = words.iter().map(|w| w[common..].join(" ")).collect();
    for (m, s) in mics.iter_mut().zip(short) {
        m.description = format!("{s} ({card})");
    }
    mics
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mic(d: &str) -> Mic {
        Mic { name: d.into(), description: d.into(), is_default: false, muted: false }
    }

    #[test]
    fn short_names_first() {
        let m = readable(vec![
            mic("Alder Lake PCH-P High Definition Audio Controller Stereo Microphone"),
            mic("Alder Lake PCH-P High Definition Audio Controller Digital Microphone"),
        ]);
        assert_eq!(m[1].description, "Digital Microphone (Alder Lake PCH-P High Definition Audio Controller)");
        // a USB mic shares no card name: names stay as they are
        let m = readable(vec![mic("Alder Lake Digital Microphone"), mic("Jabra Evolve2 65 Mono")]);
        assert_eq!(m[0].description, "Alder Lake Digital Microphone");
    }
}

/// Record from this input from now on ("" = the system default). Called before any
/// recording starts and before other threads exist (it changes the environment).
pub fn select(name: &str) {
    let name = name.trim();
    for var in ["PIPEWIRE_NODE", "PULSE_SOURCE"] {
        if name.is_empty() {
            std::env::remove_var(var);
        } else {
            std::env::set_var(var, name);
        }
    }
}

/// Readable name for a stored mic setting.
pub fn describe(name: &str) -> String {
    if name.trim().is_empty() {
        let now = list()
            .ok()
            .and_then(|l| l.into_iter().find(|m| m.is_default))
            .map(|m| format!(" (now: {})", m.description))
            .unwrap_or_default();
        return format!("system default{now}");
    }
    match list() {
        Ok(l) => match l.into_iter().find(|m| m.name == name) {
            Some(m) => m.description,
            None => format!("{name} (not connected: the default mic is used)"),
        },
        Err(_) => name.to_string(),
    }
}

// ---- recording a named source (session mode) ----

/// Speaker output ("what you hear"): the monitor of the current default output, so it
/// follows a switch to headphones.
pub const SYSTEM_AUDIO: &str = "@DEFAULT_MONITOR@";
/// The default microphone.
pub const DEFAULT_MIC: &str = "@DEFAULT_SOURCE@";

/// Records one sound-server source as 16 kHz mono (the server converts), so several
/// sources can be recorded at once, each with its own label. Used by session mode.
pub struct SourceRecorder {
    samples: Arc<Mutex<Vec<f32>>>,
    socket: UnixStream,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SourceRecorder {
    pub fn start(source: &str) -> Result<Self> {
        let (mut sock, version) = connect()?;
        let params = protocol::RecordStreamParams {
            sample_spec: protocol::SampleSpec {
                format: protocol::SampleFormat::S16Le,
                channels: 1,
                sample_rate: crate::audio::TARGET_RATE,
            },
            channel_map: protocol::ChannelMap::mono(),
            source_name: Some(CString::new(source)?),
            cvolume: Some(protocol::ChannelVolume::norm(1)),
            ..Default::default()
        };
        protocol::write_command_message(sock.get_mut(), 2, &protocol::Command::CreateRecordStream(params), version)
            .map_err(perr)?;
        let (_, reply) = protocol::read_reply_message::<protocol::CreateRecordStreamReply>(&mut sock, version)
            .map_err(perr)
            .with_context(|| format!("cannot record from `{source}`"))?;
        if reply.sample_spec.format != protocol::SampleFormat::S16Le || reply.sample_spec.channels != 1 {
            anyhow::bail!("sound server offered an unexpected format: {:?}", reply.sample_spec);
        }
        // No timeout while streaming: a silent, suspended output may send nothing for a while.
        sock.get_ref().set_read_timeout(None)?;
        let socket = sock.get_ref().try_clone()?;
        let samples = Arc::new(Mutex::new(Vec::new()));
        let buf = samples.clone();
        let thread = std::thread::spawn(move || {
            use std::io::Read;
            let mut data = Vec::new();
            loop {
                let Ok(desc) = protocol::read_descriptor(&mut sock) else { return };
                if desc.channel == u32::MAX {
                    // a message from the server (e.g. stream moved): read and ignore
                    if protocol::Command::read_tag_prefixed(&mut sock, version).is_err() {
                        return;
                    }
                    continue;
                }
                data.resize(desc.length as usize, 0);
                if sock.read_exact(&mut data).is_err() {
                    return;
                }
                let mut b = buf.lock().unwrap();
                b.extend(
                    data.chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0),
                );
            }
        });
        Ok(Self { samples, socket, thread: Some(thread) })
    }

    /// See `audio::Recorder::take_piece`.
    pub fn take_piece(&self, min: f32, max: f32) -> Option<crate::audio::Clip> {
        crate::audio::cut_piece(&mut self.samples.lock().unwrap(), min, max)
    }

    pub fn stop(mut self) -> crate::audio::Clip {
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        crate::audio::Clip {
            samples: std::mem::take(&mut *self.samples.lock().unwrap()),
        }
    }
}
