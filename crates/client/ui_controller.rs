//! Client-side UI widget list (not ECS). Load via UI assets / RmlUi; show/hide toggles visibility.

use std::collections::HashMap;

use deerpunk_ui::{UiDocument, UiRuntime};

struct Widget {
    /// Path used for load, e.g. `widgets/main_menu.rml`
    path: String,
    doc: UiDocument,
    visible: bool,
}

/// Manages loaded RmlUi documents. Id is a short name (`main_menu`) or relative path.
pub struct UiController {
    widgets: HashMap<String, Widget>,
}

impl UiController {
    pub fn new() -> Self {
        Self {
            widgets: HashMap::new(),
        }
    }

    /// Resolve id → asset path under `{gamedir}/ui`.
    pub fn resolve_path(id: &str) -> String {
        let id = id.trim().trim_start_matches('/');
        if id.is_empty() {
            return String::new();
        }
        if id.contains('/') || id.ends_with(".rml") {
            return id.to_string();
        }
        format!("widgets/{id}.rml")
    }

    /// Load document asset into the widget list (hidden until [`Self::show`]).
    /// Idempotent if already loaded with the same path.
    pub fn load(&mut self, ui: &UiRuntime, id: &str) -> Result<(), String> {
        let path = Self::resolve_path(id);
        if path.is_empty() {
            return Err("empty widget id".into());
        }
        let key = id.trim().to_string();
        if let Some(w) = self.widgets.get(&key) {
            if w.path == path {
                return Ok(());
            }
            self.widgets.remove(&key);
        }
        let doc = ui
            .load_document(&path)
            .ok_or_else(|| format!("failed to load ui document: {path}"))?;
        log::info!("[ui] load id={key} path={path}");
        self.widgets.insert(
            key,
            Widget {
                path,
                doc,
                visible: false,
            },
        );
        Ok(())
    }

    pub fn unload(&mut self, id: &str) -> Result<(), String> {
        let key = id.trim();
        if self.widgets.remove(key).is_some() {
            log::info!("[ui] unload id={key}");
            Ok(())
        } else {
            Err(format!("widget not loaded: {key}"))
        }
    }

    /// Close every loaded document (before RmlUi shutdown).
    pub fn clear(&mut self) {
        if self.widgets.is_empty() {
            return;
        }
        log::info!("[ui] clear {} widgets", self.widgets.len());
        self.widgets.clear();
    }

    pub fn show(&mut self, id: &str) -> Result<(), String> {
        let key = id.trim();
        let w = self
            .widgets
            .get_mut(key)
            .ok_or_else(|| format!("widget not loaded: {key}"))?;
        w.doc.show();
        w.visible = true;
        log::info!("[ui] show id={key}");
        Ok(())
    }

    pub fn hide(&mut self, id: &str) -> Result<(), String> {
        let key = id.trim();
        let w = self
            .widgets
            .get_mut(key)
            .ok_or_else(|| format!("widget not loaded: {key}"))?;
        w.doc.hide();
        w.visible = false;
        log::info!("[ui] hide id={key}");
        Ok(())
    }

    pub fn is_loaded(&self, id: &str) -> bool {
        self.widgets.contains_key(id.trim())
    }

    pub fn is_visible(&self, id: &str) -> bool {
        self.widgets
            .get(id.trim())
            .map(|w| w.visible)
            .unwrap_or(false)
    }
}

impl Default for UiController {
    fn default() -> Self {
        Self::new()
    }
}
