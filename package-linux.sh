#!/bin/bash
# Builds the Linux server and packs dist/ReSkateServer-linux-x64.tar.gz (copied to builds/).
set -euo pipefail
cd "$(dirname "$0")"
TARGET="${CARGO_TARGET_DIR:-target}"
cargo build --release
NAME=ReSkateServer-linux-x64
OUT=dist/$NAME
rm -rf dist
mkdir -p "$OUT/licenses" builds
cp "$TARGET/release/ReSkateServer" "$OUT/"
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
