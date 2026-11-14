# Wortfluss: voice to text, anywhere

_Command: `vtt`._

Hold **Right Ctrl** (or a key you choose), speak, let go. The text is typed into whatever app
has focus: browser, chat, editor, terminal. Works with Groq, OpenAI, Mistral or your own server.

One self-contained binary: records the mic, calls the speech service over HTTPS and types
through its own virtual keyboard. No ydotool, xdotool or clipboard tools. Wayland and X11.

## Quick start

```bash
vtt setup     # answers a few questions, checks your key, can start vtt at login
```

That's all. `vtt setup` can be run again any time to change something; your current choices
are preselected, so Enter keeps them. A running vtt switches to the new settings by itself
within a few seconds.

| Command | What it does |
|---|---|
| `vtt setup` | first time: guided setup; afterwards: menu to change one setting at a time |
| `vtt set <setting>` | change one thing: service, model, key, correction, phrases, language, hotkey, mode, ... |
| `vtt phrases [add\|remove\|clear]` | names and words to spell right |
| `vtt mics` / `vtt set mic` | list microphones / choose one (with a 3-second test) |
| `vtt retry` | transcribe pieces that failed earlier (e.g. no internet) |
| `vtt session start [title] [--source system\|mic\|both]` | transcribe a lecture, video or meeting into a Markdown file |
| `vtt session stop` / `status` | stop (notes are written) / show the running session |
| `vtt notes FILE [lecture\|meeting\|summary]` | (re)write notes for a transcript |
| `vtt help [topic]` | built-in guide: models, correction, hotkey, layout, security, costs, files, trouble |
| `vtt check` | tests everything (keyboard access, mic, key, model, autostart) and says what to fix |
| `vtt start` / `vtt stop` | start in the background / stop |
| `vtt` | run in this terminal (shows what it types; asks setup questions on first run) |
| `vtt usage` | audio sent today / this month / all time, with estimated cost |
| `vtt autostart on\|off` | start at login or not |
| `vtt test-type` / `vtt test-mic` | type a test sentence / record 4 s to `vtt-test.wav` |

## Which service and model?

| Service | Model | Price per hour of audio* | Choose it when |
|---|---|---|---|
| **Groq** | **`whisper-large-v3-turbo`** (recommended) | $0.04 | default: fastest, cheapest, free plan to start |
| Groq | `whisper-large-v3` | $0.111 | strong accents, mixed languages |
| OpenAI | `gpt-4o-mini-transcribe` | ~$0.18 | best punctuation and formatting, paid only |
| OpenAI | `gpt-4o-transcribe` / newer `gpt-transcribe` | ~$0.27–0.36 | highest accuracy |
| **OpenRouter** | **`openai/whisper-large-v3-turbo`** (recommended) | ~$0.012 | one key for 30+ models; exact cost per request in `vtt usage` |
| OpenRouter | `elevenlabs/scribe-v2`, `deepgram/nova-3`, `openai/gpt-4o-mini-transcribe` … | ~$0.18–0.30 | try other engines without new accounts |
| Mistral | `voxtral-mini-latest` | ~$0.18 | European languages; EU company |
| Custom | whatever the server offers | your hardware | privacy: self-hosted Whisper, nothing leaves your network |

\* List prices, Nov 2026; providers change them.

**Choosing a model in setup:** the recommended model is preselected. Or choose "Choose from all
N speech models", which loads the service's live list (names and prices, type to filter), so
nobody has to look up model names online. Or type any model name.

