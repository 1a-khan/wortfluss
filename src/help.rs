//! `vtt help` and `vtt help <topic>`: the built-in guide. Model tables are generated from
//! providers.rs, so they always match what `vtt setup` offers.

use crate::providers;
use anyhow::{bail, Result};

const OVERVIEW: &str = "\
Wortfluss (vtt) - hold a key, speak, let go: the text is typed wherever your cursor is.

GETTING STARTED
    vtt setup        answer a few questions (service, key, model, language, hotkey ...)
    That's it: vtt then runs in the background and starts at every login.
    Default: hold Right Ctrl, speak, release.

WITHOUT THE TERMINAL
    Tray icon (top bar): right-click for sessions and the common settings.
    Ctrl+Alt+S  session (what the computer plays)    Ctrl+Alt+M  call session
    Ctrl+Alt+K  correction on/off                    Ctrl+Alt+H  hold <-> toggle
    Ctrl+Alt+V  all settings                         Ctrl+Alt+R  rewrite selected text
    change them: vtt set shortcuts

REWRITE SELECTED TEXT (like the writing assistant on phones)
    Select text in any app, press Ctrl+Alt+R. A small window shows the corrected text right
    away; pick a tone with the mouse or 1-6: Korrigieren, Professionell, Zwanglos, Höflich,
    #social, Emojis hinzufügen. Edit the suggestion if you like, then Enter (Ersetzen) types
    it over the selection; Copy puts it on the clipboard, Esc cancels. Uses the text model of
    `vtt set correction` (about $0.0001 per suggestion). Labels are German on German systems.

COMMANDS
    vtt setup        menu of all settings with current values; change one at a time
    vtt set <what>   change one thing directly: service, model, key, correction, phrases,
                     language, mic, hotkey, mode, sounds, indicator, layout, autostart,
                     session, ui-language, shortcuts, advanced
    vtt phrases      list names/words to spell right; vtt phrases add \"A, B\" / remove \"A\"
    vtt check        test everything vtt needs and show how to fix problems
    vtt start        start in the background        vtt stop    stop it
    vtt              run in this terminal and show what is typed (good for testing)
    vtt usage        audio sent today / this month / all time, with cost
    vtt retry        transcribe pieces that failed earlier (e.g. no internet)
    vtt update       install a new version now (automatic every 6 h; auto_update = false: off)
    vtt mics         list the microphones; choose one with `vtt set mic`
    vtt session start [title] [--source system|mic|both]   transcribe a lecture/meeting
    vtt session stop  stop it and write notes        vtt session status
    vtt session redo FILE   transcribe again from kept audio (--keep-audio)
    vtt session delete-audio [FILE]                   delete kept session audio
    vtt notes FILE [lecture|meeting|summary]          (re)write notes for a transcript
    vtt speakers FILE  give \"Speaker 1\", \"Speaker 2\" ... real names
    vtt autostart on|off      start at login or not
    vtt test-type    type a test sentence after 3 seconds
    vtt test-mic     record 4 seconds to vtt-test.wav
    vtt help <topic> more help on a topic
    vtt --version

HELP TOPICS
    vtt help models       services and recommended models, with prices
    vtt help correction   automatic spelling & grammar correction
    vtt help phrases      names and words that must be spelled right
    vtt help switching    changing service or model
    vtt help hotkey       choosing the dictation key, hold vs toggle
    vtt help mic          choosing the microphone
    vtt help session      transcribing lectures, videos and meetings + notes
    vtt help layout       keyboard layouts and special characters
    vtt help security     how API keys are stored and protected
    vtt help costs        what you pay and how to check it
    vtt help files        where settings, keys and logs are
    vtt help trouble      something doesn't work
    vtt help windows      Windows: installer, start menu, differences
";

const WINDOWS: &str = "\
WORTFLUSS ON WINDOWS

