# Engine API Reference

C ABI vtable. Singleton — engine resolves ctx internally, callers never pass it.

---

## EngineApiVtable

Top-level vtable struct.

| Field | Signature | Description |
|---|---|---|
| `world` | `WorldApiVtable` | ECS operations sub-vtable |
| `schedule` | `ScheduleApiVtable` | Schedule operations sub-vtable |
| `initialize` | `(gamedir: *const u8, len: usize)` | Load configs, init map, spawn entities |
| `frame` | `(dt: f32) → i32` | Run one frame (systems + physics sync). Returns 0 on success |
| `shutdown` | `()` | Cleanup |
| `with_world_mut` | `(callback, user_data, free_user_data)` | Pass `&mut World` to `extern "C" fn(world, user_data)` |
| `with_physics` | `(callback, user_data)` | Pass `&PhysicsWorld` to `extern "C" fn(physics, user_data)` |

---

## WorldApiVtable

Type-erased ECS. Entity IDs and Component IDs are `u64`.

| Field | Signature | Description |
|---|---|---|
| `register_component` | `(name, name_len, has, get, set, remove) → u64` | Register component type with fn pointers. Returns `ComponentId` |
| `get` | `(entity, comp) → *mut c_void` | Get component as `Box<dyn Any>` ptr. Null if missing |
| `set` | `(entity, comp, data)` | Insert/overwrite component from `Box<dyn Any>` ptr |
| `has` | `(entity, comp) → bool` | Check if entity has component |
| `remove` | `(entity, comp)` | Remove component from entity |
| `spawn` | `() → u64` | Create empty entity |
| `spawn_bundle` | `(comp_ids: *const u64, comp_data: *const *mut c_void, count) → u64` | Create entity with components |
| `destroy` | `(entity)` | Despawn entity + all components |
| `clear` | `()` | Destroy all entities |
| `query` | `(require, require_n, exclude, exclude_n, out: *mut u64, max) → usize` | Query entities by component filter. Returns count |
| `query_count` | `(require, require_n, exclude, exclude_n) → usize` | Count matching entities |
| `query_matches` | `(entity, require, require_n, exclude, exclude_n) → bool` | Test entity against filter |
| `free_value` | `(ptr)` | Free a value returned by `get` |

### ComponentFns

Per-component function pointers passed to `register_component`:

| Fn | Signature | Description |
|---|---|---|
| `CompHasFn` | `(world, entity) → bool` | Check entity has component |
| `CompGetFn` | `(world, entity) → *mut c_void` | Clone into `Box<dyn Any>`, return raw ptr |
| `CompSetFn` | `(world, entity, data)` | Downcast and insert |
| `CompRemoveFn` | `(world, entity)` | Remove |

---

## ScheduleApiVtable

| Field | Signature | Description |
|---|---|---|
| `add_system` | `(system_fn: extern "C" fn(dt, user_data), user_data)` | Register system callback |
| `clear` | `()` | Remove all systems |
| `run` | `(dt)` | Execute all systems in order |

---

## Types

| Type | Definition | Description |
|---|---|---|
| `ExternSystemFn` | `extern "C" fn(dt: f32, user_data: *mut c_void)` | System callback |
| `WorldMutCallback` | `extern "C" fn(world: *mut c_void, user_data: *mut c_void)` | World access callback |
| `PhysicsCallback` | `extern "C" fn(physics: *mut c_void, user_data: *mut c_void)` | Physics access callback |
| `FreeUserDataFn` | `extern "C" fn(user_data: *mut c_void)` | Free callback for user_data |
| `CompHasFn` | `extern "C" fn(world, entity) → bool` | Component has fn |
| `CompGetFn` | `extern "C" fn(world, entity) → *mut c_void` | Component get fn |
| `CompSetFn` | `extern "C" fn(world, entity, data)` | Component set fn |
| `CompRemoveFn` | `extern "C" fn(world, entity)` | Component remove fn |

---

## Reentrant Safety

Functions called inside `schedule.run()` (from system callbacks) must NOT construct `&mut ServerEngine`. They access individual fields via raw pointer arithmetic:

- `field::ecs_world()` → `&mut World`
- `field::components()` → `&HashMap`
- `field::entities()` → `&mut Vec<u64>`
- `field::schedule()` → `&mut Schedule`
- `field::physics_world()` → `&mut PhysicsWorld`

Non-reentrant (outside `schedule.run()`): `initialize`, `shutdown`, `with_world_mut`, `with_physics`, `spawn`, `spawn_bundle`.
