//! Shader preprocessor — extracts `#@` metadata and applies `#define`/`#ifdef`
//! / `#include` before handing clean WGSL to naga.
//!
//! Line-based, single pass on expanded source. Jobs:
//! 1. Expand `#include "path"` (relative to the including file; each file once)
//! 2. Parse `#@shader` and `#@prop` directives → [`ShaderMeta`] (root file only)
//! 3. Apply `#define` / `#ifdef` / `#ifndef` / `#else` / `#endif`
//! 4. Strip all `#@` and preprocessor lines → clean WGSL for naga
//!
//! `#@prop` associates with the next `@group(N) @binding(M) var ...` or
//! struct field below it. Binding index is read from `@binding(M)`.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

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
    /// `#include` without a base directory (use [`ShaderPreprocessor::process_at`]).
    IncludeNoBase,
    /// Malformed `#include` line.
    IncludeSyntax(String),
    /// Included path escapes the shader tree (`..`).
    IncludePathEscape(String),
    /// `#include` target not found.
    IncludeNotFound(String),
    /// Circular `#include` chain.
    IncludeCycle(String),
    /// Failed to read an included file.
    IncludeIo(String),
    /// `#@` directives are not allowed inside included files.
    IncludeHasMetadata(String),
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownShaderKey(s) => write!(f, "unknown #@shader key: {s}"),
            Self::PropParse(s) => write!(f, "#@prop parse error: {s}"),
            Self::DanglingProp(s) => write!(f, "#@prop '{s}' not followed by a binding"),
            Self::UnclosedIfdef => write!(f, "#ifdef without #endif"),
            Self::UnknownDirective(s) => write!(f, "unknown #@ directive: {s}"),
            Self::IncludeNoBase => write!(f, "#include requires a base path (process_at)"),
            Self::IncludeSyntax(s) => write!(f, "#include syntax: {s}"),
            Self::IncludePathEscape(s) => write!(f, "#include path escapes base: {s}"),
            Self::IncludeNotFound(s) => write!(f, "#include not found: {s}"),
            Self::IncludeCycle(s) => write!(f, "#include cycle: {s}"),
            Self::IncludeIo(s) => write!(f, "#include read error: {s}"),
            Self::IncludeHasMetadata(s) => write!(f, "#@ in included file: {s}"),
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
    /// Process raw `.wgsl` with optional defines. `#include` is not resolved
    /// (use [`Self::process_at`] when the source may include other files).
    pub fn process(source: &str, defines: &[String]) -> Result<PreprocessedShader, ShaderError> {
        Self::process_at(source, defines, None)
    }

    /// Like [`process`], but `base` is the directory of the root shader file
    /// (`parent` of `file://…/shaders/foo.wgsl`). Required for `#include`.
    pub fn process_at(
        source: &str,
        defines: &[String],
        base: Option<&Path>,
    ) -> Result<PreprocessedShader, ShaderError> {
        let define_set: HashSet<&str> = defines.iter().map(|s| s.as_str()).collect();
        let expanded = expand_includes(source, &define_set, base, true)?;
        Self::process_expanded(&expanded, &define_set)
    }

    /// Parent directory for `#include`, using the same rules as [`ShaderLoader`]
    /// (`content_root` = gamedir, URI path = `shaders/foo.wgsl` → `gamedir/shaders`).
    pub fn shader_include_base(content_root: &Path, shader_uri: &str) -> Option<PathBuf> {
        let rel = shader_uri.strip_prefix("file://").unwrap_or(shader_uri);
        let file = resolve_content_path(content_root, rel);
        file.parent().map(Path::to_path_buf)
    }

    fn process_expanded(
        source: &str,
        define_set: &HashSet<&str>,
    ) -> Result<PreprocessedShader, ShaderError> {
        let mut meta = ShaderMeta::default();
        let mut output = String::with_capacity(source.len());

        let mut pending_prop: Option<(String, PropertyKind, PropDefault)> = None;
        let mut ifdef_stack: Vec<(bool, bool)> = Vec::new();

        for line in source.lines() {
            let trimmed = line.trim();

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
            if trimmed.starts_with("#define ") || trimmed == "#define" {
                continue;
            }
            if trimmed.starts_with("#include") {
                return Err(ShaderError::IncludeSyntax(
                    "unexpanded #include (missing base path?)".into(),
                ));
            }

            let emitting = ifdef_stack.iter().all(|(_, e)| *e);
            if !emitting {
                continue;
            }

            if let Some(directive) = trimmed.strip_prefix("#@") {
                let directive = directive.trim();

                if let Some(rest) = directive.strip_prefix("shader ") {
                    parse_shader_line(rest, &mut meta)?;
                } else if let Some(rest) = directive.strip_prefix("prop ") {
                    if let Some((name, kind, default)) = parse_prop_line(rest)? {
                        if let Some((prev_name, _, _)) = pending_prop.take() {
                            return Err(ShaderError::DanglingProp(prev_name));
                        }
                        pending_prop = Some((name, kind, default));
                    }
                } else {
                    return Err(ShaderError::UnknownDirective(directive.to_string()));
                }
                continue;
            }

            if let Some((name, kind, default)) = pending_prop.take() {
                if let Some(binding) = extract_binding_index(trimmed) {
                    meta.properties.push(ShaderPropertyMeta {
                        name,
                        kind,
                        default,
                        binding: Some(binding),
                    });
                } else {
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

struct IncludeState {
    /// Open `#include` chain (cycle detection).
    stack: Vec<PathBuf>,
    /// Each file expanded at most once (WGSL has no link step).
    once: HashSet<PathBuf>,
}

fn expand_includes(
    source: &str,
    define_set: &HashSet<&str>,
    base: Option<&Path>,
    is_root: bool,
) -> Result<String, ShaderError> {
    let mut state = IncludeState {
        stack: Vec::new(),
        once: HashSet::new(),
    };
    if !is_root {
        return expand_includes_inner(source, define_set, base, &mut state, false);
    }
    if base.is_none() && source.lines().any(|l| l.trim().starts_with("#include")) {
        return Err(ShaderError::IncludeNoBase);
    }
    expand_includes_inner(source, define_set, base, &mut state, true)
}

fn expand_includes_inner(
    source: &str,
    define_set: &HashSet<&str>,
    base: Option<&Path>,
    state: &mut IncludeState,
    allow_metadata: bool,
) -> Result<String, ShaderError> {
    let mut output = String::with_capacity(source.len());
    let mut ifdef_stack: Vec<(bool, bool)> = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();

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
            ifdef_stack.push((defined, parent_emitting && !defined));
            continue;
        }
        if trimmed == "#endif" {
            ifdef_stack.pop().ok_or(ShaderError::UnclosedIfdef)?;
            continue;
        }
        if trimmed.starts_with("#define ") || trimmed == "#define" {
            continue;
        }

        let emitting = ifdef_stack.iter().all(|(_, e)| *e);
        if !emitting {
            continue;
        }

        if let Some(path_str) = parse_include_path(trimmed) {
            let parent = base.ok_or(ShaderError::IncludeNoBase)?;
            let resolved = resolve_include_path(parent, path_str)?;
            let canonical = resolved
                .canonicalize()
                .map_err(|e| ShaderError::IncludeIo(format!("{}: {e}", resolved.display())))?;

            if state.stack.iter().any(|p| p == &canonical) {
                return Err(ShaderError::IncludeCycle(canonical.display().to_string()));
            }
            if state.once.contains(&canonical) {
                continue;
            }

            let text = std::fs::read_to_string(&resolved)
                .map_err(|e| ShaderError::IncludeNotFound(format!("{}: {e}", resolved.display())))?;

            state.stack.push(canonical.clone());
            state.once.insert(canonical);

            let child_base = resolved.parent();
            let chunk = expand_includes_inner(
                &text,
                define_set,
                child_base,
                state,
                false,
            )?;
            state.stack.pop();

            if !chunk.is_empty() {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&chunk);
                if !chunk.ends_with('\n') {
                    output.push('\n');
                }
            }
            continue;
        }

        if trimmed.starts_with("#@") {
            if allow_metadata {
                output.push_str(line);
                output.push('\n');
                continue;
            }
            return Err(ShaderError::IncludeHasMetadata(trimmed.to_string()));
        }

        output.push_str(line);
        output.push('\n');
    }

    if !ifdef_stack.is_empty() {
        return Err(ShaderError::UnclosedIfdef);
    }

    Ok(output)
}

/// `#include "inc/foo.wgsl"` or `#include "foo.wgsl"`.
fn parse_include_path(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("#include")?;
    let rest = rest.trim();
    if rest.starts_with('"') {
        let end = rest[1..].find('"')?;
        return Some(&rest[1..1 + end]);
    }
    if rest.starts_with('<') {
        let end = rest[1..].find('>')?;
        return Some(&rest[1..1 + end]);
    }
    None
}

/// Resolve a URI path segment against gamedir (mirrors `engine_resource::resolve_content_path`).
fn resolve_content_path(base_dir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    if path.len() >= 2 && path.as_bytes().get(1) == Some(&b':') {
        return PathBuf::from(path);
    }
    base_dir.join(path)
}

fn resolve_include_path(base: &Path, rel: &str) -> Result<PathBuf, ShaderError> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Err(ShaderError::IncludeSyntax("empty path".into()));
    }
    let mut out = PathBuf::from(base);
    for comp in Path::new(rel).components() {
        match comp {
            Component::ParentDir => {
                if !out.pop() {
                    return Err(ShaderError::IncludePathEscape(rel.into()));
                }
            }
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {}
            _ => return Err(ShaderError::IncludePathEscape(rel.into())),
        }
    }
    Ok(out)
}

