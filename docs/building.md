# Building from source

## Requirements

- Rust 1.80 or newer (`rustup` recommended)
- A C compiler (`gcc` or `clang`) – `zstd-safe` builds zstd and `mlua` builds Lua 5.4 from source
- Linux x86_64
- For release packages: [Zig](https://ziglang.org/download/) and
  [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) (`cargo install cargo-zigbuild`)

## Build

```bash
cargo build --release
```

The binary is `target/release/ReSkateServer`. It needs `libsteam_api.so` (in `dist-assets/`) next to
it at runtime.

## Test

```bash
cargo test --release
```

Covers the wire codec and compression, protocol messages, password proofs, game rules and the
plugin system (using the plugins in `examples/plugins/`).

## Package

```bash
./package-linux.sh
```

Builds the release binary and writes `dist/ReSkateServer-linux-x64/` and
`dist/ReSkateServer-linux-x64.tar.gz` (binary, `libsteam_api.so`, `world-layers.json`, docs and
licenses) plus `dist/egg-reskate.json`.

### Why zig? (glibc compatibility)

A plain `cargo build` links against the glibc of the build machine, so a binary built on a
rolling-release distro (Arch: glibc 2.44) refuses to start on Debian 12 or in Pterodactyl's
`yolks:debian` image (`version 'GLIBC_2.44' not found`). `package-linux.sh` therefore builds with

```bash
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.31
```

which links against glibc **2.31** (Debian 11, Ubuntu 20.04) no matter where it is built, and then
checks with `objdump` that the binary needs nothing newer. The result is in
`target/x86_64-unknown-linux-gnu/release/ReSkateServer`.

## Releases and updates (automatic)

- **Every pull request** runs `.github/workflows/ci.yml`: the tests, and the package built exactly as
  a release builds it (glibc 2.31), which must start and name the version in `Cargo.toml`.
- **Every push to `main`** runs `.github/workflows/release.yml`. When the version in `Cargo.toml`
  has no release yet, it runs the tests, builds the package and publishes the release `v<version>`
  with `ReSkateServer-linux-x64.tar.gz` and `egg-reskate.json`. Its text is
  `release-notes/v<version>.md` (GitHub's generated notes when there is none). Running servers with
  `auto_update` install it once they are empty.
- **New ReSkate releases**: a scheduled Claude Code routine watches
  [Dingo-Shenanigans/ReSkate](https://github.com/Dingo-Shenanigans/ReSkate). When a new release
  is out, it ports the server-side changes (network protocol, server, config), adds tests, sets the
  version to the ReSkate version, writes the release notes and opens a pull request; once CI is
  green it merges it, which publishes the release as above.

So a release is: change the version (and `release-notes/v<version>.md`) in a PR, merge it.

## Release profile

`Cargo.toml` builds with `lto = true`, `codegen-units = 1`, `opt-level = 3` and strips symbols.
`panic = "unwind"` is required: errors inside a frame become panics that the main loop catches and
logs, as the C++ server catches exceptions.

## Code layout

| File | |
|---|---|
| `main.rs` | Entry point, command line, signal handling, main loop, console input |
| `host.rs` | Session host: players, messages, commands, settings, admin logic |
| `protocol.rs` | ReSkate network protocol (message types, encoding, limits) |
| `wire.rs` | Packet framing, LZ4/zstd compression |
| `buffers.rs` | Byte buffer reader/writer helpers |
| `steam.rs` | Steamworks game server and networking via `libsteam_api.so` (loaded with `libloading`) |
| `config.rs` | `ReSkateServer.json` load/save, custom map discovery |
| `party.rs` | Parties and party chat |
| `throwdown.rs` | Throwdown state relay |
| `objects.rs` | Placed object sync |
| `world.rs` | Maps, park lots, world layers |
| `speed.rs` | Speed check |
| `activity.rs` | Activity log |
| `password.rs` | Password proof (PBKDF2/HMAC-SHA256) |
| `words.rs`, `bad_words.txt` | Chat word filter |
| `text.rs` | Text helpers |
| `plugins.rs` | Lua plugin system (sandbox, API, commands, timers, events) |
| `update.rs` | GitHub release check |
| `tests.rs` | Unit tests |

## Pterodactyl egg

The egg lives in `pterodactyl/egg-reskate.json`. When editing it, keep the scripts with `\r\n` line
endings as Pterodactyl exports them, and re-import it in the panel.
