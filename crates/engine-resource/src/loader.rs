use std::any::Any;

/// Options for [`super::ResourceManager::load_with`].
#[derive(Clone, Debug, Default)]
pub struct LoadOpts {
    /// Explicit loader id (`"bsp"`, `"file"`, …). Overrides extension / URI protocol.
    pub loader: Option<&'static str>,
    /// Extra key that resolves to the same [`super::AssetId`].
    pub alias: Option<String>,
}

impl LoadOpts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn loader(mut self, id: &'static str) -> Self {
        self.loader = Some(id);
        self
    }

    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }
}

/// Result of a successful loader invocation.
pub struct LoadedAsset {
    pub data: Box<dyn Any + Send>,
    /// Approximate payload size in bytes (file size or decoded buffer).
    pub size: u64,
    /// Stable type tag for inspector (`"mesh"`, `"bsp"`, `"rgba"`, …).
    pub type_name: &'static str,
}

pub trait Loader: Send + Sync {
    /// Registry id (also used as URI protocol when no extension match).
    fn id(&self) -> &str;

    /// File extensions without dot (`"bsp"`, `"obj"`). Empty = protocol-only.
    fn extensions(&self) -> &'static [&'static str] {
        &[]
    }

    fn load(&self, path: &str, query: &str) -> Result<LoadedAsset, String>;
}
