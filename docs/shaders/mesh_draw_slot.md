# Mesh draw slot (`instance_index`) for batching

Static mesh shaders must read per-draw data from **`mesh_uniform_slots[@builtin(instance_index)]`**, not slot `0`.  
Required for **`r_mesh_batch_mdi 1`** (multi-draw indirect). **`r_mesh_batch 1`** alone uses bind-batch with the same rule.

Reference implementation: **ohmyblock** `gamedir/shaders/inc/mesh_uniform_slot.wgsl` (included from `voxel.wgsl`, `voxel_fluid.wgsl`, …).

## WGSL (vertex)

```wgsl
@group(0) @binding(0) var<storage, read> mesh_uniform_slots: array<MeshUniform>;

fn mesh_mu_slot(slot: u32) -> MeshUniform {
    return mesh_uniform_slots[slot];
}

@vertex
fn vs_main(in: VsIn, @builtin(instance_index) draw_slot: u32) -> VsOut {
    let mu = mesh_mu_slot(draw_slot);
    let world_pos = mu.model * vec4(in.position, 1.0);
    // mu.view_proj, mu.material_data, mu.camera_pos …
}
```

Pass **`draw_slot`** to the fragment stage (`@location`, flat) if the FS reads `material_data`.

## Chunk world origin

- Vertices stay in **chunk-local** space.
- **`mu.model`** is the chunk transform (set on extract) — do **not** assume ECS transform is identity for batching.

## Voxel shaders (`gamedir/shaders/voxel.wgsl`, `voxel_fluid.wgsl`)

1. Add `mesh_mu_slot` if missing.
2. Change `vs_main` to take `@builtin(instance_index) draw_slot: u32`.
3. Replace any `mesh_uniform_slots[0]` / fixed slot / model-only-at-origin with `mesh_mu_slot(draw_slot)`.

## Cvars

| Cvar | Default | Meaning |
|------|---------|---------|
| `r_mesh_batch` | 1 | Share pipeline/texture binds inside batch groups |
| `r_mesh_batch_mdi` | 0 | Use `multi_draw_indexed_indirect` (lower DIP) when WGSL supports draw slot |

After shader fix: `r_mesh_batch_mdi 1` — check HUD `indirect_batches` / `indirect_commands` and lower DIP.
