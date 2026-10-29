//! Shader registry — loads `.wgsl` files, preprocesses `#@` metadata,
//! compiles via naga, reflects bind group layouts, caches by (path, defines).
//!
//! See `.tasks/todo/TASK-002` for the `#@` decorator format.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use deerpunk_engine_common::material::{
    MaterialHandle, PropertyKind, RenderPass, ShaderMeta, ShaderPreprocessor,
};
use deerpunk_engine_common::MeshTexture;

/// Stable id for a compiled shader variant.
pub type ShaderId = deerpunk_engine_common::material::ShaderId;

/// A loaded shader: metadata + WGSL source + compiled bind group layout entries.
#[derive(Clone)]
pub struct ShaderDef {
    pub id: ShaderId,
    /// Load path key (`file://shaders/brush.wgsl`, etc.).
    pub path: String,
    pub meta: ShaderMeta,
    /// Clean WGSL (no `#@`, `#define` applied) — ready for wgpu::ShaderSource.
    pub wgsl: String,
    /// Reflected bind group layout entries (group 0 = material group).
    pub bindings: Vec<BindingEntry>,
}

/// A reflected binding entry from naga.
#[derive(Clone, Debug)]
pub struct BindingEntry {
    pub group: u32,
    pub binding: u32,
    pub kind: BindingKind,
    pub visibility: BindingVisibility,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    Uniform,
    Storage,
    Texture2d,
    Texture2dArray,
    TextureCube,
    Sampler,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingVisibility {
    Vertex,
    Fragment,
    All,
}

/// Registry of compiled shader variants. Cached by (path, sorted defines).
pub struct ShaderRegistry {
    /// ShaderId → ShaderDef. Index 0 = none.
    defs: Vec<Arc<ShaderDef>>,
    /// (path, sorted_defines) → ShaderId
    cache: HashMap<(String, String), ShaderId>,
}

impl ShaderRegistry {
    pub fn new() -> Self {
        Self {
            defs: Vec::new(),
            cache: HashMap::new(),
        }
    }

    /// Load (or get cached) shader by path + defines.
    ///
    /// `path` — e.g. `"file://shaders/pbr.wgsl"` or a raw path.
    /// `defines` — preprocessor keys for ubershader variants.
    /// `source` — the raw .wgsl file contents (caller reads the file).
    pub fn load(
        &mut self,
        path: &str,
        defines: &[String],
        source: &str,
        content_root: &Path,
    ) -> Result<ShaderId, String> {
        let defines_key = canonical_defines(defines);
        let cache_key = (path.to_string(), defines_key.clone());

        if let Some(&id) = self.cache.get(&cache_key) {
            return Ok(id);
        }

        let base = ShaderPreprocessor::shader_include_base(content_root, path);
        let preprocessed = ShaderPreprocessor::process_at(source, defines, base.as_deref())
            .map_err(|e| format!("shader preprocess '{path}': {e}"))?;

        let bindings = reflect_bindings(&preprocessed.wgsl)
            .map_err(|e| format!("shader reflect '{path}': {e}"))?;

        let id = self.defs.len() as ShaderId + 1; // 0 = none
        let def = ShaderDef {
            id,
            path: path.to_string(),
            meta: preprocessed.meta,
            wgsl: preprocessed.wgsl,
            bindings,
        };

        self.defs.push(Arc::new(def));
        self.cache.insert(cache_key, id);
        Ok(id)
    }

    /// Get a shader def by id.
    pub fn get(&self, id: ShaderId) -> Option<&Arc<ShaderDef>> {
        if id == 0 {
            return None;
        }
        self.defs.get(id as usize - 1)
    }

    /// Find a shader by path + defines (reverse lookup of the cache).
    pub fn get_by_path(&self, path: &str, defines: &[String]) -> Option<&Arc<ShaderDef>> {
        let defines_key = canonical_defines(defines);
        let key = (path.to_string(), defines_key);
        let &id = self.cache.get(&key)?;
        self.get(id)
    }

    /// Number of loaded shader variants.
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Human-readable shader path for HUD (`r_speeds 3`).
    pub fn path_for_id(&self, id: ShaderId) -> Option<&str> {
        self.get(id).map(|d| d.path.as_str())
    }

    /// All cached `(path, defines_key)` pairs (for hot-reload).
    pub fn cached_variants(&self) -> Vec<(String, String)> {
        self.cache.keys().cloned().collect()
    }

    /// Recompile WGSL for an already-loaded variant (same [`ShaderId`], new source).
    pub fn reload(
        &mut self,
        path: &str,
        defines: &[String],
        source: &str,
        content_root: &Path,
    ) -> Result<ShaderId, String> {
        let defines_key = canonical_defines(defines);
        let cache_key = (path.to_string(), defines_key);
        let id = self
            .cache
            .get(&cache_key)
            .copied()
            .ok_or_else(|| format!("shader not loaded: {path}"))?;

        let base = ShaderPreprocessor::shader_include_base(content_root, path);
        let preprocessed = ShaderPreprocessor::process_at(source, defines, base.as_deref())
            .map_err(|e| format!("shader preprocess '{path}': {e}"))?;
        let bindings = reflect_bindings(&preprocessed.wgsl)
            .map_err(|e| format!("shader reflect '{path}': {e}"))?;

        let def = ShaderDef {
            id,
            path: path.to_string(),
            meta: preprocessed.meta,
            wgsl: preprocessed.wgsl,
            bindings,
        };
        self.defs[id as usize - 1] = Arc::new(def);
        Ok(id)
    }
}

impl ShaderDef {
    /// Group 1 binds a 2D albedo + `texture_2d_array` lightmap (shader-defined layout).
    pub fn group1_uses_lightmap_array(&self) -> bool {
        self.bindings
            .iter()
            .any(|b| b.group == 1 && b.kind == BindingKind::Texture2dArray)
    }

