//! `v1:materials:*` — host implementation, colocated with the material system.
//!
//! NOT in engine_methoddb.rs trampoline — methods live in their system module.

use deerpunk_engine_macro as methoddb;

use deerpunk_engine_common::material::{MaterialAsset, MaterialHandle, PropertyValueSer};
use deerpunk_engine_common::resource::AssetId;
use deerpunk_engine_common::MeshTexture;

/// Register a `.mat` / material URI in [`ResourceManager`] → [`AssetId`] (no GPU upload).
pub(crate) fn materials_load_asset(host: &mut crate::Host, path: &str) -> u64 {
    materials_load_asset_with_defines(host, path, "")
}

pub(crate) fn materials_load_asset_with_defines(
    host: &mut crate::Host,
    path: &str,
    defines: &str,
) -> u64 {
    if path.is_empty() {
        return 0;
    }
    let cache_key = if defines.is_empty() {
        path.to_string()
    } else {
        format!("{path}#{defines}")
    };
    if path.contains('#') || path.contains("://") {
        return host.resources.load::<MaterialAsset>(path).id().0;
    }
    let resolved = host.fs.resolve_path(path);
    let source = match std::fs::read_to_string(&resolved) {
        Ok(s) => s,
        Err(e) => {
            log::error!("[materials] load asset '{path}': {e}");
            return 0;
        }
    };
    let mut asset = match MaterialAsset::from_json(&source) {
        Ok(a) => a,
        Err(e) => {
            log::error!("[materials] parse '{path}': {e}");
            return 0;
        }
    };
    if !defines.is_empty() {
        for d in defines.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if !asset.shader_defines.iter().any(|x| x == d) {
                asset.shader_defines.push(d.to_string());
            }
        }
    }
    host.resources
        .register_loaded(&cache_key, asset, "material")
        .id()
        .0
}

/// `AssetId` → client [`MaterialStore`] handle (GPU row).
fn materials_resolve(host: &mut crate::Host, asset_id: u64) -> u32 {
    if asset_id == 0 {
        return 0;
    }
    let id = AssetId(asset_id);
    let Some(rh) = host.resources.handle::<MaterialAsset>(id) else {
        log::error!("[materials] resolve: unknown asset {asset_id}");
        return 0;
    };
    let Some(asset) = rh.try_get() else {
        log::error!("[materials] resolve: asset {asset_id} not loaded");
        return 0;
    };
    let path = host
        .resources
        .uri_for(id)
        .unwrap_or_else(|| format!("asset:{asset_id}"));
    let Some(ce) = host.client_engine.as_mut() else {
        log::error!("[materials] resolve '{path}': no client engine");
        return 0;
    };
    match ce.load_material_from_asset(&path, &asset, &host.resources) {
        Ok(h) => h.0,
        Err(e) => {
            log::error!("[materials] resolve '{path}': {e}");
            0
        }
    }
}

/// Legacy name: same as [`materials_load_asset`].
fn materials_load(host: &mut crate::Host, path: &str) -> u64 {
    materials_load_asset(host, path)
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
        log::info!("[materials] set_texture handle={handle} {prop_name} -> slot={slot}");
        0
    } else {
        log::error!("[materials] set_texture: bad handle={handle} or prop='{prop_name}'");
        -1
    }
}

// ── methoddb bindings (colocated, not in engine_methoddb.rs) ──────────

#[methoddb::namespace(name = "v1:materials", on_host, scope = ["server", "client"])]
impl crate::Host {
    /// Load a `.mat` asset by path → [`AssetId`] (0 = failed). GPU: host resolve before extract.
    #[methoddb::bind(name = "load")]
    pub fn api_materials_load(&mut self, path: &str) -> u64 {
        materials_load(self, path)
    }

    /// Load `.mat` with extra comma-separated shader defines (dedup key `path#defines`).
    #[methoddb::bind(name = "load_with_defines")]
    pub fn api_materials_load_with_defines(&mut self, path: &str, defines: &str) -> u64 {
        materials_load_asset_with_defines(self, path, defines)
    }

    /// Resolve a material [`AssetId`] into a client `MaterialHandle` (GPU row).
    #[methoddb::bind(name = "resolve")]
    pub fn api_materials_resolve(&mut self, asset_id: u64) -> u32 {
        materials_resolve(self, asset_id)
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
    pub fn api_materials_set_texture(&mut self, handle: u32, prop_name: &str, slot: u32) -> i32 {
        materials_set_texture(self, handle, prop_name, slot)
    }
}
