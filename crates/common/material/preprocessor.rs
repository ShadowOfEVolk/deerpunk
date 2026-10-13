//! Shader preprocessor — extracts `#@` metadata and applies `#define`/`#ifdef`
//! before handing clean WGSL to naga.
//!
//! Line-based, single pass. Three jobs:
//! 1. Parse `#@shader` and `#@prop` directives → [`ShaderMeta`]
//! 2. Apply `#define` / `#ifdef` / `#ifndef` / `#else` / `#endif` (ubershader variants)
//! 3. Strip all `#@` and preprocessor lines → clean WGSL for naga
//!
//! `#@prop` associates with the next `@group(N) @binding(M) var ...` or
//! struct field below it. Binding index is read from `@binding(M)`.

use crate::material::shader_meta::{
    PropDefault, PropertyKind, RenderPass, ShaderMeta, ShaderPropertyMeta,
};

/// Error during shader preprocessing.
#[derive(Debug)]
pub enum ShaderError {
    /// `#@shader` line with unknown key.
    UnknownShaderKey(String),
    /// `#@prop` line malformed.
    PropParse(String),
    /// `#@prop` not followed by a binding (dangling).
    DanglingProp(String),
    /// `#ifdef` without `#endif`.
    UnclosedIfdef,
    /// Unknown `#@` directive.
    UnknownDirective(String),
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownShaderKey(s) => write!(f, "unknown #@shader key: {s}"),
            Self::PropParse(s) => write!(f, "#@prop parse error: {s}"),
            Self::DanglingProp(s) => write!(f, "#@prop '{s}' not followed by a binding"),
            Self::UnclosedIfdef => write!(f, "#ifdef without #endif"),
            Self::UnknownDirective(s) => write!(f, "unknown #@ directive: {s}"),
        }
    }
}

impl std::error::Error for ShaderError {}

/// Result of preprocessing: metadata + cleaned WGSL.
pub struct PreprocessedShader {
    pub meta: ShaderMeta,
    pub wgsl: String,
}

/// Line-based shader preprocessor.
pub struct ShaderPreprocessor;

