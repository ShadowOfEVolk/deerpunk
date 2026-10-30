# engine-api module map

This crate wraps **MethodDB** (`v1:*` methods) with safe Rust types.

Full reference (every namespace, method, source file, scope):

**[../../docs/api.md](../../docs/api.md)**

Quick index of Rust modules → namespaces:

| Module | Namespace |
|--------|-----------|
| `lib` | `v1:engine` |
| `world` | `v1:world` |
| `client_world` | `v1:client_world` |
| `mesh` | `v1:mesh` |
| `bone_buffer` | `v1:bone_buffer` |
| `net` | `v1:net` |
| `audio` | `v1:audio` |
| `renderer` | `v1:renderer` |
| `materials` | `v1:materials` |
| `physics` | `v1:physics` |
| `input` | `v1:input` |
| `ui` | `v1:ui` |
| `cmd` | `v1:cmd` |
| `cvar` | `v1:cvar` |
| `log` | `v1:log` |
| `resources` | `v1:resources` |
| `host_access` | `v1:host` (client helpers) |

Game libraries should depend on `deerpunk_engine_api` or `deerpunk_engine_sdk`, not on host internals.
