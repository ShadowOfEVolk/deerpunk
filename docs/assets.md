# Assets and content

All game content lives under **`gamedir`** — the directory passed to the host at startup. Paths in JSON and scripts are relative to this root unless prefixed with `file://`.

## Layout (typical)

```text
gamedir/
  configs/          autoexec.cfg, config.cfg
  shaders/          .wgsl sources
  materials/        .mat.json
  maps/             map files / prefab spawns
  prefabs/          entity prefab JSON
  textures/         source art (also atlases)
  sound/            optional FMOD banks
  datapacks/        modular JSON + media packs
```

Datapacks add blocks, items, sound banks, particles, and UI without recompiling the engine.

---

## Resource pipeline

```mermaid
flowchart LR
    GD[gamedir files]
    RM[ResourceManager]
    LOAD[Typed loaders]
    GPU[Renderer / materials]

    GD --> RM
    RM --> LOAD
    LOAD --> GPU
```

| Loader | Types |
|--------|--------|
| File | Raw bytes, text |
| Shader | `.wgsl` via URI |
| Mesh | OBJ, internal mesh blobs |
| WAD | Legacy archive textures |
| Audio clip | Ogg/wav for Kira backend |

Register custom loaders with `v1:resources:register` when extending the engine.

---

## Materials

Materials are JSON descriptors (`materials/*.mat.json`) loaded through **`v1:materials:load`**.

| Field (typical) | Role |
|-----------------|------|
| Shader path | WGSL entry |
| Blend mode | Opaque, cutout, additive |
| Depth / cull | Pipeline state |
| Texture slots | Albedo, normal, lightmap |
| Params | Colors, factors exposed to shader |

**Color space:** use **linear RGB** for albedo tints and CPU-side color uploads unless the material marks the texture as sRGB. The renderer works in **HDR linear**; final display goes through tonemapping.

Methods:

- `v1:materials:load` — load from gamedir path  
- `v1:materials:create` — runtime material  
- `v1:materials:set_texture` — bind atlas or upload slot  
- `v1:materials:reload` — hot reload after shader edits  

---

## Meshes

| API | Use |
|-----|-----|
| `v1:mesh:create` | Upload vertex/index data from game code |
| `v1:mesh:create_with` | Tagged allocator for streaming chunks |
| ResourceManager mesh URI | Static assets from file |

Assign meshes on entities with **`Mesh3D`** (server world or **`client_world`** for local-only geometry).

Streaming worlds call host helpers:

- `v1:host:client_track_static`  
- `v1:host:client_apply_mesh_uri`  
- `v1:host:refresh_stream_meshes`  

---

## Textures and atlases

- **`v1:renderer:upload_texture_raw`** — direct RGBA upload.  
- **`v1:renderer:upload_lightmap_uri`** — baked lighting atlases.  
- **`v1:host:flush_pending_albedos`** — apply deferred atlas patches (client frame).

Block/voxel games often pack tiles into a single atlas; materials sample with atlas rect UVs in the vertex format.

---

## Shaders

Shaders live in `gamedir/shaders/`. The renderer loads them by URI; include files use `#include "inc/..."` relative to the shader directory.

After editing WGSL on disk:

- `v1:renderer:reload_shaders` — refresh pipeline cache  
- Or restart the client during development  

Pass order (world, sky, tonemap, UI) is defined in the client render graph — game code influences passes through materials and `v1:renderer:set_scene_lighting` / sky methods.

---

## Sound events (datapack)

Kira backend reads:

- `datapacks/*/sound_banks/*.json` — event name → clip list, volume, pitch  
- `datapacks/*/sound_events/*.json` — single-event files  

Play by logical name from **`v1:audio:play_2d`** / **`play_3d`**. FMOD backend uses bank files under `gamedir/sound` when configured instead.

---

## Prefabs and maps

Prefabs describe default components for spawned entities. The server spawns with **`v1:world:spawn_prefab`**. Clients receive **`SVC::SpawnPrefab`** with the same prefab id.

Maps reference prefab names and transforms; exact map format depends on your game layer.

---

## Related

- [api.md](api.md) — MethodDB for materials, mesh, renderer, audio  
- [ARCHITECTURE.md](ARCHITECTURE.md) — extract and render path  
- [network.md](network.md) — replicating entities that reference materials/meshes  
