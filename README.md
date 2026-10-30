# deerpunk

Modular game engine: authoritative server simulation, client-side rendering, MethodDB API for game libraries.

## Documentation

Start here: **[docs/README.md](docs/README.md)**

| Guide | Topic |
|-------|--------|
| [docs/api.md](docs/api.md) | MethodDB (`v1:*`) — methods, source files, scopes |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Host, client/server, frame loop |
| [docs/network.md](docs/network.md) | Packets, replication, user messages |
| [docs/assets.md](docs/assets.md) | gamedir, materials, meshes |
| [docs/renderer.md](docs/renderer.md) | Renderer stages, culling, mesh memory |

## Building a game

1. Create a **game library** (`cdylib`) with `server_init` / `server_frame` (and client exports if you ship a windowed client).
2. Link **`deerpunk_engine_api`** or the SDK crate — resolve engine services only through MethodDB names documented in [docs/api.md](docs/api.md).
3. Point the executable at your **`gamedir`** (configs, shaders, materials, datapacks).
4. Register console commands and cvars in init; drive simulation from `server_frame`.

## Repository layout

```text
crates/engine-host/      Host + MethodDB
crates/engine-client/    Window, extract, audio hook
crates/engine-server/    Server simulation glue
crates/engine-renderer/  wgpu render graph
crates/engine-api/       Safe API for game DLLs
crates/engine-network/   Wire protocol
```

Game projects typically live in a separate repository and depend on engine paths or published crates.

## Running

Build the `deerpunk` binary from this repo, set `gamedir` to your game content directory, and place the compiled game library where the host expects (`dlls/` or paths in your game’s launcher config).

See your game project for exact launch arguments and library names.
