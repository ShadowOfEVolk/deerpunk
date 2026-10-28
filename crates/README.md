# Workspace crates

Folders are prefixed so engine vs gamelib SDK are easy to tell apart.

## `engine-*` — runtime (host, simulation, render, FFI surface)

| Directory | Cargo package |
|-----------|----------------|
| `engine-api` | `deerpunk-engine-api` |
| `engine-client` | `deerpunk-engine-client` |
| `engine-client-fmod` | `deerpunk-engine-client-fmod` |
| `engine-common` | `deerpunk-engine-common` |
| `engine-network` | `deerpunk-engine-network` (wire / session / LAN; rep stays in common) |
| `engine-resource` | `deerpunk-engine-resource` (registry, mesh, `wad`; loaders in `src/loaders/`) |
| `engine-ffi` | `deerpunk-engine-ffi` |
| `engine-host` | `deerpunk-engine-host` |
| `engine-inspector-protocol` | `deerpunk-inspector-protocol` |
| `engine-macro` | `deerpunk-engine-macro` (MethodDB proc-macros) |
| `engine-renderer` | `deerpunk-engine-renderer` |
| `engine-server` | `deerpunk-engine-server` |
| `engine-ui` | `deerpunk-ui` |

## `sdk-*` — gamelib / mod SDK

| Directory | Cargo package | Rust crate name |
|-----------|----------------|-----------------|
| `sdk-common` | `deerpunk-sdk-common` | `deerpunk_engine_sdk` |
| `sdk-client` | `deerpunk-sdk-client` | (re-exports SDK) |
| `sdk-macro` | `deerpunk-sdk-macro` | `deerpunk_sdk_macro` |

Gamelib crates should depend on `deerpunk-sdk-common` (and `deerpunk-engine-ffi`), not on `engine-host` or `engine-common` directly.

**Next split (planned):** `engine-world` (Flecs `World` + `layout_meta`). BSP и прочие game loaders — `register_loader` на `ResourceManager` из gamelib или будущий `engine-resource` submodule.
