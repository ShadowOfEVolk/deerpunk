use std::any::Any;

pub trait Loader: Send + Sync {
    fn protocol(&self) -> &'static str;
    fn load(&self, uri: &str, query: &str) -> Result<Box<dyn Any + Send>, String>;
}
