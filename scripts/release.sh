#!/usr/bin/env bash
# Publish a Wortfluss release for automatic updates.
#
#   scripts/release.sh 0.3.0 [notes-file]
#
# 1. Cargo.toml must say this version, the work must be committed and pushed, and CI
#    must have passed for that commit (it builds Linux, Windows and the installer).
# 2. Tags the commit v0.3.0.
# 3. Downloads CI's binaries, signs latest.json with the release key
#    (~/.config/wortfluss-release/signing.key, never leaves this machine) and publishes
#    everything as a GitHub release of 1a-khan/wortfluss.
# Installed copies pick it up within 6 hours (or at once with `vtt update`).
set -euo pipefail

VERSION="${1:?usage: scripts/release.sh VERSION [notes-file]}"
NOTES="${2:-}"
SRC_REPO="1a-khan/wortfluss"
PUB_REPO="$SRC_REPO" # releases live next to the source (public repo)
KEY="${WORTFLUSS_RELEASE_KEY:-$HOME/.config/wortfluss-release/signing.key}"
TAG="v$VERSION"
cd "$(dirname "$0")/.."

die() { echo "release: $*" >&2; exit 1; }

grep -q "^version = \"$VERSION\"" Cargo.toml || die "Cargo.toml is not at version $VERSION"
[ -f "$KEY" ] || die "signing key $KEY missing (restore it from your backup)"
[ -z "$(git status --porcelain)" ] || die "uncommitted changes"
COMMIT=$(git rev-parse HEAD)
git fetch -q origin
git branch -r --contains "$COMMIT" | grep -q . || die "commit $COMMIT is not pushed"
[ "$(gh repo view "$PUB_REPO" --json visibility -q .visibility)" = "PUBLIC" ] \
    || die "$PUB_REPO is not public: installed copies could not download the update"
if gh release view "$TAG" --repo "$PUB_REPO" >/dev/null 2>&1; then
    die "$TAG is already published"
fi

RUN=$(gh run list --repo "$SRC_REPO" --commit "$COMMIT" --workflow CI --json databaseId,conclusion \
      -q '[.[] | select(.conclusion=="success")][0].databaseId')
[ -n "$RUN" ] || die "no successful CI run for $COMMIT yet"

# The signer must be built from the same source (it checks release-key.pub).
cargo build --release --locked -q
SIGNER=target/release/vtt

DIST=$(mktemp -d)
trap 'rm -rf "$DIST"' EXIT
gh run download "$RUN" --repo "$SRC_REPO" -n vtt-linux-x86_64 -D "$DIST/linux"
gh run download "$RUN" --repo "$SRC_REPO" -n wortfluss-windows-x86_64 -D "$DIST/windows"
OUT="$DIST/out"
mkdir -p "$OUT"
cp "$(find "$DIST/linux" -name vtt -type f)" "$OUT/vtt-linux-x86_64"
cp "$(find "$DIST/windows" -name vtt.exe -type f)" "$OUT/vtt.exe"
cp "$(find "$DIST/windows" -name wortfluss.exe -type f)" "$OUT/wortfluss.exe"
SETUP=$(find "$DIST/windows" -name 'Wortfluss-Setup-*.exe' -type f | head -1)
[ -n "$SETUP" ] || die "installer missing in CI artifacts"
cp "$SETUP" "$OUT/Wortfluss-Setup-$VERSION.exe"

"$SIGNER" _release-sign "$KEY" "$VERSION" \
    "https://github.com/$PUB_REPO/releases/download/$TAG" "$OUT" \
    "linux-x86_64:vtt=$OUT/vtt-linux-x86_64" \
    "windows-x86_64:vtt.exe=$OUT/vtt.exe" \
    "windows-x86_64:wortfluss.exe=$OUT/wortfluss.exe" >/dev/null
(cd "$OUT" && sha256sum vtt-linux-x86_64 vtt.exe wortfluss.exe "Wortfluss-Setup-$VERSION.exe" > SHA256SUMS)

git tag -a "$TAG" -m "Wortfluss $VERSION" "$COMMIT"
git push -q origin "$TAG"

if [ -n "$NOTES" ]; then NOTES_ARG=(--notes-file "$NOTES"); else NOTES_ARG=(--notes "Wortfluss $VERSION"); fi
gh release create "$TAG" --repo "$PUB_REPO" --title "Wortfluss $VERSION" "${NOTES_ARG[@]}" \
    "$OUT/Wortfluss-Setup-$VERSION.exe" "$OUT/vtt-linux-x86_64" "$OUT/vtt.exe" "$OUT/wortfluss.exe" \
    "$OUT/latest.json" "$OUT/latest.json.sig" "$OUT/SHA256SUMS"

echo "Published $TAG: https://github.com/$PUB_REPO/releases/tag/$TAG"