impl ShaderPreprocessor {
    /// Process raw `.wgsl` source with optional preprocessor defines.
    ///
    /// `defines` — list of `#define` names active (from `load(path#DEF1,DEF2)`).
    pub fn process(source: &str, defines: &[String]) -> Result<PreprocessedShader, ShaderError> {
        let define_set: std::collections::HashSet<&str> =
            defines.iter().map(|s| s.as_str()).collect();

        let mut meta = ShaderMeta::default();
        let mut output = String::with_capacity(source.len());

        // Pending #@prop waiting for the next binding/field.
        let mut pending_prop: Option<(String, PropertyKind, PropDefault)> = None;

        // #ifdef stack: each entry is (defined?, currently_emitting?)
        let mut ifdef_stack: Vec<(bool, bool)> = Vec::new();

        for line in source.lines() {
            let trimmed = line.trim();

            // --- Preprocessor: #ifdef / #ifndef / #else / #endif / #define ---
            if let Some(rest) = trimmed.strip_prefix("#ifdef ") {
                let name = rest.trim();
                let defined = define_set.contains(name);
                let emitting = ifdef_stack.iter().all(|(_, e)| *e) && defined;
                ifdef_stack.push((defined, emitting));
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("#ifndef ") {
                let name = rest.trim();
                let defined = define_set.contains(name);
                let emitting = ifdef_stack.iter().all(|(_, e)| *e) && !defined;
                ifdef_stack.push((!defined, emitting));
                continue;
            }
            if trimmed == "#else" {
                let (defined, _) = ifdef_stack.pop().ok_or(ShaderError::UnclosedIfdef)?;
                let parent_emitting = ifdef_stack.iter().all(|(_, e)| *e);
                let emitting = parent_emitting && !defined;
                ifdef_stack.push((defined, emitting));
                continue;
            }
            if trimmed == "#endif" {
                ifdef_stack.pop().ok_or(ShaderError::UnclosedIfdef)?;
                continue;
            }
            // #define lines are consumed (defines come from load query, not source)
            if trimmed.starts_with("#define ") || trimmed == "#define" {
                continue;
            }

            // Check if we're inside a non-emitting #ifdef block
            let emitting = ifdef_stack.iter().all(|(_, e)| *e);
            if !emitting {
                continue;
            }

            // --- #@ directives ---
            if let Some(directive) = trimmed.strip_prefix("#@") {
                let directive = directive.trim();

                if let Some(rest) = directive.strip_prefix("shader ") {
                    parse_shader_line(rest, &mut meta)?;
                } else if let Some(rest) = directive.strip_prefix("prop ") {
                    // Save pending prop; associate with next binding/field
                    if let Some((name, kind, default)) = parse_prop_line(rest)? {
                        if let Some((prev_name, _, _)) = pending_prop.take() {
                            // Previous prop was never associated — dangling
                            return Err(ShaderError::DanglingProp(prev_name));
                        }
                        pending_prop = Some((name, kind, default));
                    }
                } else {
                    return Err(ShaderError::UnknownDirective(directive.to_string()));
                }
                continue;
            }

            // --- Regular WGSL line — check for @binding association ---
            if let Some((name, kind, default)) = pending_prop.take() {
                if let Some(binding) = extract_binding_index(trimmed) {
                    meta.properties.push(ShaderPropertyMeta {
                        name,
                        kind,
                        default,
                        binding: Some(binding),
                    });
                } else {
                    // Not a binding line — could be a struct field with @location
                    // or a comment. Re-queue for next line.
                    // For struct fields, we look for "field_name: type," pattern.
                    // Binding comes from the enclosing uniform's @binding.
                    // TODO: struct field association (multiple props in one uniform).
                    // For now, associate with the struct's @binding when we see it.
                    pending_prop = Some((name, kind, default));
                }
            }

            output.push_str(line);
            output.push('\n');
        }

        if !ifdef_stack.is_empty() {
            return Err(ShaderError::UnclosedIfdef);
        }

        if let Some((name, _, _)) = pending_prop {
            return Err(ShaderError::DanglingProp(name));
        }

        Ok(PreprocessedShader { meta, wgsl: output })
    }
}

/// Parse `#@shader pass=mesh_opaque queue=1000` → fill ShaderMeta.
fn parse_shader_line(rest: &str, meta: &mut ShaderMeta) -> Result<(), ShaderError> {
    for token in rest.split_whitespace() {
        if let Some((key, val)) = token.split_once('=') {
            match key.trim() {
                "pass" => {
                    meta.pass = Some(RenderPass::parse(val).ok_or_else(|| {
                        ShaderError::UnknownShaderKey(format!("pass={val}"))
                    })?);
                }
                "queue" => {
                    meta.queue = val.trim().parse().unwrap_or(0);
                }
                _ => return Err(ShaderError::UnknownShaderKey(key.to_string())),
            }
        }
    }
    Ok(())
}

/// Parse `#@prop _BaseColor color [1,1,1,1]` → (name, kind, default).
/// Returns None for blank/comment props (skip).
fn parse_prop_line(
    rest: &str,
) -> Result<Option<(String, PropertyKind, PropDefault)>, ShaderError> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Ok(None);
    }

    // Split into: name  kind  rest(default)
    let mut parts = rest.splitn(3, char::is_whitespace);
    let name = parts.next().ok_or_else(|| ShaderError::PropParse(rest.into()))?;
    let kind_str = parts.next().ok_or_else(|| ShaderError::PropParse(rest.into()))?;
    let default_str = parts.next().unwrap_or("").trim();

    let kind = PropertyKind::parse(kind_str)
        .ok_or_else(|| ShaderError::PropParse(format!("unknown kind '{kind_str}'")))?;

    let default = parse_default(default_str, kind)
        .ok_or_else(|| ShaderError::PropParse(format!("bad default '{default_str}'")))?;

    Ok(Some((name.to_string(), kind, default)))
}

