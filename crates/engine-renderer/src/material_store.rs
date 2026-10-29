//! Material store — client-side material asset storage with GPU upload.
//!
//! Path → [`MaterialHandle`] → [`GpuMaterialIndex`]. One asset shared by N
//! entities. Dedup by path and content hash. No allocs in hot path (index lookup).
//!
//! See `.tasks/todo/TASK-003` and `TASK-004` for the full design.

use std::collections::HashMap;

use deerpunk_engine_common::lightmap_atlas::LightmapAtlasLayers;
use deerpunk_engine_common::material::{
    MaterialAsset, MaterialHandle, PropertyValueSer, RenderPass, ShaderId, ShaderMeta,
};
use deerpunk_engine_common::MeshTexture;

use crate::material_allocator::{MaterialAllocator, MaterialGpuEntry, MATERIAL_WHITE_ID};
use crate::shader_registry::ShaderRegistry;

/// Resolved property value — textures resolved to GPU slots, scalars inline.
#[derive(Clone, Debug)]
pub enum PropertyValue {
    Float(f32),
    Color([f32; 4]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    /// Resolved texture slot in the renderer's texture array.
    Texture(MeshTexture),
}

/// One material entry in the store.
pub struct MaterialEntry {
    pub shader: ShaderId,
    /// Resolved properties, indexed by position in ShaderMeta.properties.
    pub props: Vec<PropertyValue>,
    /// GPU SSBO row index (lazy, 0 = not yet uploaded).
    pub gpu_index: u32,
    /// Incremented on reload; invalidates cached GPU state.
    pub generation: u64,
    /// Resolved texture slots for MeshDraw (filled during property resolution).
    pub albedo: MeshTexture,
    pub normal: MeshTexture,
    pub orm: MeshTexture,
    /// Resolved scalar params for the GPU SSBO entry.
    pub color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emission: [f32; 4],
    /// GPU lightmap atlas id (`upload_lightmap_layers`), from `_Lightmap` URI at load.
    pub lightmap_id: u32,
    /// From shader `#@shader pass=...` (opaque vs transparent queue, etc.).
    pub render_pass: RenderPass,
}

impl MaterialEntry {
    /// Create a white/default entry.
    fn white() -> Self {
        Self {
            shader: 0,
            props: Vec::new(),
            gpu_index: 0,
            generation: 0,
            albedo: MeshTexture::WHITE,
            normal: MeshTexture::WHITE,
            orm: MeshTexture::WHITE,
            color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.5,
            emission: [0.0; 4],
            lightmap_id: 0,
            render_pass: RenderPass::MeshOpaque,
        }
    }
}

/// Client-side material store. Owns CPU entries; GPU SSBO rows live in the
/// renderer's shared `MaterialAllocator` (same buffer the legacy `Material`
/// path writes to, so bind group 2 is consistent for both paths).
pub struct MaterialStore {
    /// Index = MaterialHandle - 1. Handle 0 = white/default.
    entries: Vec<MaterialEntry>,
    /// path → MaterialHandle (dedup by path).
    by_path: HashMap<String, MaterialHandle>,
    /// content hash → MaterialHandle (dedup by content).
    by_hash: HashMap<u64, MaterialHandle>,
}

impl MaterialStore {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            by_path: HashMap::new(),
            by_hash: HashMap::new(),
        }
    }

    /// Drop all loaded materials (client disconnect / map teardown). Reload on next `load`.
    pub fn reset(&mut self) {
        self.entries.clear();
        self.by_path.clear();
        self.by_hash.clear();
    }

    /// After shader hot-reload: force GPU material rows to be re-uploaded.
    pub fn bump_shader_generations(&mut self) {
        for entry in &mut self.entries {
            if entry.shader == 0 {
                continue;
            }
            entry.generation += 1;
            entry.gpu_index = 0;
        }
    }

    /// Already-loaded `.mat` path (no disk read).
    pub fn handle_for_path(&self, path: &str) -> Option<MaterialHandle> {
        self.by_path.get(path).copied()
    }

    /// Get or load a material by path. Dedup by path.
    ///
    /// `source` — the raw .mat file contents (JSON or CBOR, caller reads the file).
    /// `shaders` — registry for shader resolution.
    /// `tex_resolver` — resolves texture paths to GPU slots.
    pub fn load(
        &mut self,
        path: &str,
        source: &str,
        shaders: &mut ShaderRegistry,
        tex_resolver: &mut dyn TextureResolver,
    ) -> Result<MaterialHandle, String> {
        if let Some(&h) = self.by_path.get(path) {
            return Ok(h);
        }

        let asset = MaterialAsset::from_json(source)
            .map_err(|e| format!("material parse '{path}': {e}"))?;

        self.create_from_asset(path, &asset, shaders, tex_resolver)
    }

    /// Create a material from an asset (no file). Dedup by content hash.
    pub fn create_from_asset(
        &mut self,
        debug_path: &str,
        asset: &MaterialAsset,
        shaders: &mut ShaderRegistry,
        tex_resolver: &mut dyn TextureResolver,
    ) -> Result<MaterialHandle, String> {
        let mut asset = asset.clone();
        asset.normalize_lightmap_texture_props();
        let shader_id =
            resolve_shader_id(shaders, tex_resolver, &asset.shader, &asset.shader_defines)?;
        let shader_def = shaders
            .get(shader_id)
            .ok_or_else(|| format!("shader not loaded: {}", asset.shader))?;
        let shader_meta = shader_def.meta.clone();

        // Resolve properties against shader schema.
        let props = resolve_properties(&asset, &shader_meta, tex_resolver)?;

        let hash = material_content_hash(&asset, &props);
        if let Some(&h) = self.by_hash.get(&hash) {
            // Same content, different path → alias.
            self.by_path.insert(debug_path.to_string(), h);
            return Ok(h);
        }

        let handle = MaterialHandle(self.entries.len() as u32 + 1);
        let mut entry = MaterialEntry::white();
        entry.shader = shader_id;
        entry.render_pass = shader_meta.pass.unwrap_or(RenderPass::MeshOpaque);
        entry.props = props;
        fill_draw_fields(&mut entry, &shader_meta);

        self.entries.push(entry);
        self.by_path.insert(debug_path.to_string(), handle);
        self.by_hash.insert(hash, handle);

        Ok(handle)
    }

    /// Get a material entry by handle.
    pub fn get(&self, handle: MaterialHandle) -> Option<&MaterialEntry> {
        if handle.is_none() {
            return None;
        }
        self.entries.get(handle.0 as usize - 1)
    }

    /// Get a material entry mutably.
    pub fn get_mut(&mut self, handle: MaterialHandle) -> Option<&mut MaterialEntry> {
        if handle.is_none() {
            return None;
        }
        self.entries.get_mut(handle.0 as usize - 1)
    }

    /// Resolve a handle to its GPU material index. Uploads to the shared
    /// `MaterialAllocator` SSBO if needed (same buffer the renderer binds).
    pub fn resolve_gpu(
        &mut self,
        handle: MaterialHandle,
        allocator: &mut MaterialAllocator,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> u32 {
        self.resolve_gpu_tracked(handle, allocator, device, queue, None)
    }

    /// Like [`resolve_gpu`], optionally counts upload vs cache for [`crate::render_profile::SceneExtractGpuStats`].
    pub fn resolve_gpu_tracked(
        &mut self,
        handle: MaterialHandle,
        allocator: &mut MaterialAllocator,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stats: Option<&mut crate::render_profile::SceneExtractGpuStats>,
    ) -> u32 {
        if handle.is_none() {
            return MATERIAL_WHITE_ID;
        }
        let Some(entry) = self.entries.get_mut(handle.0 as usize - 1) else {
            return MATERIAL_WHITE_ID;
        };
        if entry.gpu_index != 0 {
            if let Some(s) = stats {
                s.material_cached += 1;
            }
            return entry.gpu_index;
        }

        let gpu_entry = MaterialGpuEntry {
            color: entry.color,
            params: [entry.metallic, entry.roughness, 0.0, 0.0],
            albedo_idx: entry.albedo.0,
            normal_idx: entry.normal.0,
            orm_idx: entry.orm.0,
            _pad: 0,
            emission: entry.emission,
            sun_dir: [0.0, 0.0, 1.0, 0.0],
            ambient: [0.0; 4],
        };
        let id = allocator.ensure_uploaded_raw(device, queue, &gpu_entry);
        entry.gpu_index = id;
        if let Some(s) = stats {
            s.material_uploads += 1;
        }
        id
    }

    /// Update a texture property on an existing material by name.
    ///
    /// Sets the convenience field (`albedo`/`normal`/`orm`) and invalidates
    /// the GPU cache so the next `resolve_gpu` re-uploads with the new
    /// texture. All entities sharing this handle pick up the change — no
    /// need to re-assign `DrawMaterial` per entity.
    ///
    /// `prop_name` — one of `"_BaseMap"`, `"_NormalMap"`, `"_ORMMap"`.
    /// Returns `false` if the handle is invalid or the prop name is unknown.
    pub fn set_texture(
        &mut self,
        handle: MaterialHandle,
        prop_name: &str,
        slot: MeshTexture,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(handle.0 as usize - 1) else {
            return false;
        };
        match prop_name {
            "_BaseMap" => entry.albedo = slot,
            "_NormalMap" => entry.normal = slot,
            "_ORMMap" => entry.orm = slot,
            _ => return false,
        }
        // Invalidate GPU cache — next resolve_gpu re-uploads.
        entry.gpu_index = 0;
        entry.generation = entry.generation.wrapping_add(1);
        true
    }

    /// Number of loaded materials.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Upload `_Lightmap` BSP URIs (`#lightmap:…`) after [`create_from_asset`].
    pub fn attach_lightmaps_from_asset(
        &mut self,
        handle: MaterialHandle,
        asset: &MaterialAsset,
        resources: &deerpunk_engine_common::resource::ResourceManager,
        renderer: &mut crate::gpu::Renderer,
    ) {
        let Some(entry) = self.entries.get_mut(handle.0 as usize - 1) else {
            return;
        };
        for (name, value) in &asset.properties {
            let PropertyValueSer::Texture(uri) = value else {
                continue;
            };
            if !uri.contains("#lightmap:") {
                continue;
            }
            if name != "_Lightmap" && entry.lightmap_id != 0 {
                continue;
            }
            if let Ok(id) = upload_lightmap_uri(resources, renderer, uri) {
                entry.lightmap_id = id;
            }
        }
    }
}

