//! `v1:materials:*` — host implementation, colocated with the material system.
//!
//! NOT in engine_methoddb.rs trampoline — methods live in their system module.

use deerpunk_engine_macro as methoddb;

use deerpunk_engine_common::material::{MaterialAsset, MaterialHandle, PropertyValueSer};
use deerpunk_engine_common::MeshTexture;

/// Load a material into the client `MaterialStore`.
///
/// `path` may be a gamedir-relative `.mat` file or a resource URI
/// (e.g. `file://maps/foo.bsp#material` from the gamelib BSP loader).
fn materials_load(host: &mut crate::Host, path: &str) -> u32 {
    if !path.contains('#') && !path.contains("://") {
        if let Some(ce) = host.client_engine.as_ref() {
            if let Some(h) = ce.material_handle_for_path(path) {
                return h.0;
            }
        }
    }

    let Some(ce) = host.client_engine.as_mut() else {
        log::error!("[materials] load '{path}': no client engine");
        return 0;
    };

    if path.contains('#') || path.contains("://") {
        let handle = host.resources.load::<MaterialAsset>(path);
        let asset = match handle.try_get() {
            Some(a) => a,
            None => {
                log::error!("[materials] load '{path}': resource not ready");
                return 0;
            }
        };
        match ce.load_material_from_asset(path, &asset, &host.resources) {
            Ok(h) => {
                log::info!("[materials] loaded uri '{path}' -> handle {}", h.0);
                h.0
            }
            Err(e) => {
                log::error!("[materials] load uri '{path}': {e}");
                0
            }
        }
    } else {
        let resolved = host.fs.resolve_path(path);
        let source = match std::fs::read_to_string(&resolved) {
            Ok(s) => s,
            Err(e) => {
                log::error!("[materials] load '{path}': {e}");
                return 0;
            }
        };
        match ce.load_material_resolved(path, &source, &host.resources) {
            Ok(handle) => {
                log::info!("[materials] loaded '{path}' -> handle {}", handle.0);
                handle.0
            }
            Err(e) => {
                log::error!("[materials] load '{path}': {e}");
                0
            }
        }
    }
}

/// Create a material at runtime.
/// `shader_path` — e.g. "file://shaders/mesh.wgsl"
/// `defines` — comma-joined preprocessor defines
/// `props_json` — JSON-encoded Vec<(String, PropertyValueSer)>
///
/// Texture properties may use the `"slot:N"` convention to reference a
/// pre-uploaded texture slot (e.g. the BSP albedo atlas uploaded via
/// `renderer.upload_texture_raw`). Any other path resolves to white.
fn materials_create(
    host: &mut crate::Host,
    shader_path: &str,
    defines: &str,
    props_json: &[u8],
) -> u32 {
    let props: Vec<(String, PropertyValueSer)> = match serde_json::from_slice(props_json) {
        Ok(p) => p,
        Err(e) => {
            log::error!("[materials] create: props JSON parse: {e}");
            return 0;
        }
    };

    let Some(ce) = host.client_engine.as_mut() else {
        log::error!("[materials] create: no client engine");
        return 0;
    };

    let shader_defines: Vec<String> = if defines.is_empty() {
        Vec::new()
    } else {
        defines.split(',').map(str::to_string).collect()
    };

    let asset = MaterialAsset {
        shader: shader_path.to_string(),
        shader_defines,
        properties: props,
    };

    let debug_path = format!("create:{}#{}", shader_path, asset.properties.len());
    match ce.load_material_from_asset(&debug_path, &asset, &host.resources) {
        Ok(handle) => {
            log::info!("[materials] created '{shader_path}' -> handle {}", handle.0);
            handle.0
        }
        Err(e) => {
            log::error!("[materials] create '{shader_path}': {e}");
            0
        }
    }
}

/// Hot-reload a material file (dev).
fn materials_reload(host: &mut crate::Host, path: &str) -> i32 {
    log::info!("[materials] reload '{path}'");
    // TODO: invalidate MaterialStore generation for this path
    0
}

/// Update a texture property on an existing material.
/// `handle` — MaterialHandle from `load`/`create`.
/// `prop_name` — "_BaseMap", "_NormalMap", or "_ORMMap".
/// `slot` — texture slot from `renderer.upload_texture_raw` / `flush_pending_albedos`.
/// Returns 0 on success, -1 on error.
fn materials_set_texture(host: &mut crate::Host, handle: u32, prop_name: &str, slot: u32) -> i32 {
    let Some(ce) = host.client_engine.as_mut() else {
        log::error!("[materials] set_texture: no client engine");
        return -1;
    };
    if ce
        .material_store
        .set_texture(MaterialHandle(handle), prop_name, MeshTexture(slot))
    {
        log::info!(
            "[materials] set_texture handle={handle} {prop_name} -> slot={slot}"
        );
        0
    } else {
        log::error!(
            "[materials] set_texture: bad handle={handle} or prop='{prop_name}'"
        );
        -1
    }
}

// ── methoddb bindings (colocated, not in engine_methoddb.rs) ──────────

#[methoddb::namespace(name = "v1:materials", on_host, scope = ["server", "client"])]
impl crate::Host {
    /// Load a `.mat` asset by path → MaterialHandle (0 = failed).
    #[methoddb::bind(name = "load")]
    pub fn api_materials_load(&mut self, path: &str) -> u32 {
        materials_load(self, path)
    }

    /// Create material at runtime: shader_path + defines + props_json → MaterialHandle.
    #[methoddb::bind(name = "create")]
    pub fn api_materials_create(
        &mut self,
        shader_path: &str,
        defines: &str,
        props_json: &[u8],
    ) -> u32 {
        materials_create(self, shader_path, defines, props_json)
    }

    /// Hot-reload material file → 0 on success, -1 on error.
    #[methoddb::bind(name = "reload")]
    pub fn api_materials_reload(&mut self, path: &str) -> i32 {
        materials_reload(self, path)
    }

    /// Update a texture property on an existing material → 0 ok, -1 error.
    #[methoddb::bind(name = "set_texture")]
    pub fn api_materials_set_texture(
        &mut self,
        handle: u32,
        prop_name: &str,
        slot: u32,
    ) -> i32 {
        materials_set_texture(self, handle, prop_name, slot)
    }
}