Install: run Wortfluss-Setup.exe (no administrator rights needed; installs for your user
in %LOCALAPPDATA%\\Programs\\Wortfluss). It offers to start at login and to add the `vtt`
command, then opens the setup: service, API key, dictation key, microphone.

Start menu:  Wortfluss            starts it (icon in the notification area, bottom right;
                                  click ^ if it's hidden, drag it out to keep it visible)
             Wortfluss settings   the same as `vtt setup`
Everything else works as on Linux: the tray menu, Ctrl+Alt shortcuts, `vtt set ...`,
sessions, notes, speaker separation.

Differences to Linux:
- No permission setup: the keyboard hook and typing work for every user.
- Typing sends each character as itself, so every keyboard layout and every script
  (ü, ş, ß, اردو, emoji) works; `vtt set layout` has nothing to do.
- Apps running as administrator can't receive typed text from a normal program
  (Windows security). Run Wortfluss as administrator too only if you really need that.
- \"What the computer plays\" is recorded from the current default output (WASAPI
  loopback). Long silences aren't recorded at all, so times in the transcript can run a
  little ahead after a pause.
- Settings: %APPDATA%\\Wortfluss (config.toml, secrets.env); the master key for the API
  keys is in the Windows Credential Manager. Logs, usage, failed pieces:
  %LOCALAPPDATA%\\Wortfluss.
- The installer isn't code-signed yet, so Windows SmartScreen may warn on first run
  (More info > Run anyway).
";

const CORRECTION: &str = "\
SPELLING & GRAMMAR CORRECTION                                   (off by default)

Without it, vtt types exactly what the speech model heard. With it, a small text model
of the same service fixes spelling, capital letters, commas and grammar first, without
translating or rewording.

    Turn on/off:   vtt set correction
    Extra time:    about 0.3-1 s per dictation
    Extra cost:    a fraction of a cent per dictation (100-300 tokens)

