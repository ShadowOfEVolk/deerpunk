pub mod loader;
pub mod mesh;
pub mod obj;
pub mod fbx;
pub mod transform;

use loader::Loader;
use mesh::Mesh;
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

enum ResourceState {
    Pending,
    Loaded(Box<dyn Any + Send>),
    Failed(String),
}

pub struct ResourceHandle<T> {
    inner: Arc<Mutex<ResourceState>>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Clone + 'static> ResourceHandle<T> {
    pub fn is_loaded(&self) -> bool {
        matches!(*self.inner.lock().unwrap(), ResourceState::Loaded(_))
    }

    pub fn is_failed(&self) -> bool {
        matches!(*self.inner.lock().unwrap(), ResourceState::Failed(_))
    }

    pub fn unwrap(&self) -> T {
        let state = self.inner.lock().unwrap();
        match &*state {
            ResourceState::Loaded(data) => data.downcast_ref::<T>().expect("type mismatch").clone(),
            ResourceState::Pending => panic!("resource not loaded yet"),
            ResourceState::Failed(msg) => panic!("resource failed: {msg}"),
        }
    }

    pub fn try_get(&self) -> Option<T> {
        let state = self.inner.lock().unwrap();
        match &*state {
            ResourceState::Loaded(data) => data.downcast_ref::<T>().cloned(),
            _ => None,
        }
    }
}

impl<T> Clone for ResourceHandle<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _phantom: std::marker::PhantomData,
        }
    }
}

pub struct ResourceManager {
    loaders: HashMap<&'static str, Box<dyn Loader>>,
    cache: HashMap<String, Arc<Mutex<ResourceState>>>,
    fallback_mesh: Option<Mesh>,
}

impl ResourceManager {
    pub fn new() -> Self {
        Self {
            loaders: HashMap::new(),
            cache: HashMap::new(),
            fallback_mesh: None,
        }
    }

    pub fn register_loader(&mut self, loader: Box<dyn Loader>) {
        self.loaders.insert(loader.protocol(), loader);
    }

    pub fn set_fallback_mesh(&mut self, mesh: Mesh) {
        self.fallback_mesh = Some(mesh);
    }

    pub fn load<T: Clone + Send + 'static>(&mut self, uri: &str) -> ResourceHandle<T> {
        let key = uri.to_string();

        if let Some(existing) = self.cache.get(&key) {
            return ResourceHandle {
                inner: existing.clone(),
                _phantom: std::marker::PhantomData,
            };
        }

        let state = Arc::new(Mutex::new(ResourceState::Pending));
        self.cache.insert(key.clone(), state.clone());

        let (protocol, path, query) = parse_uri(uri);

        if let Some(loader) = self.loaders.get(protocol.as_str()) {
            match loader.load(&path, &query) {
                Ok(data) => {
                    *state.lock().unwrap() = ResourceState::Loaded(data);
                }
                Err(msg) => {
                    eprintln!("[resource] failed to load {uri}: {msg}");
                    if let Some(fb) = &self.fallback_mesh {
                        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<Mesh>() {
                            *state.lock().unwrap() =
                                ResourceState::Loaded(Box::new(fb.clone()));
                        } else {
                            *state.lock().unwrap() = ResourceState::Failed(msg);
                        }
                    } else {
                        *state.lock().unwrap() = ResourceState::Failed(msg);
                    }
                }
            }
        } else {
            *state.lock().unwrap() = ResourceState::Failed(format!("no loader for protocol: {protocol}"));
        }

        ResourceHandle {
            inner: state,
            _phantom: std::marker::PhantomData,
        }
    }
}

fn parse_uri(uri: &str) -> (String, String, String) {
    let (protocol, rest) = match uri.split_once("://") {
        Some((p, r)) => (p.to_string(), r),
        None => ("file".to_string(), uri),
    };
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (rest.to_string(), String::new()),
    };
    let path = path.trim_end_matches('/').to_string();
    (protocol, path, query)
}

pub struct FileLoader;

impl Loader for FileLoader {
    fn protocol(&self) -> &'static str {
        "file"
    }

    fn load(&self, path: &str, query: &str) -> Result<Box<dyn Any + Send>, String> {
        let mut mesh = if path.ends_with(".obj") {
            let data = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {path}: {e}"))?;
            obj::parse_obj(&data)?
        } else if path.ends_with(".fbx") {
            let data = std::fs::read(path)
                .map_err(|e| format!("cannot read {path}: {e}"))?;
            fbx::parse_fbx(&data)?
        } else {
            return Err(format!("unrecognised file extension: {path}"));
        };
        let transform = transform::parse_mesh_transform(query);
        transform::apply_mesh_transform(&mut mesh, &transform);
        Ok(Box::new(mesh))
    }
}