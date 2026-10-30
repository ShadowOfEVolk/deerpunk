# deerpunk documentation

Documentation for game and tool authors using the engine.

| Document | Contents |
|----------|----------|
| [api.md](api.md) | **MethodDB** — every `v1:*` method, source file, and DLL scope |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Process model, host loop, client vs server, rendering |
| [network.md](network.md) | Wire protocol, replication, user messages, sounds |
| [assets.md](assets.md) | `gamedir`, materials, meshes, resources |
| [renderer.md](renderer.md) | GPU pipeline: extract, prepare, cull, mesh allocators, indirect draw |

Game code lives in a **game library** (`cdylib`). The engine loads it and exposes services only through **MethodDB** string names resolved at init time. Safe Rust wrappers live in `crates/engine-api`.
