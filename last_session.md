Session 2026-11-13 (2):
- Layout-aware typing via libxkbcommon; live layout detection from GNOME dconf (mru-sources).
- New: `vtt setup` wizard (Groq/OpenAI/Mistral/custom, key check via /models, model list,
  language, spelling hints, press-to-capture hotkey, mode, beeps, layout, autostart, advanced).
- New: `vtt check`, `vtt start`, `vtt stop`, single-instance lock, settings hot-reload (re-exec).
- Config format: per-provider sections; old flat Groq config migrates automatically.
- OS-specific code in platform.rs / hotkey.rs / typer.rs / layout.rs (see README "Windows port").
- Autostart enabled; vtt running in background.
- Added OpenRouter (live STT model list via ?output_modalities=transcription, exact cost from usage.cost).
- Bedrock: no STT (Voxtral audio unsupported); Amazon Transcribe would need SigV4 + streaming. Not built.
Next: user tests OpenRouter key; Windows port.
- Added optional spelling/grammar correction (correct_text, per-provider correction_model; Groq qwen/qwen3.8-27b tested 0.4s).
- API keys: encrypted in ~/.config/vtt/secrets.env (ChaCha20-Poly1305), master key in OS keyring (keyring crate), machine-id fallback. User's Groq key migrated.
- Added built-in guide: vtt help [models|correction|hotkey|layout|security|costs|files|trouble] (src/help.rs; model tables generated from providers.rs).
- Added settings menu (vtt setup on existing install), vtt set <setting>, vtt phrases add/remove/clear; phrases also fed to correction (OpenRouter/Mistral ignore prompt). User now on OpenRouter + Claude Haiku correction.
- Long dictation: pieces (~30 s, cut at pauses) sent while recording, typed in order; live 16 kHz conversion (memory fix); speech detector skips silent pieces, shortens pauses; toggle auto-stop after 60 s silence; failed pieces saved, vtt retry; max recording up to 1 h.
- Mic selection: vtt mics, vtt set mic (3 s level test), mic in setup step 7; lists PipeWire/Pulse sources via pulseaudio crate, records via PIPEWIRE_NODE/PULSE_SOURCE (verified with pw-dump links).
- Session mode (CHG-2026-012): vtt session start/stop/status/run, sources system/mic/both (You/Others) via pulse record streams, live Markdown transcript, notes (lecture/meeting/summary) + vtt notes, session hotkey, vtt set session. E2E tested with a silent PipeWire null sink + speech-dispatcher sample.
- CHG-2026-013: tray icon (ksni), Ctrl+Alt shortcuts (vtt set shortcuts), toggle mode on clean tap, kept session audio (--keep-audio, delete prompt, 7-day expiry, vtt session redo/delete-audio), headphone hint for call sessions. NOTE: tests must not use the live mic (a both-mode test captured a room conversation; deleted).
- CHG-2026-014: renamed product Wortfluss (repo 1a-khan/wortfluss, private), icon drawn in code, OTP-style supervision (vtt run -> vtt worker; vtt session run -> session worker with --continue), keyboard hotplug manager, per-item isolation, GitHub Actions CI.