/// Load `file://…#lightmap:…` and upload to the GPU lightmap pool.
pub fn upload_lightmap_uri(
    resources: &deerpunk_engine_common::resource::ResourceManager,
    renderer: &mut crate::gpu::Renderer,
    uri: &str,
) -> Result<u32, String> {
    if !uri.contains("#lightmap:") {
        return Ok(0);
    }
    let handle = resources.load::<LightmapAtlasLayers>(uri);
    let atlas = handle
        .try_get()
        .ok_or_else(|| format!("lightmap resource not loaded: '{uri}'"))?;
    Ok(renderer.upload_lightmap_layers(
        atlas.width,
        atlas.height,
        [
            &atlas.layers[0].pixels,
            &atlas.layers[1].pixels,
            &atlas.layers[2].pixels,
            &atlas.layers[3].pixels,
        ],
    ))
}

/// Load WGSL via [`ResourceManager`] (cached by URI), compile via [`ShaderRegistry`] (cached by path+defines).
fn resolve_shader_id(
    shaders: &mut ShaderRegistry,
    tex_resolver: &mut dyn TextureResolver,
    path: &str,
    defines: &[String],
) -> Result<ShaderId, String> {
    let source = tex_resolver.load_shader_source(path)?;
    let root = tex_resolver.shader_content_root();
    shaders.load(path, defines, &source, &root)
}

