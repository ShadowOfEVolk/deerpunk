pub trait AppApi {
    fn frame_dt(&self) -> f32;
    fn frame_number(&self) -> u64;
    fn uptime_secs(&self) -> f32;
    fn request_quit(&self);
}
