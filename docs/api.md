# Engine API Reference

Game DLL получает API через MethodDB resolver (не фиксированный vtable).

## Bootstrap

Host загружает `server.dll` и вызывает:

| Export | Signature | Description |
|---|---|---|
| `server_init` | `(ServerInitData)` | Передаёт version, `ctx`, resolver |
| `server_frame` | `(dt: f32)` | Один кадр game logic |
| `server_shutdown` | `()` | Cleanup |

```c
typedef struct {
    uint32_t version;
    uint8_t *ctx;
    void *(*resolver)(const uint8_t *name, size_t len);
} ServerInitData;
```

Gamlib резолвит методы по строке (`v1:world:get` …) и кеширует fn ptr.
Safe-обёртка: `crates/engine-api`.

---

## Method names

Все callbacks: первый аргумент `*mut u8` (= `Host` ctx), если не указано иное.

### Engine

| Name | Notes |
|---|---|
| `v1:engine:frame` | `(ctx, dt) → i32` |
| `v1:engine:shutdown` | `(ctx)` |
| `v1:engine:with_world_mut` | `(ctx, callback, user_data, free_user_data)` |

### World

| Name | Notes |
|---|---|
| `v1:world:get` | `(ctx, entity, comp) → *mut c_void` — указатель в storage Flecs |
| `v1:world:set` | `(ctx, entity, comp, data, size)` |
| `v1:world:has` / `remove` / `spawn` / `destroy` | |
| `v1:world:spawn_prefab` | `(ctx, name, len, opts: *const SpawnPrefabOpts) → u64` — prefab + optional map opts |
| `v1:world:clear` | stub |
| `v1:world:spawn_bundle` | stub |
| `v1:world:query` / `query_count` / `query_matches` | stub |
| `v1:world:free_value` | no-op (`get` не аллоцирует) |
| `v1:world:component_init_raw` | `(ctx, name, len, size, align) → ComponentId` |
| `v1:world:query_prepare` | `(ctx, expr, len) → query_id` |
| `v1:world:query_execute` | `(ctx, query_id, callback, user_data)` |

### Schedule / Input / ImGui / Physics / Renderer

| Name |
|---|
| `v1:schedule:add_system` / `clear` / `run` |
| `v1:input:is_key_down` / `mouse_dx` / `mouse_dy` |
| `v1:imgui:begin_window` / `end_window` / `text` / `value_f32` / `value_vec3` / `separator` |
| `v1:physics:create_shape` / `destroy_shape` / `create_parts` / `destroy_parts` |
| `v1:physics:cast_capsule` | Rapier swept capsule |
| `v1:physics:trace_line` | BSP mesh ray (`TraceLineResult` out-param) |
| `v1:renderer:load_texture` |

---

## Types

| Type | Definition |
|---|---|
| `EntityId` / `ComponentId` | `u64` |
| `Resolver` | `extern "C" fn(*const u8, usize) → *const ()` |

Регистрация методов на стороне host: `inventory::submit! { EngineApiMethod { ... } }` → `MethodDB`.
