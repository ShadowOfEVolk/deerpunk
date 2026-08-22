pub trait RendererApi {
    fn frame_width(&self) -> u32;
    fn frame_height(&self) -> u32;
}
