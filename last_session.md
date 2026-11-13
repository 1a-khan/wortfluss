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
