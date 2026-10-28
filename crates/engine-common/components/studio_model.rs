//! GoldSrc-style studio instance state (bodygroups + skin).
//!
//! - `.mdl` path: [`MapKv`] key `model` → [`crate::scene::sync_studio_presentation`] sets `Mesh3D.uri`.
//! - Shader/material: [`crate::material::DrawMaterial`] on the entity (prefab / `materials.load`).
//! - Flecs: [`crate::scene::register_studio_present_observers`] on `StudioModel` add / `MapKv` set.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StudioModel {
    pub body: u32,
    pub skin: u32,
}

impl crate::layout_meta::LayoutDescribe for StudioModel {
    fn layout_meta() -> crate::layout_meta::LayoutMeta {
        crate::layout_meta::studio_model_layout()
    }
}