/// Parse `#@shader pass=mesh_opaque queue=1000` → fill ShaderMeta.
fn parse_shader_line(rest: &str, meta: &mut ShaderMeta) -> Result<(), ShaderError> {
    for token in rest.split_whitespace() {
        if token == "skinned" {
            meta.skinned = true;
            continue;
        }
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
                "skinned" => {
                    meta.skinned = matches!(val.trim(), "1" | "true" | "yes");
                }
                _ => return Err(ShaderError::UnknownShaderKey(key.to_string())),
            }
        }
    }
    Ok(())
}

/// Parse `#@prop _BaseColor color [1,1,1,1]` → (name, kind, default).
fn parse_prop_line(
    rest: &str,
) -> Result<Option<(String, PropertyKind, PropDefault)>, ShaderError> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Ok(None);
    }

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
            let name = s.trim_matches('"').to_string();
            if name.is_empty() {
                Some(PropDefault::Texture("white".into()))
            } else {
                Some(PropDefault::Texture(name))
            }
        }
    }
}

fn parse_float_array(s: &str, n: usize) -> Option<Vec<f32>> {
    let s = s.trim().trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
    if parts.len() != n {
        return None;
    }
    parts.iter().map(|p| p.parse::<f32>().ok()).collect()
}

fn extract_binding_index(line: &str) -> Option<u32> {
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
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TMP_SEQ: AtomicU32 = AtomicU32::new(0);

    fn temp_shader_dir() -> PathBuf {
        let n = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("deerpunk_shader_test_{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_skinned_flag() {
        let src = r#"#@shader pass=mesh_opaque queue=1000 skinned
@group(0) @binding(0) var<uniform> u: f32;
"#;
        let result = ShaderPreprocessor::process(src, &[]).unwrap();
        assert!(result.meta.skinned);
    }

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

    #[test]
    fn include_inlines_once() {
        let dir = temp_shader_dir();
        fs::write(dir.join("common.wgsl"), "const FOO: f32 = 1.0;\n").unwrap();
        let root = r#"
#include "common.wgsl"
#include "common.wgsl"
fn main() {}
"#;
        let out = ShaderPreprocessor::process_at(root, &[], Some(&dir))
            .unwrap()
            .wgsl;
        assert_eq!(out.matches("const FOO").count(), 1);
        assert!(out.contains("fn main"));
    }

    #[test]
    fn include_nested_and_relative() {
        let dir = temp_shader_dir();
        fs::create_dir_all(dir.join("inc")).unwrap();
        fs::write(dir.join("inc/a.wgsl"), "#include \"../b.wgsl\"\n").unwrap();
        fs::write(dir.join("b.wgsl"), "const B: f32 = 2.0;\n").unwrap();
        let root = "#include \"inc/a.wgsl\"\n";
        let out = ShaderPreprocessor::process_at(root, &[], Some(&dir))
            .unwrap()
            .wgsl;
        assert!(out.contains("const B"));
    }

    #[test]
    fn include_cycle_errors() {
        let dir = temp_shader_dir();
        fs::write(dir.join("a.wgsl"), "#include \"b.wgsl\"\n").unwrap();
        fs::write(dir.join("b.wgsl"), "#include \"a.wgsl\"\n").unwrap();
        let err = ShaderPreprocessor::process_at("#include \"a.wgsl\"\n", &[], Some(&dir))
            .err()
            .expect("expected cycle error");
        assert!(matches!(err, ShaderError::IncludeCycle(_)));
    }

    #[test]
    fn include_no_base_errors() {
        let err = ShaderPreprocessor::process("#include \"x.wgsl\"\n", &[])
            .err()
            .expect("expected no-base error");
        assert!(matches!(err, ShaderError::IncludeNoBase));
    }

    #[test]
    fn shader_include_base_gamedir_relative() {
        let dir = temp_shader_dir();
        fs::create_dir_all(dir.join("shaders/inc")).unwrap();
        fs::write(dir.join("shaders/foo.wgsl"), "").unwrap();
        let base = ShaderPreprocessor::shader_include_base(&dir, "file://shaders/foo.wgsl")
            .unwrap()
            .canonicalize()
            .unwrap();
        let expect = dir.join("shaders").canonicalize().unwrap();
        assert_eq!(base, expect);
    }
}
