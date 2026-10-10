# Архитектура deerpunk

Закон слоёв движка. Новая фича должна попадать в один слой и один контракт.
Если непонятно куда — сначала во внутренний Rust API, в ABI gamelib выносить только по необходимости.

## Слои и crates

```
bin (deerpunk)          — точка входа, грузит DLL, крутит Host::run
engine/host             — Host: склейка систем, MethodDB, загрузка ServerDll
engine/server           — серверная логика движка (без окна/GPU)
engine/client           — окно, ввод, extract сцены → snapshot
engine/renderer         — только GPU: snapshot → кадр (wgpu)
engine/common           — ядро: ECS World, физика, ресурсы, IPC, типы
crates/engine-api       — единственный safe ABI для game DLL (без линка на host)
```

Правило зависимостей:

| Crate | Можно | Нельзя |
|---|---|---|
| `common` | math, flecs, rapier, serde | wgpu, window, imgui, host, client, renderer |
| `server` | common | client, renderer, host |
| `client` | common, renderer (snapshot API) | host internals, мутация мира вне extract |
| `renderer` | common types + snapshot | host, server, flecs World, game DLL |
| `host` | common, server, client (features) | прямой доступ из DLL к полям Host |
| `engine-api` | POD / ABI типы | host, common, flecs, wgpu |

Три контракта (не смешивать):

1. **ABI gamelib** — `ServerInitData`, строки `v1:*`, POD-компоненты (`crates/engine-api` + trampolines в `host/engine_api`).
2. **Внутренний Rust API** — `World`, `PhysicsWorld`, schedule (`engine/common`, `engine/host`).
3. **Render contract** — `RenderScene` / `RenderSnapshot` (`engine/renderer`). Renderer не знает про ECS.

---

## Host

`Host` (`engine/host`) — процессный оркестратор. Не геймплей.

Владеет:

- `ServerEngine` — ECS-мир + физика (серверная сторона)
- `ClientEngine` (feature `client`/`full`) — окно, ввод, рендер
- `CmdSystem` / `CvarSystem` / `FileSystem`
- `ResourceManager` — ассеты (см. [assets.md](assets.md))
- `ServerDll` — загруженная game library

Жизненный цикл:

1. `Host::new` — создать подсистемы
2. `initialize(gamedir)` — FS, cvar, cmd, `autoexec.cfg`, регистрация встроенных компонентов
3. `load_server_dll` — передать DLL `ctx` (= указатель на Host) и resolver MethodDB
4. `run` / `frame` — тик: DLL → симуляция мира → extract → render

DLL видит Host только как opaque `*mut u8` (`ctx`). Наружу не отдаём `&mut Host` и не шарим внутренности common напрямую.

---

## Client и Server

Как в духе Quake/Xash, но пока в одном процессе.

### Server (авторитетная симуляция)

- `ServerEngine` + game **ServerDll**
- Владеет сценой (Flecs `World`) и `PhysicsWorld`
- Тик: `server_frame(dt)` в DLL, затем `World::simulate`
- Сюда относится gameplay: сущности, правила, спавн, логика

### Client (представление)

- `ClientEngine`: окно (SDL/winit-слой в `App`), `Input`, `Renderer`, `RenderExtractor`
- Читает мир только через extract → `RenderScene` → `RenderSnapshot`
- Не содержит gameplay; не пишет правила игры в `renderer`

Пока single-process: сервер читает input через MethodDB — временный компромисс.
Когда появится сеть: input идёт client→server сообщениями; сервер не трогает клавиатуру.

```
Host::run loop
  → world.simulate
  → ServerDll.server_frame (game)
  → RenderExtractor.extract(World) → RenderScene
  → ClientEngine.tick → Renderer (GPU)
```

---

## ServerDll (gamelib)

Отдельная `cdylib` (`dlls/server.dll`) с игровым кодом. Движок не знает про конкретную игру.

Обязательные exports:

| Символ | Назначение |
|---|---|
| `server_init` | принимает `ServerInitData { version, ctx, resolver }` |
| `server_frame` | один кадр логики `(dt: f32)` |
| `server_shutdown` | cleanup |

Загрузка: `libloading` в `engine/host/server_dll.rs`.

Контракт для DLL:

- не линковать `engine-host` / `engine-common`
- ходить в движок только через `crates/engine-api` (resolve `v1:*` при init)
- компоненты через FFI — `#[repr(C)]` + без внутренних указателей (`Vec`/`String`/`Box` в layout запрещены)
- версия API (`SERVER_API_VERSION`) должна совпадать с host

Hot-reload — цель архитектуры; сейчас есть load, полный unload/reinit ещё не закон.

