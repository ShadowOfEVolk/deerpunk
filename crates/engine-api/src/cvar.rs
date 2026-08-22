pub trait CvarApi {
    fn get_f32(&self, name: &str) -> Option<f32>;
    fn set(&self, name: &str, value: &str) -> bool;
}