/// Resolve material asset properties against shader schema.
fn resolve_properties(
    asset: &MaterialAsset,
    meta: &ShaderMeta,
    tex_resolver: &mut dyn TextureResolver,
) -> Result<Vec<PropertyValue>, String> {
    let mut result = Vec::with_capacity(meta.properties.len());
    for prop_meta in &meta.properties {
        let value = asset
            .get(&prop_meta.name)
            .map(|v| resolve_value(&prop_meta.name, v, prop_meta.kind, tex_resolver))
            .transpose()?
            .unwrap_or_else(|| {
                default_value(
                    &prop_meta.name,
                    &prop_meta.default,
                    prop_meta.kind,
                    tex_resolver,
                )
            });
        result.push(value);
    }
    Ok(result)
}

/// Extract MeshDraw-relevant fields from resolved properties.
/// Fills the MaterialEntry's albedo/normal/orm/color/metallic/roughness/emission
/// by matching property **names** against the standard PBR convention
/// (`_BaseColor`, `_Metallic`, `_Roughness`, `_BaseMap`, `_NormalMap`, `_ORMMap`,
/// `_Emission`).
fn fill_draw_fields(entry: &mut MaterialEntry, meta: &ShaderMeta) {
    for (prop_meta, value) in meta.properties.iter().zip(entry.props.iter()) {
        match (prop_meta.name.as_str(), value) {
            ("_BaseColor", PropertyValue::Color(v)) => entry.color = *v,
            ("_Emission", PropertyValue::Color(v)) => entry.emission = *v,
            ("_Metallic", PropertyValue::Float(v)) => entry.metallic = *v,
            ("_Roughness", PropertyValue::Float(v)) => entry.roughness = *v,
            ("_BaseMap", PropertyValue::Texture(slot)) => entry.albedo = *slot,
            ("_NormalMap", PropertyValue::Texture(slot)) => entry.normal = *slot,
            ("_ORMMap", PropertyValue::Texture(slot)) => entry.orm = *slot,
            _ => {}
        }
    }
}

