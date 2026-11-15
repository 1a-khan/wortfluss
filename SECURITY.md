# Security

Wortfluss reads keyboard input, records audio and installs signed updates, so security
reports are very welcome.

**Please report security problems privately:** on GitHub, *Security → Report a
vulnerability* (https://github.com/1a-khan/wortfluss/security/advisories/new). Don't open a
public issue for them. You'll get an answer as soon as possible; fixes are shipped through
the automatic updates.

Supported: the latest release (installed copies update themselves).

How updates are protected: every release's `latest.json` is signed with the Wortfluss
release key (Ed25519, public half in `release-key.pub`); installations accept only signed,
newer versions whose files match the signed SHA-256 hashes. See README → Automatic updates.
