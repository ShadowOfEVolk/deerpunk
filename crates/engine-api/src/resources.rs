pub trait ResourcesApi {
    fn load_bytes(&self, path: &str) -> Option<Vec<u8>>;
    fn load_text(&self, path: &str) -> Option<String>;
}