/// Parse a default value string based on property kind.
fn parse_default(s: &str, kind: PropertyKind) -> Option<PropDefault> {
    let s = s.trim();
    match kind {
        PropertyKind::Float => {
            let v: f32 = s.parse().ok()?;
            Some(PropDefault::Float(v))
        }
        PropertyKind::Color | PropertyKind::Vec4 => {
            let arr = parse_float_array(s, 4)?;
            Some(PropDefault::Color([arr[0], arr[1], arr[2], arr[3]]))
        }
        PropertyKind::Vec3 => {
            let arr = parse_float_array(s, 3)?;
            Some(PropDefault::Vec3([arr[0], arr[1], arr[2]]))
        }
        PropertyKind::Texture2d | PropertyKind::TextureCube => {
            // Strip quotes if present
            let name = s.trim_matches('"').to_string();
            if name.is_empty() {
                Some(PropDefault::Texture("white".into()))
            } else {
                Some(PropDefault::Texture(name))
            }
        }
    }
}

/// Parse `[1,1,1,1]` or `1,1,1,1` into a fixed-size f32 array.
fn parse_float_array(s: &str, n: usize) -> Option<Vec<f32>> {
    let s = s.trim().trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
    if parts.len() != n {
        return None;
    }
    parts.iter().map(|p| p.parse::<f32>().ok()).collect()
}

/// Extract `@binding(N)` from a WGSL line. Returns the binding index.
fn extract_binding_index(line: &str) -> Option<u32> {
    // Look for @binding(N) in the line
    let mut rest = line;
    while let Some(pos) = rest.find("@binding(") {
        let after = &rest[pos + "@binding(".len()..];
        if let Some(end) = after.find(')') {
            let num_str = &after[..end];
            if let Ok(n) = num_str.parse::<u32>() {
                return Some(n);
            }
        }
        rest = &rest[pos + 1..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_shader() {
        let src = r#"#@shader pass=mesh_opaque queue=1000

#@prop _BaseColor color [1,1,1,1]
@group(0) @binding(0) var<uniform> u_material: MaterialUniform;

#@prop _BaseMap texture2d white
@group(0) @binding(1) var base_map: texture_2d<f32>;

@group(0) @binding(4) var samp: sampler;
"#;
        let result = ShaderPreprocessor::process(src, &[]).unwrap();
        assert_eq!(result.meta.pass, Some(RenderPass::MeshOpaque));
        assert_eq!(result.meta.queue, 1000);
        assert_eq!(result.meta.properties.len(), 2);
        assert_eq!(result.meta.properties[0].name, "_BaseColor");
        assert_eq!(result.meta.properties[0].binding, Some(0));
        assert_eq!(result.meta.properties[1].name, "_BaseMap");
        assert_eq!(result.meta.properties[1].binding, Some(1));
        // Clean WGSL: no #@ lines
        assert!(!result.wgsl.contains("#@"));
    }

    #[test]
    fn ifdef_excludes_block() {
        let src = r#"#@shader pass=mesh_opaque queue=0
#ifdef ALPHA_MASK
@group(1) @binding(0) var<uniform> alpha_mask: f32;
#endif
@group(0) @binding(0) var<uniform> u: f32;
"#;
        let without = ShaderPreprocessor::process(src, &[]).unwrap();
        assert!(!without.wgsl.contains("alpha_mask"));

        let with = ShaderPreprocessor::process(src, &["ALPHA_MASK".into()]).unwrap();
        assert!(with.wgsl.contains("alpha_mask"));
    }

    #[test]
    fn dangling_prop_errors() {
        let src = "#@prop _Foo float 1.0\n// no binding here\n";
        let result = ShaderPreprocessor::process(src, &[]);
        assert!(result.is_err());
    }
}