/// Resolve a serialized value to a runtime value.
fn resolve_value(
    prop_name: &str,
    ser: &PropertyValueSer,
    kind: deerpunk_engine_common::material::PropertyKind,
    tex_resolver: &mut dyn TextureResolver,
) -> Result<PropertyValue, String> {
    use deerpunk_engine_common::material::PropertyKind as K;
    match (ser, kind) {
        (PropertyValueSer::Float(v), K::Float) => Ok(PropertyValue::Float(*v)),
        (PropertyValueSer::Color(v), K::Color) => Ok(PropertyValue::Color(*v)),
        (PropertyValueSer::Vec3(v), K::Vec3) => Ok(PropertyValue::Vec3(*v)),
        (PropertyValueSer::Vec4(v), K::Vec4) => Ok(PropertyValue::Vec4(*v)),
        (PropertyValueSer::Texture(path), K::Texture2d) => {
            if is_lightmap_texture_uri(prop_name, path) {
                return Ok(PropertyValue::Texture(MeshTexture::WHITE));
            }
            let slot = tex_resolver.resolve(path)?;
            Ok(PropertyValue::Texture(slot))
        }
        (PropertyValueSer::Texture(path), K::TextureCube) => {
            if is_lightmap_texture_uri(prop_name, path) {
                return Ok(PropertyValue::Texture(MeshTexture::WHITE));
            }
            let slot = tex_resolver.resolve(path)?;
            Ok(PropertyValue::Texture(slot))
        }
        _ => Err(format!("property value kind mismatch")),
    }
}

#[inline]
fn is_lightmap_texture_uri(_prop_name: &str, path: &str) -> bool {
    path.contains("#lightmap:")
}

/// Get default value for a property.
fn default_value(
    prop_name: &str,
    default: &deerpunk_engine_common::material::PropDefault,
    kind: deerpunk_engine_common::material::PropertyKind,
    tex_resolver: &mut dyn TextureResolver,
) -> PropertyValue {
    use deerpunk_engine_common::material::{PropDefault, PropertyKind as K};
    match (default, kind) {
        (PropDefault::Float(v), K::Float) => PropertyValue::Float(*v),
        (PropDefault::Color(v), K::Color) => PropertyValue::Color(*v),
        (PropDefault::Color(v), K::Vec4) => PropertyValue::Vec4(*v),
        (PropDefault::Vec3(v), K::Vec3) => PropertyValue::Vec3(*v),
        (PropDefault::Vec4(v), K::Vec4) => PropertyValue::Vec4(*v),
        (PropDefault::Texture(name), K::Texture2d)
        | (PropDefault::Texture(name), K::TextureCube) => {
            let uri = deerpunk_engine_common::resource::fallback_name_to_texgen_uri(name)
                .unwrap_or(name.as_str());
            if is_lightmap_texture_uri(prop_name, uri) {
                return PropertyValue::Texture(MeshTexture::WHITE);
            }
            let slot = tex_resolver
                .resolve(uri)
                .expect("material default texture resolve");
            PropertyValue::Texture(slot)
        }
        _ => PropertyValue::Float(0.0),
    }
}

