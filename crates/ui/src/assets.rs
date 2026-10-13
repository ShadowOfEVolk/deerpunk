use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use deerpunk_engine_common::resource::{loader::Loader, ResourceManager};

static UI_BASE: OnceLock<Mutex<PathBuf>> = OnceLock::new();

fn base_lock() -> &'static Mutex<PathBuf> {
    UI_BASE.get_or_init(|| Mutex::new(PathBuf::new()))
}

/// Set the root directory for RmlUi file resolves (usually `{gamedir}/ui`).
pub fn set_ui_base_path(path: impl Into<PathBuf>) {
    *base_lock().lock().unwrap() = path.into();
}

pub fn ui_base_path() -> PathBuf {
    base_lock().lock().unwrap().clone()
}

/// Read bytes for a UI-relative path (`widgets/hello.rml`, `fonts/…`).
pub trait UiAssetSource: Send + Sync {
    fn read(&self, relative: &str) -> Result<Vec<u8>, String>;
    fn base(&self) -> &Path;
}

/// Simple directory-backed source under `{base}/`.
pub struct UiDirAssets {
    base: PathBuf,
}

impl UiDirAssets {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        let base = base.into();
        // Strip \\?\ so RmlUi/C++ and path joins stay consistent on Windows.
        let base = {
            let s = base.to_string_lossy();
            if let Some(rest) = s.strip_prefix(r"\\?\") {
                PathBuf::from(rest)
            } else {
                base
            }
        };
        log::info!(target: "ui", "asset base = {}", base.display());
        set_ui_base_path(base.clone());
        Self { base }
    }
}

impl UiAssetSource for UiDirAssets {
    fn read(&self, relative: &str) -> Result<Vec<u8>, String> {
        let path = self.base.join(relative);
        std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn base(&self) -> &Path {
        &self.base
    }
}

/// `ui://widgets/hello.rml` → load text/bytes from UI base via ResourceManager.
pub struct UiProtocolLoader;

impl Loader for UiProtocolLoader {
    fn id(&self) -> &'static str {
        "ui"
    }

    fn load(&self, path: &str, _query: &str) -> Result<deerpunk_engine_common::resource::LoadedAsset, String> {
        use deerpunk_engine_common::resource::LoadedAsset;
        let base = ui_base_path();
        if base.as_os_str().is_empty() {
            return Err("ui base path not set (call UiDirAssets::new / set_ui_base_path)".into());
        }
        let full = base.join(path);
        let data = std::fs::read(&full).map_err(|e| format!("{}: {e}", full.display()))?;
        let size = data.len() as u64;
        if path.ends_with(".rml") || path.ends_with(".rcss") || path.ends_with(".lua") {
            let text = String::from_utf8(data).map_err(|e| e.to_string())?;
            Ok(LoadedAsset {
                data: Box::new(text),
                size,
                type_name: "ui_text",
            })
        } else {
            Ok(LoadedAsset {
                data: Box::new(data),
                size,
                type_name: "ui_bytes",
            })
        }
    }
}

pub fn register_ui_loaders(rm: &mut ResourceManager) {
    rm.register_loader(Box::new(UiProtocolLoader));
}