    /// BSP / brush shaders use extended vertex inputs (`atlas_rect`, `lm_uv`, …).
    pub fn uses_extended_mesh_vertex(&self) -> bool {
        self.group1_uses_lightmap_array()
            || self.wgsl.contains("atlas_rect")
            || self.wgsl.contains("lm_uv")
    }

    /// Skinned opaque mesh: bone palette in group 0 binding 1 (`mesh_skinned` layout).
    pub fn uses_skinned_mesh_vertex(&self) -> bool {
        if self.meta.skinned {
            return true;
        }
        self.meta.pass == Some(RenderPass::MeshOpaque)
            && self.bindings.iter().any(|b| {
                b.group == 0 && b.binding == 1 && b.kind == BindingKind::Uniform
            })
    }
}

/// Canonicalize defines: sorted, comma-joined. Empty → "".
fn canonical_defines(defines: &[String]) -> String {
    if defines.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&str> = defines.iter().map(|s| s.as_str()).collect();
    sorted.sort_unstable();
    sorted.join(",")
}

/// Reflect bind group entries from WGSL source using naga.
///
/// Parses the WGSL and extracts all `@group(N) @binding(M)` resource bindings.
fn reflect_bindings(wgsl: &str) -> Result<Vec<BindingEntry>, String> {
    use naga::front::wgsl;
    use naga::valid::{Validator, Capabilities, ValidationFlags};

    let module = wgsl::parse_str(wgsl)
        .map_err(|e| format!("naga parse: {e:?}"))?;

    // Validate to catch errors early.
    let _ = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map_err(|e| format!("naga validate: {e:?}"))?;

    let mut entries = Vec::new();

    // Walk module types and global variables for @group/@binding.
    for (_, var) in module.global_variables.iter() {
        let Some(binding) = &var.binding else { continue };
        let group = binding.group;
        let binding_idx = binding.binding;

        let kind = classify_binding(&module, var);
        let visibility = if var.space == naga::AddressSpace::Uniform {
            BindingVisibility::All
        } else {
            BindingVisibility::Fragment
        };

        entries.push(BindingEntry {
            group,
            binding: binding_idx,
            kind,
            visibility,
        });
    }

    entries.sort_by_key(|e| (e.group, e.binding));
    Ok(entries)
}

/// Classify a naga global variable into a BindingKind.
fn classify_binding(module: &naga::Module, var: &naga::GlobalVariable) -> BindingKind {
    match &module.types[var.ty].inner {
        naga::TypeInner::Struct { .. } => BindingKind::Uniform,
        naga::TypeInner::Image { dim, arrayed, .. } => match (dim, arrayed) {
            (naga::ImageDimension::D2, true) => BindingKind::Texture2dArray,
            (naga::ImageDimension::D2, false) => BindingKind::Texture2d,
            (naga::ImageDimension::Cube, _) => BindingKind::TextureCube,
            _ => BindingKind::Texture2d,
        },
        naga::TypeInner::Sampler { .. } => BindingKind::Sampler,
        naga::TypeInner::BindingArray { .. } => BindingKind::Storage,
        _ => BindingKind::Uniform,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_and_reflect() {
        let src = r#"#@shader pass=mesh_opaque queue=1000

#@prop _BaseColor color [1,1,1,1]
@group(0) @binding(0) var<uniform> u_material: MaterialUniform;

#@prop _BaseMap texture2d white
@group(0) @binding(1) var base_map: texture_2d<f32>;

@group(0) @binding(2) var samp: sampler;

struct MaterialUniform {
    base_color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return vec4<f32>(pos, 1.0);
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return u_material.base_color;
}
"#;
        let mut reg = ShaderRegistry::new();
        let id = reg.load("test.wgsl", &[], src, Path::new(".")).unwrap();
        let def = reg.get(id).unwrap();
        assert_eq!(def.meta.pass, Some(RenderPass::MeshOpaque));
        assert_eq!(def.meta.properties.len(), 2);
        assert_eq!(def.bindings.len(), 3);
        assert_eq!(def.bindings[0].binding, 0);
        assert_eq!(def.bindings[0].kind, BindingKind::Uniform);
        assert_eq!(def.bindings[1].binding, 1);
        assert_eq!(def.bindings[1].kind, BindingKind::Texture2d);
        assert_eq!(def.bindings[2].binding, 2);
        assert_eq!(def.bindings[2].kind, BindingKind::Sampler);
    }

    #[test]
    fn cache_dedup() {
        let src = r#"#@shader pass=mesh_opaque queue=0
@group(0) @binding(0) var<uniform> u: f32;
@vertex fn vs(@location(0) p: vec3<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(p, 1.0); }
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }
"#;
        let mut reg = ShaderRegistry::new();
        let id1 = reg.load("a.wgsl", &[], src, Path::new(".")).unwrap();
        let id2 = reg.load("a.wgsl", &[], src, Path::new(".")).unwrap();
        assert_eq!(id1, id2);
    }
}