---

## MethodDB

Единственный канал host → gamelib. Не фиксированный гигантский vtable, а таблица имён → fn ptr.

Как устроено:

1. Host регистрирует trampolines через `inventory::submit! { EngineApiMethod { name: "v1:…", … } }`
2. При загрузке DLL собирается `MethodDB` (hashmap)
3. DLL получает `resolver(name) → *const ()`, резолвит нужные методы один раз, кеширует
4. Каждый вызов: `(ctx, …args)` → trampoline → поле Host / `ServerEngine`

Имена стабильны, с префиксом версии: `v1:world:get`, `v1:physics:spawn_dynamic_box`, …
Ломающие изменения — только через `v2:` (или явный выпил `v1:`).

Safe-обёртка для Rust-gamelib: `crates/engine-api`.

Запрещено:

- второй параллельный ABI (старый vtable, глобальный ENGINE_CTX, сырой `&mut World` в DLL)
- оставлять stub-методы в MethodDB «на потом» — либо реализовать, либо не экспортировать

---

## Основные системы

### Сцена (ECS World)

- Реализация: Flecs (`engine/common/world.rs`), обёртка `World`
- Живёт в `ServerEngine.world`
- Сущности + компоненты по имени (DLL-safe), queries (`query_prepare` / `query_execute` / batches)
- Встроенные компоненты регистрирует Host: `Transform`, `Mesh3D`, `Material`, `Camera`, …
- Игровые компоненты регистрирует DLL через `v1:world:component_init_raw`
- Симуляция кадра: `World::simulate(dt)` (Flecs progress + системы)

Сцена = источник истины для объектов. Рендер и физика — потребители/спутники, не второй «мир объектов».

### Физика

- Rapier3D, обёртка `PhysicsWorld` в `engine/common/physics`
- Живёт рядом со сценой в `ServerEngine`
- Связь entity ↔ rigid body через `body_map` / vehicle map
- Стегории коллизий (terrain, tank, projectile, …)
- Из gamelib — узкий MethodDB (`v1:physics:*`); остальное — внутренний API host/server

Тик физики должен оставаться на серверной стороне. Клиент физику не авторитетит.

### Рендер

Три шага, жёстко разделённые:

1. **ECS** — компоненты `Mesh3D`, `Material`, `Camera`, transform…
2. **Extract** (`engine/client/render_extractor.rs`) — queries → `RenderScene` (камеры, mesh draws, terrain…)
3. **GPU** (`engine/renderer`) — `RenderSnapshot` → passes (sky, meshes, HDR tonemap, …)

**UI** (`crates/ui` / RmlUi): document = UI unit. Client собирает `UiFrame` (геометрия), renderer рисует отдельным **ui pass** после tonemap (alpha blend). Не blit CPU-framebuffer.

Renderer не импортирует Flecs и не читает `World`. Любая новая отрисовка: сначала данные в snapshot-контракт, потом GPU-код.

Режимы вывода: window surface, offscreen, shared memory (editor viewport).

### Cmd (консоль команд)

Quake-style командный буфер в `engine/host/cmd.rs`.

- Регистрация именованных команд (callbacks)
- Буфер строк, исполнение по кадру / из конфига
- `exec_config_file` (например `configs/autoexec.cfg`)
- Связан с `CvarSystem` и `FileSystem` (указатели-refs внутри host)

Cmd — язык управления движком и отладки, не замена MethodDB для gameplay API.

### Cvar

Конфигурационные переменные в `engine/host/cvar.rs`.

- Статические дефолты через `inventory::submit! { CvarDef { … } }`
- Runtime map `name → string`, парсинг в f32/bool/usize
- Примеры: `gamedir`, `r_texturehandle`, `ipc_debug`

Cvar меняет поведение и настройки; не хранит состояние игры (здоровье, инвентарь и т.п. — это компоненты сцены).

---

## Куда класть новое

| Хочешь… | Куда |
|---|---|
| Тип/утилита без GPU и окна | `engine/common` |
| Геймплей / правила | ServerDll (+ при необходимости `v1:*`) |
| Серверную систему движка | `engine/server` или `ServerEngine` |
| Ввод, окно, extract | `engine/client` |
| Шейдер / GPU pass | `engine/renderer` + поле в snapshot |
| Вызов из game DLL | trampoline + `engine-api`, имя `v1:domain:name` |
| Консоль / конфиг | cmd / cvar на Host |
| Загрузка ассетов / лоадеры | `ResourceManager` + [assets.md](assets.md) |

Критерий: через месяц по этому файлу должно быть очевидно, в какой crate идёт изменение, и что gamelib по-прежнему ходит только через MethodDB.
