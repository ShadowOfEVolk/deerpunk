//! Graph resource name constants.

pub mod res {
    use crate::render_graph::ResId;

    pub const HDR: ResId = ResId("hdr_color");
    pub const DEPTH: ResId = ResId("depth");
    pub const SURFACE: ResId = ResId("surface");
}
