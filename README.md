# deerpunk engine

Экспериментальный игровой движок на Rust. MethodDB resolver API с hot-reload DLL, Flecs ECS, Rapier3D физикой и wgpu рендером.

## Модули

```
deerpunk/
├── engine/
│   ├── common/      # Ядро: Flecs ECS (C FFI), физика, ресурсы, networking, IPC, cmd/cvar
│   ├── host/        # Vtable API, trampolines, ServerEngine, ImGui
│   ├── client/      # Камера, sky (Hosek-Wilkie)
│   ├── renderer/    # wgpu pipeline: sprites, meshes, sky, clouds, HDR tonemap, fonts
│   └── server/      # Server-side логика
├── crates/
│   ├── sdk-client/  # Client SDK
│   ├── sdk-common/  # Общие типы SDK
│   └── engine-api/  # Safe Rust обёртка MethodDB для game DLL
├── editor/          # Level editor (iced GUI, IPC viewport)
└── src/main.rs      # Engine binary: загружает game DLL через libloading
```

## Технологии

### Графика

| Технология | Версия | Роль |
|---|---|---|
| **wgpu** | 30 | GPU API. Render passes: sky → volumetric clouds → 3D sprites → mesh (depth) → debug → HUD |
| **tiny-skia** | 0.11 | CPU 2D-рендеринг (спрайты, текст, glyph atlas) |
| **winit** | 0.30 | Окно, event loop, ввод |
| **imgui** | 0.12 | Дебажный GUI (инспектор ECS, профилировщик) |
| **imgui-wgpu** | local fork | ImGui wgpu backend |

Шейдеры (WGSL): `sprite`, `mesh`, `sky`, `cloud`, `debug`, `luminance`, `tonemap`, `storm`.

HDR pipeline: luminance pass → CPU readback → auto-exposure → tonemap (ACES/Reinhard/Hable).

### Физика

| Технология | Версия | Роль |
|---|---|---|
| **Rapier3D** | 0.35 | Rigid body dynamics, collision detection, vehicle controller |
| **glam** | 0.33 | Математика (Vec3, Mat4, Quat). Версия привязана к rapier3d |

### ECS

| Технология | Версия | Роль |
|---|---|---|
| **Flecs** | 4.1.6 | Основной ECS. C-библиотека, компилируется через `cc`. Компоненты по имени строки (DLL-safe) |
| **hecs** | 0.11 | Re-export для обратной совместимости (game DLL queries) |

Flecs подключается через C FFI: `flecs_sys.rs` (bindings) + `flecs_world.rs` (safe wrapper) + `shim.c` (обёртки над inline-функциями). Один самодостаточный `.c` файл (~90K строк).

### Сеть и IPC

| Технология | Версия | Роль |
|---|---|---|
| **interprocess** | 2 | Named pipes — headless mode: движок → editor |
| **memmap2** | 0.9 | Shared memory frame buffer (mmap, zero-copy viewport) |
| **bincode** | 1.3 | Бинарная сериализация (события, replication) |

### Сериализация

| Технология | Версия | Роль |
|---|---|---|
| **serde** + **serde_json** | 1 | Конфиги, prefab'ы |
| **bincode** | 1.3 | События, сеть, данные |
| **handlebars** | 6 | Шаблоны конфигов |

### Инструменты

| Технология | Версия | Роль |
|---|---|---|
| **libloading** | 0.8 | Hot-reload загрузка game DLL |
| **iced** | 0.14 | GUI editor'а ( Elm-архитектура) |
| **cc** | 1 | Компиляция Flecs C-кода |
| **inventory** | 0.3 | Статическая регистрация компонентов |
| **once_cell** | 1.19 | Lazy statics (cvar, cmd, singleton'ы) |
| **tracy-client** | 0.17 | Frame profiling (feature-gated) |

## Архитектура

### MethodDB FFI

Host передаёт game DLL `ServerInitData { version, ctx, resolver }`.
DLL резолвит методы по имени (`v1:world:get` …) через `MethodDB` (inventory).
Safe-обёртка — `crates/engine-api`.

```
game DLL → resolver("v1:…") → trampolines → Host / ServerEngine
```

### Hot-reload DLL

Game server компилируется как `cdylib`. Движок загружает через `libloading`. Перекомпиляция DLL подменяет сервер на лету.

### Headless vs Windowed

Без feature `window`: рендер в shared memory (mmap) + named pipe IPC. С `window`: winit event loop + wgpu surface.

### Quake-style консоль

`cmd()` + `cvar()` системы. CLI: `+command arg`. TCP: `:9998`.

### Sub-crate dependencies

```
engine-api  → (C ABI / MethodDB, без линка на host)
sdk-client  → sdk-common
engine-client → engine-common
engine-renderer → engine-common + engine-client
editor → engine-common + iced
```

engine-common — лёгкий (только core типы). engine-host — тяжёлый (Flecs, physics, schedule). engine-renderer — GPU-only (wgpu, tiny-skia, imgui).
