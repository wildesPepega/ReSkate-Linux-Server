# Building from source

## Requirements

- Rust 1.80 or newer (`rustup` recommended)
- A C compiler (`gcc` or `clang`) – `zstd-safe` builds zstd and `mlua` builds Lua 5.4 from source
- Linux x86_64

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

### Portable builds (older glibc)

The binary links dynamically against the system glibc, so it runs on systems with the same or a
newer glibc than the build machine. To support older distros, build inside an old container:

```bash
docker run --rm -v "$PWD":/src -w /src rust:1-bullseye ./package-linux.sh
```

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
