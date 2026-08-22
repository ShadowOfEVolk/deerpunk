# engine_api — карта модулей и будущего API

Все интерфейсы пока пустые (stub-методы). Каждый раздел описывает,
какие методы нужно будет перенести из движка при полной миграции.

---

## version
Константа версии движка.

| Иtem | Описание |
|------|----------|
| `ENGINE_VERSION: &str` | `env!("CARGO_PKG_VERSION")` — строка версии |

---

## app
Жизненный цикл приложения, кадровый цикл.

| Метод | Источник | Описание |
|-------|----------|----------|
| `frame_dt()` | `Engine` / `FrameScheduler` | Delta time текущего кадра |
| `frame_number()` | `FrameScheduler` | Номер кадра (u64) |
| `uptime_secs()` | `FrameScheduler` | Время работы в секундах |
| `request_quit()` | — | Запрос на выход из цикла |

**Будущее (cmd/cvar):**
- `execute_command(cmd)` → `CmdSystem::execute_command`
- `cvar_get(name)` → `CvarSystem::get_f32`
- `cvar_set(name, val)` → `CvarSystem::set`
- `exec_config(path)` → `CmdSystem::exec_config_file`

---

## renderer
Рендеринг: текстуры, HDR, тонмаппинг, карта.

| Метод | Источник | Описание |
|-------|----------|----------|
| `frame_width()` | `frame_width()` | Ширина окна |
| `frame_height()` | `frame_height()` | Высота окна |

**Будущее (gpu):**
- `upload_map_atlas(pixels, w, h)` → `Renderer::upload_map_atlas`
- `upload_mesh_texture(slot, pixels, w, h)` → `Renderer::upload_mesh_texture`
- `set_wireframe(on)` → `Renderer::set_wireframe`
- `set_exposure(v)` → `Renderer::set_exposure_override`
- `set_tonemap_mode(m)` → `Renderer::set_tonemap_mode`
- `auto_exposure_stats()` → `Renderer::auto_exposure_stats`

---

## audio
Процедурная музыка и звуки.

| Метод | Источник | Описание |
|-------|----------|----------|
| *(пусто)* | — | Будет: play/stop/gain |

**Будущее:**
- `set_music_intensity(v)` → APU intensity
- `play_sfx(kind)` → звуковые эффекты

---

## physics
Физика (rapier3d обёртка).

| Метод | Источник | Описание |
|-------|----------|----------|
| `gravity()` | `PhysicsWorld` | Гравитация [x,y,z] |

**Будущее (PhysicsWorld):**
- `create_body(e, x, y, z, yaw, &ColliderDesc)` → `PhysicsWorld::create_body`
- `destroy_body(e)` → `PhysicsWorld::destroy_body`
- `body_position(e)` → `PhysicsWorld::body_position`
- `body_yaw(e)` → `PhysicsWorld::body_yaw`
- `apply_impulse(e, vx, vy)` → `PhysicsWorld::apply_impulse`
- `ray_cast(origin, dir, max, mask)` → `PhysicsWorld::ray_cast_closest`
- `step(dt)` → `PhysicsWorld::step`
- `update_vehicles(dt)` → `PhysicsWorld::update_vehicles`
- `create_vehicle(e)` → `PhysicsWorld::create_vehicle`
- `set_vehicle_drive(e, speed)` → `PhysicsWorld::set_vehicle_drive`
- `set_vehicle_brake(e, brake)` → `PhysicsWorld::set_vehicle_brake`
- `set_vehicle_steering(e, angle)` → `PhysicsWorld::set_vehicle_steering`

---

## world
ECS-мир: создание/уничтожение сущностей, компоненты, запросы, системы.

### Registration
| Метод | Описание |
|-------|----------|
| `components_register(type_name)` | Регистрация типа компонента по имени |
| `systems_register(name)` | Регистрация именованной системы |

### Entity lifecycle
| Метод | Описание |
|-------|----------|
| `spawn() -> u64` | Создать пустую сущность |
| `destroy(entity)` | Уничтожить сущность |
| `spawn_prefab(path) -> Vec<u64>` | Заспавнить из префаб-файла |

### Component access
| Метод | Описание |
|-------|----------|
| `components_add(entity, type_name, data)` | Добавить компонент (raw bytes) |
| `components_get(entity, type_name) -> Option<Vec<u8>>` | Прочитать компонент (копия) |
| `components_get_mut(entity, type_name) -> Option<&mut [u8]>` | Mutable доступ к компоненту |
| `components_remove(entity, type_name)` | Удалить компонент |

### Queries
| Метод | Описание |
|-------|----------|
| `query(require, exclude) -> Vec<u64>` | Сущности с require и без exclude компонентов |
| `query_count(require, exclude) -> usize` | Количество совпадений |
| `query_matches(entity, require, exclude) -> bool` | Проверка конкретной сущности |

---

## cmd
Консольные команды.

| Метод | Источник | Описание |
|-------|----------|----------|
| `execute(cmd)` | `CmdSystem::execute_command` | Выполнить строку команд |

**Будущее:**
- `register(name, callback)` → `CmdSystem::register_command`
- `exec_config(path)` → `CmdSystem::exec_config_file`

---

## cvar
Система переменных конфигурации.

| Метод | Источник | Описание |
|-------|----------|----------|
| `get_f32(name)` | `CvarSystem::get_f32` | Получить float значение |
| `set(name, val)` | `CvarSystem::set` | Установить значение |

**Будущее:**
- `get_string(name)` → `CvarSystem::get_string`
- `get_bool(name)` → `CvarSystem::get_bool`
- `get_usize(name)` → `CvarSystem::get_usize`
- `get_f32_or(name, default)` → `CvarSystem::get_f32_or`
- `set_or_create(name, val)` → `CvarSystem::set_or_create`
- `dump()` → `CvarSystem::dump`

---

## resources
Загрузка файлов, мешей, шейдеров.

| Метод | Источник | Описание |
|-------|----------|----------|
| `load_bytes(path)` | `FileSystem::read_file` | Загрузить байты |
| `load_text(path)` | `FileSystem::load_file` | Загрузить текст |

**Будущее:**
- `load_mesh(uri)` → `ResourceManager::load::<Mesh>`
- `file_exists(path)` → `FileSystem::file_exists`

---

## Стратегия миграции

1. ✅ Создать crate с пустыми трейтами (сейчас)
2. Добавить по 1-2 метода в каждый модуль
3. На стороне движка: создать `EngineApiImpl` реализующий все трейты
4. В `game::init(api: &dyn EngineApi)` принимать API
5. По одному заменять прямые вызовы engine internals на вызовы через API
6. Когда все методы мигрированы — убрать pub из engine internals