**AWS:** Amazon Bedrock has no working speech-to-text (its Voxtral models don't accept audio).
AWS's speech service is Amazon Transcribe, which is not supported yet. Groq bills each request as at least 10 s
(about $0.0001 for a short dictation, so $1 covers about 9,000), and its free plan costs nothing
until you hit rate limits. `vtt usage` shows your own numbers.

Language: "detect automatically" works well if you switch languages. A fixed language is a bit
faster and avoids rare mix-ups. **Spelling help** (names, brands) fixes words like "Grog" → "Groq".

## Tray icon and keyboard shortcuts

The vtt icon in the top bar (system tray) has a right-click menu: start/stop a session
(computer audio, microphone, call), open the transcripts folder, and the common settings
(correction, hold/toggle, microphone, notes type, keep session audio, beeps), plus
"All settings..." and "Quit". On GNOME this needs the AppIndicator extension (on by default
on Ubuntu); KDE shows it natively.

| Shortcut | Action |
|---|---|
| Ctrl+Alt+S | start/stop a session (what the computer plays) |
| Ctrl+Alt+M | start/stop a call session (You / Others) |
| Ctrl+Alt+K | spelling correction on/off |
| Ctrl+Alt+H | hold <-> toggle |
| Ctrl+Alt+V | open all settings |

Change them by pressing the new combination: `vtt set shortcuts`. They are read from the
keyboard like the dictation key, so the focused app also receives them.

## Session mode: lectures, videos, meetings

Instead of typing, vtt writes everything said into `~/Documents/vtt/YYYY-MM-DD HH-MM Title.md`,
live, with `[hh:mm:ss]` times, and writes notes at the top when the session stops.

| Source | For | Labels |
|---|---|---|
| `system` (default) | online lectures, videos, tutorials: what the computer plays | none |
| `mic` | a lecture or talk in the room | none |
| `both` | calls and online meetings (use headphones) | **You** (mic) / **Others** (computer) |

- Notes: lecture notes (summary, key points, terms, questions), meeting minutes (decisions,
  action items), or a summary; written by the text model of `vtt set correction`.
  `vtt notes FILE TYPE` redoes them without touching the transcript.
- Text only by default. Optional: keep the audio (`--keep-audio` or `vtt set session`); a
  notification then asks to delete it, it expires after 7 days, and `vtt session redo FILE`
  transcribes it again. Silence is skipped; a session stops after 30 min without speech.
- Call sessions warn when no headphones are connected (echo would duplicate text).
- Optional session key (press to start, again to stop) and defaults: `vtt set session`.
- Cost for 1 hour: ~$0.01 transcript (OpenRouter Whisper Turbo) + ~$0.002 notes.
- **Consent:** only record other people with their permission (in Germany § 201 StGB).

## Spelling & grammar correction (optional)

Off by default: vtt types exactly what was heard. Turn it on in `vtt setup` (step 4) and a small
text model of the same service fixes spelling, capitals, commas and grammar before typing,
without translating or rewording. It never answers a dictated question; if the result looks
wrong (empty, much longer or shorter), vtt types the original instead.

| Service | Recommended correction model | Extra time |
|---|---|---|
| Groq | `qwen/qwen3.8-27b` | ~0.4 s |
| OpenRouter | `anthropic/claude-haiku-5.5` | ~0.5–1 s |
| OpenAI | `gpt-6-luna` | ~0.5–1 s |
| Mistral | `mistral-small-latest` | ~0.5–1 s |

Cost: about 100–300 tokens per dictation, a fraction of a cent (Groq's free plan covers it).
Setup can also list every text model of the service, and it runs a test sentence before saving.

## Dictation key

In `vtt setup` you just **press** the key you want. Good choices: Right Ctrl, Menu, Pause,
Scroll Lock, Insert, F9–F12. Setup warns about bad ones (letters, Shift, AltGr on German/Turkish
keyboards). Hold mode starts on press and types on release; using the key in a shortcut
(Ctrl+C) cancels silently. Toggle mode: press to start, press to stop.

## Self-detection

- **Keyboard layout**: follows the layout the desktop is using right now, including switching
  German ↔ Turkish (read from GNOME's settings; other desktops use `/etc/default/keyboard`).
  Or set a fixed one in setup.
- **Checks**: setup and `vtt check` test keyboard access, the virtual keyboard, the mic, the key
  and the model, and print the fix for anything missing.
- **Single instance**: starting vtt twice is harmless; the second one says it's already running.
- **Settings reload**: a running vtt notices changed settings and restarts itself when idle.

## One-time permission setup (Linux, every machine)

vtt reads the hotkey from `/dev/input` and types through `/dev/uinput`. Both need the `input` group:

```bash
sudo usermod -aG input "$USER"
echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' \
  | sudo tee /etc/udev/rules.d/80-vtt-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then **log out and back in**. `vtt check` confirms it worked.

> Members of the `input` group can read all keyboard input. That's the cost of a global hotkey
> on Wayland. Use this on machines where you're the only user.

## Files

| What | Where |
|---|---|
| settings (no keys) | `~/.config/vtt/config.toml` |
| API keys, encrypted | `~/.config/vtt/secrets.env` |
| usage log (time, seconds, model; no text) | `~/.local/share/vtt/usage.tsv` |
| background log (no dictated text) | `~/.local/state/vtt/vtt.log` |
| start at login | `~/.config/autostart/vtt.desktop` |

## API key security

`vtt setup` never writes keys into `config.toml`. They go to `secrets.env`, encrypted with
ChaCha20-Poly1305. The 256-bit master key is created once and stored in your OS keyring
(GNOME Keyring / KWallet on Linux, Credential Manager on Windows), which is itself encrypted
and unlocked when you log in.

- **A leaked `secrets.env`** (git, backup, shared folder, another computer) is useless without
  your keyring. Each value is also tied to its name, so values can't be swapped around.
- **Not protected:** someone logged in as you on your computer can unlock the keyring as vtt
  does. No local program can prevent that.
- **No keyring** (headless systems): vtt falls back to a key derived from the machine id and user
  id, so the file still doesn't work elsewhere, but it's weaker. `vtt check` says which is used.
- Environment variables (`GROQ_API_KEY`, ...) still override and may hold a plain key or an
  `enc:` value copied from `secrets.env`.
- Keyring reset or a new computer: run `vtt setup` and enter the key again.

## Build

```bash
sudo apt install build-essential pkg-config libasound2-dev libxkbcommon-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo build --release && cp target/release/vtt ~/.local/bin/
```

At runtime the binary only needs `libasound.so.2` and `libxkbcommon.so.0`, which every desktop
Linux has. Copy `vtt` to another machine and run `vtt setup` there.

## Limits

- Characters your layout has no key for (Urdu script, emoji) are typed with **Ctrl+Shift+U**
  Unicode input. That works in GTK/GNOME apps but not everywhere (some terminals).

## Reliability: supervision (Erlang/OTP style)

```
vtt run              supervisor: restarts the worker after a crash (permanent child)
 └─ vtt worker       hotkey, recording, transcription, typing, tray
     ├─ keyboard manager   re-attaches keyboards (also newly plugged-in ones)
     ├─ transcription      each piece in isolation: a failure skips that piece
     └─ typing             each text in isolation; the virtual keyboard is recreated
vtt session run      supervisor: restarts a crashed session (transient child),
 └─ vtt session worker      which continues the same transcript at the right time
```

- Restart intensity like OTP: more than 5 crashes in 60 s (sessions: 3 in 5 min) and the
  supervisor gives up with a notification instead of looping; back-off 1, 2, 4 ... 30 s.
- A settings change is a clean, requested restart (exit code 75), not a crash.
- `vtt stop` / tray Quit / Ctrl+C stop the child first and give it time to finish (a
  session finishes its last piece and the notes).
- Crashes are logged in `~/.local/state/vtt/vtt.log`; the first one shows a notification.
- A session crash can lose the piece that was being recorded (up to ~30 s); the transcript
  marks the spot.

## Windows port (planned)

The code is split so a Windows build replaces only the OS-specific parts:

| Module | Linux now | Windows |
|---|---|---|
| `supervisor.rs` | child processes + SIGTERM | child processes + `TerminateProcess` / job object |
| `platform.rs` | XDG paths, `.desktop` autostart, lock file, `setsid` | `%APPDATA%`, `HKCU\…\Run` key, named mutex, `DETACHED_PROCESS` |
| `hotkey.rs` | evdev (`/dev/input`) | `SetWindowsHookEx(WH_KEYBOARD_LL)`, no permissions needed |
| `typer.rs` + `layout.rs` | uinput + xkbcommon key mapping | `SendInput` with `KEYEVENTF_UNICODE`: any character, any layout, no mapping needed |
| `audio.rs` | cpal (ALSA) | cpal (WASAPI), unchanged |
| `tray.rs` | StatusNotifierItem (ksni, D-Bus) | `tray-icon` crate |
| `mics.rs` | sound-server source list + per-source recording (PulseAudio protocol) | cpal/WASAPI devices; system audio via WASAPI loopback |
| `setup.rs`, `config.rs`, `providers.rs`, `stt.rs`, `usage.rs` | shared | shared |

For clients, Windows needs none of the permission steps above. A single `vtt.exe` plus
`vtt setup` (or later a small settings window and tray icon) is the whole installation.
