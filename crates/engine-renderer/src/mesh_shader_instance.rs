//! Mesh pass shaders must index `mesh_uniform_slots` via `@builtin(instance_index)`
//! so multi-draw indirect batches share pipeline binds but keep per-chunk `model`.

/// True if WGSL looks like it uses per-draw uniform slots (required for `r_mesh_batch_mdi`).
pub fn mesh_shader_supports_draw_slot(wgsl: &str) -> bool {
    wgsl.contains("@builtin(instance_index)")
        && (wgsl.contains("mesh_uniform_slots") || wgsl.contains("mesh_mu_slot("))
}

#[cfg(test)]
mod tests {
    use super::mesh_shader_supports_draw_slot;

    #[test]
    fn detects_draw_slot_usage() {
        let ok = r#"
            @group(0) @binding(0) var<storage, read> mesh_uniform_slots: array<MeshUniform>;
            fn vs_main(@builtin(instance_index) draw_slot: u32) {}
        "#;
        assert!(mesh_shader_supports_draw_slot(ok));
        assert!(!mesh_shader_supports_draw_slot("fn vs_main() {}"));
    }
}