Safeguards: it never answers a dictated question; if the result looks wrong (empty, much
longer or shorter) or the service fails, vtt types the original text. When you run `vtt`
in a terminal you see both versions (\"heard:\" and the corrected one).

Keep it off when dictating code or terminal commands: it may add capitals or a full stop.
";

const HOTKEY: &str = "\
DICTATION KEY

Change it with `vtt set hotkey`: you just press the key you want.

    Good keys:   Right Ctrl (default), Menu, Pause, Scroll Lock, Insert, F9-F12
    Avoid:       letters/numbers/space (lost for typing), Shift (every capital letter
                 would record), Right Alt on German/Turkish layouts (it's AltGr: @ EUR { })

Hold mode (default): hold the key while speaking, let go to type. Using the key in a
shortcut (Ctrl+C, Ctrl+V ...) cancels silently, so Right Ctrl keeps working as Ctrl.

Toggle mode: press once to start, again to stop. Good for long dictations.
Switch with `vtt set mode`, which also sets the longest recording (1 min to 1 hour).

LONG DICTATIONS: while you speak, vtt sends the recording in pieces of about 30 s,
cut at a natural pause, and types each piece's text while you keep talking. In hold
mode the text is typed once you release the key (a held Ctrl would turn letters into
shortcuts). Piece size: `vtt set advanced`.

SILENCE: pieces without speech (pauses, breathing, fan or street noise, keyboard
clicks) are never sent, and pauses longer than 1 s inside a piece are shortened, so
you don't pay for silence. Toggle mode stops by itself after 60 s without speech.

If the internet drops, unsent pieces are saved; `vtt retry` transcribes them later.

Beeps: high beep = recording, low beep = sent (`vtt set sounds`).
While listening, a small sound wave appears next to the text cursor; grey dots = turning
speech into text (`vtt set indicator`; on GNOME after logging out and in once).
";

const LAYOUT: &str = "\
KEYBOARD LAYOUT

vtt types by pressing keys on a virtual keyboard, so it must know your layout. With
\"automatic\" (default) it follows the layout the desktop uses right now, including
switching (e.g. German <-> Turkish with Super+Space). On non-GNOME desktops it uses the
system default from /etc/default/keyboard. You can also set a fixed layout in setup.

Letters your layout has no key for (e.g. Urdu script on a German keyboard, emoji) are
entered with Ctrl+Shift+U unicode input. That works in GTK/GNOME apps, not everywhere.

Wrong characters (e.g. z/y swapped, '?' becomes '_')? The layout is wrong:
run `vtt check` to see what vtt detected, and fix it with `vtt set layout`.
";

const SECURITY: &str = "\
API KEY SECURITY

API keys are never written to config.toml. `vtt setup` stores them encrypted
(ChaCha20-Poly1305) in secrets.env. The master key is random, created once, and kept in
your login keyring (GNOME Keyring / KWallet; Credential Manager on Windows), which is
encrypted and unlocked when you log in.

    Protected:      a leaked secrets.env (git, backup, cloud folder, other computer)
                    can't be decrypted.
    Not protected:  someone logged in as you on this computer; they can open the
                    keyring like vtt does. No local program can prevent that.

No keyring (headless systems): a weaker key derived from this machine is used instead;
`vtt check` tells you which one is active.

Keyring reset or new computer: run `vtt set key` and enter the key again.
Environment variables (GROQ_API_KEY, OPENAI_API_KEY, OPENROUTER_API_KEY, MISTRAL_API_KEY)
override the stored key and may contain a plain key or an enc: value from secrets.env.
";

const PHRASES: &str = "\
PHRASES: NAMES AND WORDS TO SPELL RIGHT

    vtt phrases                      show the list
    vtt phrases add \"Ammad, Groq\"    add (comma separated)
    vtt phrases remove \"Groq\"        remove
    vtt phrases clear                remove all
    vtt set phrases                  the same, with menus

Use it for your name, company, product names, greetings: anything the speech model
tends to misspell (e.g. \"Grog\" instead of \"Groq\").

Groq, OpenAI and custom servers use the phrases while transcribing. OpenRouter and
Mistral ignore them there, so for those turn on spelling correction (`vtt set
correction`), which then applies the phrases. With correction on, they are applied on
every service.
";

const SWITCHING: &str = "\
CHANGING SERVICE OR MODEL

    vtt set service    pick another service. Its recommended model is chosen
                       automatically (or your earlier choice for that service), and the
                       API key is only asked for if that service has none yet. Each
                       service keeps its own key, so switching back needs no typing.
    vtt set model      keep the recommended model, pick from everything the service
                       offers right now (type to filter), or type a model name.
    vtt set key        replace the API key of the current service.

A running vtt switches within a few seconds; no restart needed.
See `vtt help models` for the recommended models and prices.
";

const MIC: &str = "\
MICROPHONE

    vtt mics        list the microphones the system knows (with readable names)
    vtt set mic     choose one; vtt records 3 s so you can see it hears you

Default: the system default microphone (Settings > Sound), so plugging in a headset
and selecting it there also switches vtt. Choose a specific microphone to always use
it, e.g. a USB or Bluetooth headset. If that microphone is not connected, the default
one is used, and `vtt check` tells you.

Silent recordings: check the microphone isn't muted (`vtt mics` shows [muted]) and
try `vtt test-mic`.
";

const SESSION: &str = "\
SESSION MODE: LECTURES, VIDEOS, MEETINGS

Instead of typing, vtt writes everything that is said into a Markdown file, live, and
adds notes at the end.

    vtt session start \"Physics 3\"              what the computer plays (default)
    vtt session start --source mic              a lecture in the room
    vtt session start \"Team call\" --source both  calls: lines labelled You / Others
    vtt session stop                            stop, then the notes are written
    vtt session status                          is one running? which file?
    vtt notes FILE meeting                      write other notes for a transcript

WHO SAID WHAT (speaker separation, `vtt set speakers` or --speakers): the lines are
labelled \"Speaker 1\", \"Speaker 2\" ... and the same voice keeps its name for the whole
session. In a call (--source both) \"You\" stays You and the other side is separated.
Afterwards give them names; the notes change too:

    vtt speakers FILE                           asks for a name for each speaker
    vtt speakers FILE \"Speaker 1=Anna\" \"Speaker 2=Ben\"

Needs OpenRouter (ElevenLabs Scribe v2, ~$0.28/h), OpenAI or Mistral; Groq can't tell
voices apart. Works best with clear turns; people talking at the same time, very short
answers (\"ja\") and similar voices can be assigned to the wrong speaker.

Options: --notes lecture|meeting|summary|none, --speakers, --dir FOLDER, --keep-audio. Defaults,
a session key (press to start, press again to stop) and the folder: `vtt set session`.
Also from the tray icon menu, or Ctrl+Alt+S (session) / Ctrl+Alt+M (call).

KEEP AUDIO (off by default): the audio is saved next to the transcript (hidden .audio
folder). After the session a notification asks: Delete audio / Keep for now. Kept audio
is deleted automatically after 7 days. `vtt session redo FILE` transcribes it again,
e.g. after changing the model or language.

Files: ~/Documents/vtt/2026-11-14 10-00 Title.md. Notes at the top, transcript with
[hh:mm:ss] times below. Only text is kept, never audio. Pauses and silence are skipped;
a session stops by itself after 30 minutes without speech.

Notes use the text model of `vtt set correction` (correction itself can stay off).
Cost for 1 hour: ~$0.01 transcript (OpenRouter Whisper Turbo) + ~$0.002 notes.

\"both\" with loudspeakers: the microphone also hears the other side, so text can
appear twice. Use headphones; vtt reminds you when no headphones are connected.

CONSENT: only record other people (meetings, calls, lectures) with their permission.
In Germany recording non-public speech without consent is a criminal offence
(§ 201 StGB), and many universities don't allow recording lectures without asking.
";

const COSTS: &str = "\
COSTS

Speech services charge per length of audio, not per word. A short dictation costs about
$0.0001 with the recommended Groq model; Groq's free plan costs nothing until you hit its
rate limits (you then get a \"rate limit\" notification; wait a bit).

    vtt usage    requests, minutes and estimated cost: today, this month, all time.
                 OpenRouter reports the exact cost of each request; for the others vtt
                 estimates from list prices. Groq counts each request as at least 10 s.

Very short presses (< 0.4 s) and pieces without speech are never sent. Silence at the
start and end is cut off and long pauses inside are shortened before sending. Official numbers are in your provider's dashboard.
See `vtt help models` for prices per model.
";

const TROUBLE: &str = "\
TROUBLESHOOTING                         first step, always:  vtt check

Nothing happens when I hold the key
    - Is vtt running?  `vtt check` (last line) or `vtt start`
    - \"no keyboard could be opened\": permission setup missing, see below
Wrong characters are typed            ->  vtt help layout
\"API key rejected\" / \"HTTP 401\"     ->  vtt set key
\"rate limit\" / \"HTTP 429\"           ->  free plan limit; wait, or upgrade the plan
\"not enough credit\" / \"HTTP 402\"    ->  top up (OpenRouter), or vtt set service
\"cannot decrypt\"                    ->  keyring reset or new computer: vtt set key
Words are often wrong
    - set your language: vtt set language
    - add names/brands: vtt phrases add \"Name, Brand\"
    - turn on correction: vtt set correction
    - try another model: vtt set model   (vtt help models)
Text appears twice or letters go missing in one app
    - raise the typing delay: vtt set advanced
Silence or no microphone       ->  vtt test-mic, then check Settings > Sound > Input

One-time permission setup (Linux), then log out and back in:
    sudo usermod -aG input \"$USER\"
    echo 'KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\", OPTIONS+=\"static_node=uinput\"' \\
      | sudo tee /etc/udev/rules.d/80-vtt-uinput.rules
    sudo udevadm control --reload-rules && sudo udevadm trigger

Background log (no dictated text in it):  ~/.local/state/vtt/vtt.log
";

fn files() -> String {
    let p = |r: anyhow::Result<std::path::PathBuf>| {
        r.map(|p| p.display().to_string()).unwrap_or_else(|_| "?".into())
    };
    format!(
        "\
FILES

    settings (no API keys)      {}
    API keys, encrypted         {}
    usage log (no text)         {}
    background log (no text)    {}
    start at login              ~/.config/autostart/vtt.desktop

config.toml has comments explaining every setting; a running vtt applies changes to it
within a few seconds. Dictated text is never written to disk.
",
        p(crate::config::path()),
        p(crate::secrets::path()),
        p(crate::platform::data_dir().map(|d| d.join("usage.tsv"))),
        p(crate::platform::log_file()),
    )
}

fn correction() -> String {
    let mut out = CORRECTION.to_string();
    out.push_str("\nCorrection models (first = recommended; setup can list all text models):\n");
    for p in providers::PROVIDERS {
        for (i, m) in providers::chat_models(p.id).iter().enumerate() {
            let service = if i == 0 { p.name } else { "" };
            out.push_str(&format!("    {service:<12} {:<30} {}\n", m.id, m.note));
        }
    }
    out
}

fn models() -> String {
    let mut out = String::from(
        "SERVICES AND MODELS          (first model of each service = recommended default)\n",
    );
    for p in providers::PROVIDERS {
        out.push_str(&format!("\n{} - {}\n", p.name, p.blurb));
        if !p.key_url.is_empty() {
            out.push_str(&format!("    get a key: {}\n", p.key_url));
        }
        for m in p.models {
            out.push_str(&format!(
                "    {:<34} ~${:<6.3}/hour  {}\n",
                m.id, m.usd_per_hour, m.note
            ));
        }
        if p.id == "openrouter" {
            out.push_str("    ... plus 25+ more: setup shows the full live list (type to filter)\n");
        }
        if p.id == "custom" {
            out.push_str("    any OpenAI-compatible server, e.g. a self-hosted Whisper; models come from the server\n");
        }
    }
    out.push_str(
        "\nPrices are list prices (Nov 2026) and may change. In `vtt setup` you can keep the\n\
         recommended model, pick from everything the service currently offers, or type a name.\n\
         Language: \"automatic\" is best if you switch languages; a fixed one is a bit faster.\n",
    );
    out
}

pub fn show(topic: Option<&str>) -> Result<()> {
    let text = match topic.map(|t| t.trim().to_lowercase()) {
        None => OVERVIEW.to_string(),
        Some(t) => match t.as_str() {
            "models" | "model" | "services" | "service" | "providers" => models(),
            "correction" | "correct" | "spelling" | "grammar" => correction(),
            "hotkey" | "key" | "keys" | "mode" => HOTKEY.into(),
            "layout" | "keyboard" => LAYOUT.into(),
            "mic" | "mics" | "microphone" | "input" => MIC.into(),
            "session" | "sessions" | "lecture" | "meeting" | "notes" => SESSION.into(),
            "security" | "secrets" | "api-key" | "apikey" | "encryption" => SECURITY.into(),
            "phrases" | "phrase" | "words" | "names" => PHRASES.into(),
            "switching" | "switch" | "change" | "set" => SWITCHING.into(),
            "costs" | "cost" | "price" | "prices" | "billing" | "usage" => COSTS.into(),
            "files" | "file" | "config" => files(),
            "trouble" | "troubleshooting" | "problems" | "faq" => TROUBLE.into(),
            "windows" | "win" | "installer" | "install" => WINDOWS.into(),
            other => bail!("no help topic `{other}`. Topics: models, correction, phrases, switching, hotkey, mic, session, layout, security, costs, files, trouble, windows"),
        },
    };
    print!("{text}");
    if topic.is_none() {
        println!("\n{}\n{}", crate::credits::line(), crate::credits::links().trim_end());
    }
    Ok(())
}
