# Asset Manager (`ResourceManager`)

Runtime asset registry for deerpunk. Lives in `engine/common/resource`, owned by `Host`.

Inspectable via the inspector collection `assets` (`assets.list` / `assets.get`).

## Goals

- Load by URI (`file://…`, `wad://…`, `ui://…`, …).
- Stable **`AssetId`** per loaded asset.
- Pluggable **loaders** selected by explicit id, file extension, or URI protocol.
- **Aliases**: extra keys that resolve to the same `AssetId`.
- Canonical URI = **physical source** (where bytes actually came from).

## Core types

| Type | Role |
|------|------|
| `ResourceManager` | Registry: loaders, key→id map, asset records |
| `AssetId` | Monotonic `u64` id; stable for the process lifetime |
| `ResourceHandle<T>` | Typed handle: `id()`, `try_get()`, `unwrap()` |
| `LoadOpts` | Optional loader id + optional alias on load |
| `Loader` | Trait: `id()`, `extensions()`, `load(path, query)` |
| `LoadedAsset` | `data` + `size` + `type_name` from a loader |
| `AssetInfo` | Snapshot row for inspector / debug |

Code: `engine/common/resource/{mod,loader,bsp_loader,wad_loader}.rs`.

## URIs

Format:

```text
protocol://path[?query]
```

- Missing protocol → treated as `file`.
- Path separators are normalized to `/` when building `file://` URIs (`path_to_file_uri`).
- Query is passed to the loader (e.g. mesh transform hints for `.obj` / `.fbx`).

Examples:

| URI | Meaning |
|-----|---------|
| `file://D:/game/maps/foo.bsp` | BSP30 map on disk |
| `file://models/box.obj` | Mesh relative to process / absolute path in URI |
| `wad://darkdemo.wad/water` | Texture `water` inside a specific WAD |
| `ui://widgets/hello.rml` | UI file under `{gamedir}/ui` |

### Canonical vs alias (textures)

GoldSrc maps reference textures **by short name only** (`water`). They do not record which WAD provided the pixels. Search order: embedded miptex in the BSP, then `wad` list from worldspawn, first match wins.

| Key | Role |
|-----|------|
| **Canonical URI** | Physical source — unique identity. Example: `wad://darkdemo.wad/water` |
| **Alias** | Logical lookup name for BSP / tools. Example: `tex://water` |

Rules:

- A BSP map asset (`file://…/map.bsp`) is **not** the parent or source of textures that live in WADs.
- Only textures **embedded** in the BSP file use a BSP-related physical URI (when that path is implemented).
- `alias` is an extra key → same `AssetId`; it does not change the canonical URI.
- There is no separate `label` option: if you need a different key, put it in the URI or register an alias.

## Loading API

```rust
// Extension / protocol picks the loader
let h = rm.load::<Mesh>("file://models/box.obj");

// Explicit loader + alias
let h = rm.load_with::<BspWorldMesh>(
    &path_to_file_uri(&bsp_path),
    LoadOpts::default()
        .loader("bsp")
        .alias("maps/foo"), // optional extra key
);
let id = h.id();
let data = h.try_get(); // None if failed
```

### Loader resolution order

1. `LoadOpts.loader` if set (e.g. `"bsp"`, `"file"`, `"wad"`).
2. Else file extension from the path (`bsp` → `BspLoader`, `obj`/`fbx`/`wgsl` → `FileLoader`, `wad` → `WadLoader`).
3. Else URI protocol (`file`, `wad`, `ui`, …).

### Dedup

Loading an URI (or alias) that already maps to an `AssetId` returns the existing handle. A new `LoadOpts.alias` on that call is bound to the same id.

## Loaders (built-in)

Registered by `register_default_loaders(rm, gamedir)` from `Host::initialize`:

| Loader id | Extensions | Produces (`type_name`) | Notes |
|-----------|------------|------------------------|-------|
| `file` | `obj`, `fbx`, `wgsl` | `mesh` / `shader` | Generic files; client loads all WGSL via this loader |
| `bsp` | `bsp` | `bsp` (`BspWorldMesh`) | Uses gamedir for WAD search during parse |
| `wad` | `wad` | `rgba` | Path form: `file.wad/texname` (also `wad://…`) |

Additional loaders (e.g. `ui`) may be registered by other crates via `register_loader`.

Custom loader:

```rust
impl Loader for MyLoader {
    fn id(&self) -> &'static str { "my" }
    fn extensions(&self) -> &'static [&'static str] { &["myext"] }
    fn load(&self, path: &str, query: &str) -> Result<LoadedAsset, String> { … }
}
rm.register_loader(Box::new(MyLoader));
```

## Host integration

- `Host` owns `resources: ResourceManager`.
- `Host::new` registers `FileLoader`, then `ClientEngine` loads shaders through the manager (`file://{SHADER_DIR}/*.wgsl`, `type_name = shader`).
- `initialize(gamedir)` calls `register_default_loaders` (bsp/wad/file).
- Scene `Worldspawn` loads maps through the manager:

  `file://{gamedir}/maps/{name}.bsp` with `LoadOpts::loader("bsp")`.

- Each frame, inspector syncs `resources.list_assets()` into the `assets` collection.

## Inspector

Collection id: `assets`.

List item fields:

- `index` — `AssetId.0`
- `name` — canonical URI
- `type` — loader `type_name`
- `size` — bytes (file or decoded payload estimate)
- `aliases` — extra keys
- `loaded` — successfully loaded vs failed/pending

## Planned / not done yet

- Register each resolved WAD (or embedded) texture as its own asset with canonical `wad://file.wad/name` and alias `tex://name` during BSP load.
- Content-hash dedup across identical textures from different WADs (optional).
- Async / streaming loads (`Pending` is reserved but loads are currently synchronous).

## Non-goals

- GPU resource tracking (texture arrays, mesh slabs) — that stays in the renderer.
- Treating the map URI as a virtual parent of WAD textures.
