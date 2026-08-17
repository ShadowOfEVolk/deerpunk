use std::any::Any;

pub trait Loader: Send + Sync {
    fn protocol(&self) -> &'static str;
    fn load(&self, uri: &str, query: &str) -> Result<Box<dyn Any + Send>, String>;
}

pub struct PrefabLoader;

impl Loader for PrefabLoader {
    fn protocol(&self) -> &'static str {
        "prefab"
    }

    fn load(&self, path: &str, _query: &str) -> Result<Box<dyn Any + Send>, String> {
        let template = crate::prefab::PrefabTemplate::load(path);
        Ok(Box::new(template))
    }
}