/// Hash resolved properties + BSP lightmap URIs (not stored in [`PropertyValue`]).
fn material_content_hash(asset: &MaterialAsset, props: &[PropertyValue]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    asset.shader.hash(&mut hasher);
    for def in &asset.shader_defines {
        def.hash(&mut hasher);
    }
    for prop in props {
        match prop {
            PropertyValue::Float(v) => v.to_bits().hash(&mut hasher),
            PropertyValue::Color(v) => {
                for f in v {
                    f.to_bits().hash(&mut hasher);
                }
            }
            PropertyValue::Vec3(v) => {
                for f in v {
                    f.to_bits().hash(&mut hasher);
                }
            }
            PropertyValue::Vec4(v) => {
                for f in v {
                    f.to_bits().hash(&mut hasher);
                }
            }
            PropertyValue::Texture(slot) => slot.0.hash(&mut hasher),
        }
    }
    for (name, value) in &asset.properties {
        if let PropertyValueSer::Texture(uri) = value {
            if uri.contains("#lightmap:") {
                name.hash(&mut hasher);
                uri.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

/// Trait for resolving texture paths to GPU slots.
/// Implemented by the renderer or a texture resolver wrapper.
pub trait TextureResolver {
    /// Resolve a texture URI (or `slot:N`) to a GPU slot.
    fn resolve(&mut self, path: &str) -> Result<MeshTexture, String>;
    /// Raw WGSL for `asset.shader` (via [`ShaderSource`] in [`ResourceManager`]).
    fn load_shader_source(&mut self, path: &str) -> Result<String, String>;
    /// Gamedir root for `#include` in shader sources (same as [`ResourceManager::base_dir`]).
    fn shader_content_root(&self) -> std::path::PathBuf;
}

/// URI → [`ResourceManager::load`] → GPU upload. Fallback names map to `texgen://…`.
pub struct GpuTextureResolver<'a> {
    pub resources: &'a deerpunk_engine_common::resource::ResourceManager,
    pub renderer: &'a mut crate::gpu::Renderer,
    /// Dedup GPU uploads by resource URI.
    uploaded: HashMap<String, MeshTexture>,
}

impl<'a> GpuTextureResolver<'a> {
    pub fn new(
        resources: &'a deerpunk_engine_common::resource::ResourceManager,
        renderer: &'a mut crate::gpu::Renderer,
    ) -> Self {
        Self {
            resources,
            renderer,
            uploaded: HashMap::new(),
        }
    }
}

impl<'a> TextureResolver for GpuTextureResolver<'a> {
    fn resolve(&mut self, path: &str) -> Result<MeshTexture, String> {
        if let Some(rest) = path.strip_prefix("slot:") {
            let n: u32 = rest
                .parse()
                .map_err(|_| format!("bad slot path '{path}'"))?;
            return Ok(MeshTexture(n));
        }
        let uri =
            deerpunk_engine_common::resource::fallback_name_to_texgen_uri(path).unwrap_or(path);

        if let Some(&slot) = self.uploaded.get(uri) {
            return Ok(slot);
        }

        use deerpunk_engine_common::wad::RgbaImage;
        let handle = self.resources.load::<RgbaImage>(uri);
        let img = handle
            .try_get()
            .ok_or_else(|| format!("texture not loaded: '{uri}' (asset={:?})", handle.id()))?;
        let slot = self
            .renderer
            .upload_texture(&img.pixels, img.width, img.height);
        self.uploaded.insert(uri.to_string(), slot);
        Ok(slot)
    }

    fn load_shader_source(&mut self, path: &str) -> Result<String, String> {
        use deerpunk_engine_common::resource::ShaderSource;
        let handle = self.resources.load::<ShaderSource>(path);
        let src = handle
            .try_get()
            .ok_or_else(|| format!("shader load failed: {path} (asset={:?})", handle.id()))?;
        Ok(src.wgsl)
    }

    fn shader_content_root(&self) -> std::path::PathBuf {
        self.resources.base_dir()
    }
}

/// Back-compat alias.
pub type UriTextureResolver<'a> = GpuTextureResolver<'a>;
