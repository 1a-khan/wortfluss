# Installing and using Wortfluss

*Deutsch: [INSTALL.de.md](INSTALL.de.md)*

Wortfluss turns your voice into text in any app: hold a key, speak, let go, and the text is
typed where your cursor is. It can also rewrite selected text and turn lectures or meetings
into notes.

**Download:** always from the official page
**[github.com/1a-khan/wortfluss/releases/latest](https://github.com/1a-khan/wortfluss/releases/latest)**
(section *Assets*). After that, Wortfluss updates itself.

## 1. Get an API key (5 minutes, once)

Wortfluss sends your speech to a speech-recognition service of your choice, using **your own
key**, so you only pay for what you use (often nothing).

**Recommended: Groq** (free plan is enough for normal dictation):

1. Go to [console.groq.com](https://console.groq.com) and sign up (Google account or email).
2. Open **API Keys** → **Create API Key**, give it a name such as "Wortfluss".
3. Copy the key (starts with `gsk_`). You will paste it in step 3.

Alternatives: OpenRouter, OpenAI or Mistral; the setup explains each one.
Keep the key private: don't send it by email or post it anywhere.

## 2. Install

### Windows 10 / 11

1. Download **`Wortfluss-Setup-<version>.exe`** from the
   [releases page](https://github.com/1a-khan/wortfluss/releases/latest).
2. Double-click it. Windows may show *"Windows protected your PC"* (the installer is not
   code-signed yet): click **More info** → **Run anyway**.
3. Click through the installer. No administrator rights are needed. Leave
   **"Start Wortfluss when I log in"** ticked.
4. At the end, leave **"Set up Wortfluss now"** ticked and click **Finish**.

**Is it safe? (virus scanner warnings)** Wortfluss has to do things that virus scanners
also see in spy software: it watches for its dictation key, records the microphone, sends
the audio to the speech service you chose, puts text into other apps and updates itself.
So a scanner may warn about it, especially while the program is not code-signed yet.

- Check that the file is the real one: right-click `vtt.exe` → **Properties** →
  **Details** shows *Wortfluss*, *MIAK IT (Ammad Khan)* and the version. The
  `SHA256SUMS` file on the release page lists the checksum of every download.
- The complete source code is public:
  [github.com/1a-khan/wortfluss](https://github.com/1a-khan/wortfluss). Your audio and
  text go only to the service you set up; besides that, Wortfluss only asks GitHub for
  updates.
- If a scanner blocks it, please tell us (or the scanner's maker) so it can be cleared.
  Company PCs: your IT can allow it for the folder
  `%LOCALAPPDATA%\Programs\Wortfluss`.

### Linux (Ubuntu and other distributions, Wayland or X11)

Open a terminal and run these commands one by one:

```bash
# 1. Download and install the program
mkdir -p ~/.local/bin
curl -L -o ~/.local/bin/vtt https://github.com/1a-khan/wortfluss/releases/latest/download/vtt-linux-x86_64
chmod +x ~/.local/bin/vtt

# 2. Allow the dictation key and typing (asks for your password, once per computer)
sudo usermod -aG input "$USER"
echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' \
  | sudo tee /etc/udev/rules.d/80-vtt-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then **log out and log back in** (needed once for step 2), open a terminal again and run:

```bash
vtt setup
```

If `vtt` is not found, run `~/.local/bin/vtt setup` instead.
Step 2 lets Wortfluss see the dictation key and type for you. Any program in the `input`
group can read the keyboard, so use this on computers where you are the only user.

## 3. First setup

The setup (a window with questions) asks:

1. **Service:** choose *Groq* (or the one you have a key for).
2. **API key:** paste the key from step 1. It is checked at once and stored encrypted on
   your computer.
3. **Language you speak:** e.g. German, or *automatic* if you switch languages.
4. **Dictation key:** press the key you want to hold while speaking. Default: **Right Ctrl**.
5. **Microphone** and **start at login:** the defaults are fine.

Wortfluss then runs in the background. You will see its icon in the tray / notification area
(on Windows maybe under the small arrow ^ next to the clock).

**Linux with GNOME (e.g. Ubuntu):** Wortfluss installs a small GNOME extension for the
listening bubble and for pasting. When a message asks you to, **log out and back in once**
so GNOME loads it. Until then everything works, the text is just typed instead of pasted
and there is no bubble.

## 4. Daily use

| You want to... | Do this |
|---|---|
| **Dictate** | Click into any text field, **hold Right Ctrl**, speak, let go. The text appears after about a second. |
| Know it is listening | A short beep, and a small bubble with a moving sound wave next to the text cursor (in apps that don't report their cursor: at the bottom of the screen). Grey dots: the text is on its way. |
| **Improve selected text** | Select text, press **Ctrl+Alt+R**, choose a tone with 1–6 (correct, professional, casual, polite, social, emojis), **Enter** replaces it. To get it in another language, pick one under *Translate into* and press *Translate*. |
| **Transcribe a lecture, video or meeting** | **Ctrl+Alt+S** starts and stops; the transcript and notes land in *Documents/vtt* (tray → *Open transcripts folder*). Only record people with their consent. |
| Change settings | Tray icon → *All settings...* (or **Ctrl+Alt+V**), Start menu → *Wortfluss settings*, or `vtt setup`. |
| Check that everything works | `vtt check` (Windows: in a command window) |
| Help | Start menu → *Wortfluss help*, or `vtt help` |

Tips:

- Speak in whole sentences; punctuation is added automatically.
- Names and special words: tray → *All settings...* → *Phrases*, or `vtt phrases add "Name, Word"`.
- The interface follows your system language (English, Deutsch, Türkçe, Español, Polski,
  Italiano).
- The text is **pasted** (fast, any language and emoji). Your clipboard is used for a moment
  and then put back, so something you copied before is still there afterwards. (Linux: a
  copied image is replaced.) Prefer typing letter by letter? *All settings...* → *Put text in
  by pasting* → No.
- Turn the bubble off: *All settings...* → *Listening bubble at the text cursor*.

## 5. Updates

Nothing to do: Wortfluss looks for new versions every few hours and installs them in the
background when you are not dictating. Only versions signed by the publisher are accepted.
After the update a message says *"Wortfluss was updated to version X"*.
To check at once: tray icon → *Check for updates* (it tells you when it is searching and
downloading), or `vtt update`. Linux with GNOME: if an update asks you to log out and back in,
do so when convenient.

## 6. Uninstall

- **Windows:** Settings → Apps → *Wortfluss* → Uninstall (or Start menu → *Uninstall Wortfluss*).
  Your settings and transcripts stay in `%APPDATA%\Wortfluss` and *Documents\vtt*; delete
  them by hand if you want.
- **Linux:** `vtt stop`, `vtt autostart off`, then delete `~/.local/bin/vtt`, and if you like
  `~/.config/vtt` (settings) and `~/Documents/vtt` (transcripts).

## Problems?

1. Run `vtt check`: it tests the key, microphone, keyboard access and autostart and says
   what to fix.
2. Nothing appears in a program that runs **as administrator** (Windows): Windows doesn't
   allow that; start that program normally.
3. Linux: the text is typed slowly instead of pasted, or there is no bubble: log out and back
   in once (GNOME loads the Wortfluss extension then). Text in a terminal shows `^V`: tell us
   which terminal program it is.
4. An app shows strange or doubled letters: make sure *Put text in by pasting* is on
   (`vtt set paste`).
5. Still stuck: [report a problem](https://github.com/1a-khan/wortfluss/issues/new/choose)
   (never post your API key), or contact [MIAK IT](https://miak-it.de).

---
Wortfluss is made by Ammad Khan, [MIAK IT](https://miak-it.de). Free and open source
(Apache License 2.0).
