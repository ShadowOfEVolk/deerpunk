//! Client-side scene assembly before [`RenderSnapshot`] (extract → snapshot → GPU passes).

pub mod sky_compose;

pub use sky_compose::{
    apply_compose, fill_procedural_sky_default, smoothstep_sky, ClientComposeSkyFn,
    SkySceneCompose,
};
