#!/bin/bash
# Builds the Linux server and packs dist/ReSkateServer-linux-x64.tar.gz (copied to builds/).
set -euo pipefail
cd "$(dirname "$0")"
TARGET="${CARGO_TARGET_DIR:-target}"
# Link against an old glibc so the server runs on older distros and in panel containers, not just
# on the build machine. Needs zig and cargo-zigbuild (docs/building.md).
GLIBC=2.31
TRIPLE=x86_64-unknown-linux-gnu
if ! command -v cargo-zigbuild >/dev/null || ! command -v zig >/dev/null; then
    echo "package-linux.sh needs zig and cargo-zigbuild, see docs/building.md" >&2
    exit 1
fi
cargo zigbuild --release --target "$TRIPLE.$GLIBC"
BIN="$TARGET/$TRIPLE/release/ReSkateServer"
NEWEST=$(objdump -T "$BIN" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1)
if [ "$(printf '%s\n%s\n' "$NEWEST" "$GLIBC" | sort -V | tail -1)" != "$GLIBC" ]; then
    echo "The binary needs glibc $NEWEST, newer than $GLIBC" >&2
    exit 1
fi
echo "Needs glibc $NEWEST or newer"
NAME=ReSkateServer-linux-x64
OUT=dist/$NAME
rm -rf dist
mkdir -p "$OUT/licenses" builds
cp "$BIN" "$OUT/"
cp dist-assets/libsteam_api.so dist-assets/world-layers.json "$OUT/"
cp README.md "$OUT/README.md"
cp -r docs "$OUT/docs"
mkdir -p "$OUT/examples"
cp -r examples/plugins "$OUT/examples/plugins"
cp LICENSE "$OUT/LICENSE.txt"
cp dist-assets/zstd-LICENSE.txt "$OUT/licenses/"
cargo tree -e normal --prefix none --format "{p} {l}" | sed 's/ (\*)//' | sort -u > "$OUT/licenses/rust-crates.txt"
cp pterodactyl/egg-reskate.json dist/
chmod +x "$OUT/ReSkateServer"
tar -czf "dist/$NAME.tar.gz" -C dist "$NAME"
cp "dist/$NAME.tar.gz" builds/
echo "Packed dist/$NAME.tar.gz (copied to builds/)"
