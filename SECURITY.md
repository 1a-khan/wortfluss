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

## Dependencies (supply chain)

Wortfluss is written in Rust and uses about 25 libraries directly (several hundred
including theirs). They are checked against the RustSec advisory database:

- **On every push and pull request** (`dependencies` job in CI) with
  [cargo-deny](https://github.com/EmbarkStudios/cargo-deny), policy in `deny.toml`:
  known vulnerabilities and unmaintained or unsound crates fail the build, licenses must
  be compatible with Apache-2.0, and crates may only come from crates.io. A release can
  only be published from a commit where this passed.
- **Every Monday** for `main` (`Weekly dependency audit`), because new advisories appear
  for code that hasn't changed.
- **Dependabot** proposes updates for crates and GitHub Actions as pull requests; CI
  tests them before anything is merged.
- GitHub Actions are pinned to commit hashes, and the workflow token can only read.
- Every release has an **SBOM** per platform (`sbom-*.cdx.json`, CycloneDX): every
  component and version inside the binaries, for your own scanners.

Accepted exceptions are listed in `deny.toml` with the reason. Currently: `zvariant`
(Linux only) is no longer maintained on its own because it was merged into zbus 6; it
goes away once the tray (`ksni`) and keyring libraries move to zbus 6.
