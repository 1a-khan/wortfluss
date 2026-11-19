# Wortfluss: voice to text, anywhere

_Command: `vtt`._

Hold **Right Ctrl** (or a key you choose), speak, let go. The text appears in whatever app
has focus: browser, chat, editor, terminal. Works with Groq, OpenAI, Mistral or your own server.

One self-contained binary: records the mic, calls the speech service over HTTPS and pastes
the text (your clipboard is put back) or types it through its own virtual keyboard. No
ydotool, xdotool or clipboard tools. Wayland and X11.

**Download:** [latest release](https://github.com/1a-khan/wortfluss/releases/latest)
(Windows installer `Wortfluss-Setup-<version>.exe`, Linux `vtt-linux-x86_64`).
**Step-by-step guide for users:** [English](docs/INSTALL.md) · [Deutsch](docs/INSTALL.de.md).

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
| `vtt retry [FILE]` | transcribe pieces that failed earlier (e.g. no internet); fills the gaps in session transcripts |
| `vtt session start [title] [--source system\|mic\|both]` | transcribe a lecture, video or meeting into a Markdown file |
| `vtt session stop` / `status` | stop (notes are written) / show the running session |
| `vtt notes FILE [lecture\|meeting\|summary]` | (re)write notes for a transcript |
| `vtt help [topic]` | built-in guide: models, correction, hotkey, layout, security, costs, files, trouble |
| `vtt check` | tests everything (keyboard access, mic, key, model, autostart) and says what to fix |
| `vtt start` / `vtt stop` | start in the background / stop |
| `vtt` | run in this terminal (shows what it types; asks setup questions on first run) |
| `vtt usage` | audio sent today / this month / all time, with estimated cost, and time saved by dictating |
| `vtt typing-test` | measure your typing speed (used for "time saved"; default 40 words per minute) |
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
| Ctrl+Alt+R | rewrite the selected text (tone, correction) |

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

- **Who said what** (`vtt set speakers`, tray, or `--speakers`): lines are labelled
  *Speaker 1, Speaker 2 …*, consistently over the whole session; in calls *You* stays and
  the other side is separated. `vtt speakers FILE "Speaker 1=Anna"` puts in real names
  (transcript and notes). Needs a speaker-labelling model: OpenRouter `elevenlabs/scribe-v2`
  (~$0.28/h incl. overhead), OpenAI `gpt-4o-transcribe-diarize` or Mistral Voxtral. How
  names stay consistent: each piece is sent with ~3 s voice samples of the speakers known
  so far in front of it, and the service's label for each sample maps to the session name.
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

## Listening indicator

While Wortfluss listens, a small bubble with a live sound wave appears right next to the
text cursor, so you see at a glance that it is taking dictation (and that the mic hears
you). After you stop, the bars turn into grey moving dots until the text is in.

- Hold mode shows it after 250 ms, so shortcuts like Ctrl+C don't flash it.
- **Windows:** works in apps that report their text cursor (most classic Windows apps);
  elsewhere the bubble sits at the bottom center of the screen.
- **Linux (GNOME):** apps can't place windows next to another app's cursor on Wayland, so
  Wortfluss installs a tiny GNOME Shell extension (`wortfluss@mail.miak-it.com`) that draws the
  bubble (the same extension also pastes, see below). It finds the cursor through the
  accessibility interface (GTK apps, LibreOffice, Firefox...); elsewhere it uses the bottom
  center. **Log out and in once** after the first start, and after updates that change the
  extension, so GNOME loads it. Other desktops: no bubble (tray icon and beeps still work).
- Off: `vtt set indicator`. On GNOME the extension is removed only when pasting is off too.
- Preview without speaking: `vtt _indicator-demo`.

## Pasting or typing

By default the text is **pasted**: Wortfluss puts it on the clipboard, presses the paste
keys and then puts your clipboard back (after about half a second). That is fast, even for
long texts, and works with any keyboard layout and script (umlauts, Turkish, Urdu, emoji).

- **Windows:** clipboard and Ctrl+V. The text is kept out of the clipboard history (Win+V);
  your previous text or image is restored. Typing key by key garbled text in some apps
  (Windows 11 Notepad repeated letters), which pasting avoids.
- **Linux (GNOME):** on Wayland only the desktop itself may set the clipboard for another
  app, so the Wortfluss GNOME extension (the one that draws the bubble) does it and presses
  Ctrl+V, or **Ctrl+Shift+V in terminals** (GNOME Terminal, Ptyxis, Console, kitty,
  Alacritty, foot, WezTerm, Ghostty, Konsole, Tilix, ...). Only text can be put back on the
  clipboard; a copied image is replaced. Other desktops, or before the next login after an
  update: the text is typed as before.
- **Typing instead:** `vtt set paste` → off (or `paste = false` in config.toml). Then text
  is typed key by key through the virtual keyboard (Linux) or Unicode key events (Windows);
  `key_delay_ms` slows it down for apps that drop characters.
- Test without speaking: `vtt test-type` says *Done (pasted)* or *Done (typed)*.

## Rewrite selected text (writing assistant)

Like the writing assistant on Samsung/Galaxy phones, in every app: select text, press
**Ctrl+Alt+R** (or tray → *Rewrite selected text...*). A small window opens with the corrected
text already suggested. Pick a tone with the mouse or the keys 1–6:

| Key | Tone (German / English system) | What it does |
|---|---|---|
| 1 | Korrigieren / Correct | fixes spelling, grammar, punctuation; wording stays |
| 2 | Professionell / Professional | business tone, greeting and closing for messages |
| 3 | Zwanglos / Casual | relaxed, as between friends (German: du) |
| 4 | Höflich / Polite | softened requests, please and thank you; keeps du/Sie |
| 5 | #social | short social media post with hashtags |
| 6 | Emojis hinzufügen / Add emojis | same text with fitting emojis |

**Translate:** pick a language under *Übersetzen in / Translate into* (German, English, Turkish,
Spanish, French and 17 more) and press *Übersetzen*. It translates the suggestion you are
looking at (e.g. the Professional version, with your edits) by meaning, like a human translator,
not word by word. The tone stays: Professional becomes business Turkish with its usual greeting
and closing, Casual uses the informal address (sen), #social gets Turkish hashtags, and so on.
While a language is chosen, every tone (1–6) comes out in it; a new language in the list
switches right away, *Originalsprache / Original language* goes back. Enter replaces the
selection with the translation. The language is remembered for next time. Languages in other scripts
(Arabic, Urdu, Hindi, Chinese ...) are not offered yet: the window can't display them.

The suggestion can be edited. **Enter** (*Ersetzen*) puts it over the selection like dictated
text (pasted, or typed when pasting is off; typed line breaks are Shift+Enter, so chat apps
don't send early). On Windows it waits until the app with the selection is in front again;
if it is gone, the new text is left on the clipboard. *Kopieren* puts it on
the clipboard, **Esc** cancels. The text keeps its language and is never answered or acted on.
It uses the text model of `vtt set correction` (also when correction itself is off): about
$0.0001 per suggestion. How the selection is read: Linux uses the primary selection (anything
selected), Windows copies it with Ctrl+C and restores the clipboard afterwards. Up to 6000
characters.

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

## Dependency security

Every push runs [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) (policy in
`deny.toml`) against the RustSec advisory database: known vulnerabilities, unmaintained
crates, licenses and download sources. The same check runs every Monday for `main`,
Dependabot proposes updates, GitHub Actions are pinned to commit hashes, and each release
carries an SBOM (`sbom-*.cdx.json`). Details in [SECURITY.md](SECURITY.md).

## Build

```bash
sudo apt install build-essential pkg-config libasound2-dev libxkbcommon-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo build --release && cp target/release/vtt ~/.local/bin/
```

At runtime the binary only needs `libasound.so.2` and `libxkbcommon.so.0`, which every desktop
Linux has. Copy `vtt` to another machine and run `vtt setup` there.

## Limits

- When typing (pasting off, or not on GNOME), characters your layout has no key for (Urdu
  script, emoji) are typed with **Ctrl+Shift+U** Unicode input. That works in GTK/GNOME apps
  but not everywhere (some terminals). Pasting has no such limit.

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
  supervisor stops restarting, with a notification, instead of looping; back-off 1, 2, 4
  ... 30 s. Dictation then tries again after 5 minutes (the cause may pass).
- The child watches its supervisor too: if that is killed or crashes, the dictation worker
  hands over to a fresh `vtt run` (when idle) and a session carries on by itself, so an
  orphaned worker can never run alongside a new one and type twice.
- A settings change is a clean, requested restart (exit code 75), not a crash.
- `vtt stop` / tray Quit / Ctrl+C stop the child first and give it time to finish (a
  session finishes its last piece and the notes).
- Crashes are logged in `~/.local/state/vtt/vtt.log`; the first one shows a notification.
- A session crash can lose the piece that was being recorded (up to ~30 s); the transcript
  marks the spot.

**Session: no internet, service down.** Recording never depends on the network. A piece
that can't be transcribed gets a marker in the transcript (`_[not transcribed yet …]_`)
and its audio is kept; a notification says so once. While the service is unreachable, new
pieces are marked right away and one is tried every 5 s, growing to 60 s. As soon as one
works, every marker is replaced by its text, in place, and a notification says how many
were filled in. Parts still missing when the session ends: `vtt retry FILE` (or `vtt
retry` for all transcripts in the session folder).

**Session: sound device gone.** If the sound system restarts or a device disappears, the
recording is reopened every 2 s (a notification after 10 s, and again when it records);
the transcript times stay right.

## Interface languages

The tray menu, notifications and the rewrite window speak English, German, Turkish, Spanish,
Polish and Italian. They follow the system language; to choose one, `vtt set ui-language`
(or `ui_language = "tr"` in config.toml). Terminal setup and `vtt help` are in English. The
language you dictate in is a separate setting (`vtt set language`); dictation, correction and
rewriting keep the language you speak or write in any case.

## Automatic updates

Like Teams or Postman, Wortfluss updates itself in the background: 2 minutes after it starts and
then every 6 hours it looks for a new release, downloads it, checks it, and switches to it when it
is idle (never during a dictation, a session or an open rewrite window). After the restart the
new version says "Wortfluss was updated to version X". Tray → *Check for updates* (shows
"searching" and "version X found, downloading") or `vtt update` look right away;
`auto_update = false` in config.toml turns it off.

- **Where from:** the releases of this repository (`1a-khan/wortfluss`).
- **Safe:** each release has a `latest.json` (version, files, SHA-256) signed with the Wortfluss
  release key (Ed25519). The public half is built into the program (`release-key.pub`); an update
  is installed only if the signature, every file's hash and size match and the version is newer.
- **No admin rights:** the program lives in a folder the user owns (Windows
  `%LOCALAPPDATA%\Programs\Wortfluss`, Linux `~/.local/bin`). New files are written as
  `NAME.new`, then swapped in by renaming (a running Windows .exe is renamed to `NAME.old`, or
  `NAME.old-2`... if an older one is still in use, and deleted at the next start).

Publishing (maintainer): bump `version` in Cargo.toml, commit, push, wait for CI, then
`scripts/release.sh VERSION [notes.md]`. It tags the commit, signs the CI binaries with
`~/.config/wortfluss-release/signing.key`, publishes them and prints VirusTotal links for
each file. **Back up that key** (password
manager): without it no update can be published to existing installations.

## Windows

The same program builds for Windows; only the operating-system parts differ
(`src/linux/` and `src/windows/`, chosen at compile time):

| Part | Linux | Windows |
|---|---|---|
| hotkey & shortcuts | evdev (`/dev/input`), needs the `input` group | low-level keyboard hook, no permissions |
| putting text in | pasted by the GNOME extension; else uinput + xkbcommon key map per layout | pasted (clipboard + Ctrl+V); else `SendInput` with `KEYEVENTF_UNICODE` |
| microphone | sound-server sources (PulseAudio protocol) | cpal/WASAPI devices by name |
| system audio (sessions) | monitor of the default output | WASAPI loopback of the default output |
| tray | StatusNotifierItem (ksni) | notification area (`tray-icon`) |
| stop / background | SIGTERM, `setsid` | stop file, `DETACHED_PROCESS`, no console window |
| autostart | `~/.config/autostart/vtt.desktop` | `HKCU\…\CurrentVersion\Run` → `wortfluss.exe` |
| master key | GNOME Keyring / KWallet | Windows Credential Manager |
| settings | `~/.config/vtt` | `%APPDATA%\Wortfluss` |

Shared: setup, config, services, transcription, sessions, notes, speaker separation,
supervision, usage. Config files and key names (`KEY_RIGHTCTRL`) are the same on both.

**Installer** (built by CI on every push, artifact `wortfluss-windows-x86_64`):
`Wortfluss-Setup-<version>.exe`, per user, no admin rights. Start menu entries
*Wortfluss* (window-less launcher `wortfluss.exe`) and *Wortfluss settings* (`vtt setup`),
optional start at login and `vtt` on the PATH, then it opens the setup. Uninstalling stops
Wortfluss and removes autostart and PATH entries; settings and transcripts stay.

Windows limits: apps running as administrator can't receive pasted or typed text; the
installer is not code-signed yet (SmartScreen warns once). The .exe files carry version
information, icon and manifest (publisher MIAK IT), added by `build.rs`. Details: `vtt help windows`.

## License & credits

Wortfluss is created by **Ammad Khan** ([MIAK IT](https://miak-it.de)) and is open
source under the [Apache License 2.0](LICENSE): free to use, change and share, also
commercially. If you pass on Wortfluss or something built from it, include the
[NOTICE](NOTICE) file so the original author is credited. The name "Wortfluss" and its logo
are not part of the license; please give your own version its own name.

- Website: https://miak-it.de
- GitHub: https://github.com/1a-khan
- Source: https://github.com/1a-khan/wortfluss
