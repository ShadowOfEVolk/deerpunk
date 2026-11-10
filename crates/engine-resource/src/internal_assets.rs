//! Read-only assets embedded in the engine binary (`internal://…` URIs).

const FILES: &[(&str, &str)] = &[
    (
        "shaders/inc/dither.wgsl",
        include_str!("../assets/internal/shaders/inc/dither.wgsl"),
    ),
    (
        "shaders/inc/mesh_uniform_slot.wgsl",
        include_str!("../assets/internal/shaders/inc/mesh_uniform_slot.wgsl"),
    ),
    (
        "shaders/particles/billboard.wgsl",
        include_str!("../assets/internal/shaders/particles/billboard.wgsl"),
    ),
];

/// Normalize `internal://` path: forward slashes, no leading `/`.
pub fn normalize_internal_path(path: &str) -> String {
    path.trim()
        .trim_start_matches('/')
        .replace('\\', "/")
}

/// Lookup embedded file by path (without `internal://` prefix).
pub fn internal_asset(path: &str) -> Option<&'static str> {
    let key = normalize_internal_path(path);
    FILES
        .iter()
        .find(|(k, _)| *k == key.as_str())
        .map(|(_, text)| *text)
}

/// All registered internal paths (for tooling / tests).
pub fn internal_asset_paths() -> impl Iterator<Item = &'static str> {
    FILES.iter().map(|(k, _)| *k)
}

#[cfg(test)]
mod tests {
    use super::{internal_asset, internal_asset_paths, normalize_internal_path};

    #[test]
    fn dither_embedded() {
        let s = internal_asset("shaders/inc/dither.wgsl").expect("dither");
        assert!(s.contains("dither_threshold"));
    }

    #[test]
    fn paths_listed() {
        let paths: Vec<_> = internal_asset_paths().collect();
        assert!(paths.contains(&"shaders/inc/dither.wgsl"));
    }

    #[test]
    fn normalize_strips_slashes() {
        assert_eq!(
            normalize_internal_path("/shaders/inc/dither.wgsl"),
            "shaders/inc/dither.wgsl"
        );
    }
}
