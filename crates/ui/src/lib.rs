//! deerpunk-ui — RmlUi documents as first-class UI units (Panorama-style).
//!
//! One ECS entity ≈ one `UiDocument` (`.rml` root). Buttons live inside the
//! document tree, not as separate Flecs entities.
//!
//! Assets are resolved relative to a UI base path (typically `{gamedir}/ui/`).

mod assets;
mod draw_list;
mod ffi;
mod runtime;

pub use assets::{UiAssetSource, UiDirAssets, register_ui_loaders};
pub use deerpunk_engine_common::ui_draw::{UiFrame, UiVertex};
pub use runtime::{
    drain_commands, set_client_in_game, set_content_root, UiDocument, UiRuntime,
};

pub mod paths {
    pub const WIDGETS: &str = "widgets/";
    pub const STYLES: &str = "styles/";
    pub const FONTS: &str = "fonts/";
}
