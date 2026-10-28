//! Runtime asset registry, mesh handles, and built-in loaders under [`loaders`].

pub mod loaders;
pub mod loader;
pub mod mesh;
pub mod mesh_allocator;
pub mod mesh_content;
pub mod mesh_handle;
pub mod mesh_registry;
pub mod wad;

use loader::Loader;
use mesh::Mesh;
pub use mesh_allocator::{
    ForeignMeshAllocator, MeshAllocHandle, MeshAllocator, MeshAllocatorRegistration,
    MeshAllocatorVTable,
};
pub use mesh_handle::MeshHandle;
pub use mesh_registry::MeshRegistry;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use loader::{LoadOpts, LoadedAsset};
pub use loaders::{
    fallback_name_to_texgen_uri, register_default_loaders, AudioClip, FileLoader, ShaderLoader,
    ShaderSource, TexgenLoader, WadLoader,
};
/// OBJ helpers used by scene prefab appliers (MTL diffuse lookup).
pub use loaders::obj;

/// Stable id for a loaded (or failed) asset. Survives aliases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssetId(pub u64);

enum ResourceState {
    Pending,
    Loaded(Box<dyn Any + Send>),
    Failed(String),
}

pub struct ResourceHandle<T> {
    id: AssetId,
    inner: Arc<Mutex<ResourceState>>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Clone + 'static> ResourceHandle<T> {
    pub fn id(&self) -> AssetId {
        self.id
    }

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
            id: self.id,
            inner: self.inner.clone(),
            _phantom: std::marker::PhantomData,
        }
    }
}

struct AssetRecord {
    id: AssetId,
    uri: String,
    aliases: Vec<String>,
    type_name: String,
    size: u64,
    state: Arc<Mutex<ResourceState>>,
}

/// Inspector / debug row.
#[derive(Clone, Debug)]
pub struct AssetInfo {
    pub id: AssetId,
    pub uri: String,
    pub aliases: Vec<String>,
    pub type_name: String,
    pub size: u64,
    pub loaded: bool,
}

struct ResourceManagerInner {
    loaders: HashMap<String, Arc<dyn Loader>>,
    extensions: HashMap<String, String>,
    keys: HashMap<String, AssetId>,
    assets: HashMap<AssetId, AssetRecord>,
    next_id: u64,
    base_dir: PathBuf,
    fallback_mesh: Option<Mesh>,
}

/// Asset cache + loader registry. All mutation is interior (`RefCell`); use `&self` for [`load`].
pub struct ResourceManager {
    inner: RefCell<ResourceManagerInner>,
}

impl ResourceManager {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(ResourceManagerInner {
                loaders: HashMap::new(),
                extensions: HashMap::new(),
                keys: HashMap::new(),
                assets: HashMap::new(),
                next_id: 1,
                base_dir: PathBuf::new(),
                fallback_mesh: None,
            }),
        }
    }

    pub fn set_base_dir(&self, dir: impl Into<PathBuf>) {
        let dir = dir.into();
        let mut inner = self.inner.borrow_mut();
        inner.base_dir = dir;
        let root = inner.base_dir.clone();
        if inner.loaders.contains_key("wad") {
            Self::register_loader_inner(
                &mut inner,
                Box::new(loaders::WadLoader { root: root.clone() }),
            );
        }
        if inner.loaders.contains_key("shader") {
            Self::register_loader_inner(
                &mut inner,
                Box::new(loaders::ShaderLoader { root: root.clone() }),
            );
        }
        if inner.loaders.contains_key("file") {
            Self::register_loader_inner(&mut inner, Box::new(loaders::FileLoader { root }));
        }
    }

    pub fn base_dir(&self) -> PathBuf {
        self.inner.borrow().base_dir.clone()
    }

    /// Drop a cached asset so the next [`load`] re-reads from disk.
    pub fn evict(&self, uri: &str) {
        let mut inner = self.inner.borrow_mut();
        let Some(id) = inner.keys.remove(uri) else {
            return;
        };
        if let Some(rec) = inner.assets.remove(&id) {
            inner.keys.remove(&rec.uri);
            for alias in &rec.aliases {
                inner.keys.remove(alias);
            }
        }
    }

    pub fn register_loader(&self, loader: Box<dyn Loader>) {
        let mut inner = self.inner.borrow_mut();
        Self::register_loader_inner(&mut inner, loader);
    }

    fn register_loader_inner(inner: &mut ResourceManagerInner, loader: Box<dyn Loader>) {
        let id = loader.id().to_string();
        for ext in loader.extensions() {
            inner
                .extensions
                .insert(ext.to_ascii_lowercase(), id.clone());
        }
        inner.loaders.insert(id, Arc::from(loader));
    }

    pub fn set_fallback_mesh(&self, mesh: Mesh) {
        self.inner.borrow_mut().fallback_mesh = Some(mesh);
    }

    /// Load with default options (extension / URI protocol selects loader).
    pub fn load<T: Clone + Send + 'static>(&self, uri: &str) -> ResourceHandle<T> {
        self.load_with(uri, LoadOpts::default())
    }

    /// Load with explicit loader / alias. Re-entrant: loaders may call `load` again on `&self`.
    pub fn load_with<T: Clone + Send + 'static>(
        &self,
        uri: &str,
        opts: LoadOpts,
    ) -> ResourceHandle<T> {
        let key = uri.to_string();

        if let Some(handle) = self.lookup_cached::<T>(&key, &opts) {
            return handle;
        }

        {
            let mut inner = self.inner.borrow_mut();
            if let Some(&id) = inner.keys.get(&key) {
                let failed = inner
                    .assets
                    .get(&id)
                    .map(|rec| matches!(*rec.state.lock().unwrap(), ResourceState::Failed(_)))
                    .unwrap_or(false);
                if failed {
                    inner.keys.remove(&key);
                    inner.assets.remove(&id);
                }
            }
        }

        let (id, state, loader, path, query, fallback_mesh) = {
            let mut inner = self.inner.borrow_mut();
            let id = AssetId(inner.next_id);
            inner.next_id += 1;

            let state = Arc::new(Mutex::new(ResourceState::Pending));
            let (protocol, path, query) = parse_uri(uri);
            let loader_id = Self::resolve_loader_id_inner(&inner, &opts, &protocol, &path, &query);
            let loader = inner.loaders.get(&loader_id).cloned();
            let fallback_mesh = inner.fallback_mesh.clone();

            let mut aliases = Vec::new();
            if let Some(alias) = opts.alias.clone() {
                aliases.push(alias.clone());
                inner.keys.insert(alias, id);
            }
            inner.keys.insert(key.clone(), id);
            inner.assets.insert(
                id,
                AssetRecord {
                    id,
                    uri: key.clone(),
                    aliases,
                    type_name: "pending".into(),
                    size: 0,
                    state: state.clone(),
                },
            );

            (id, state, loader, path, query, fallback_mesh)
        };

        let load_result = loader.as_ref().map(|l| l.load(&path, &query));

        let (type_name, size) = match load_result {
            None => {
                let msg = "no loader for resolved id";
                *state.lock().unwrap() = ResourceState::Failed(msg.to_string());
                ("unknown".to_string(), 0u64)
            }
            Some(Err(msg)) => {
                eprintln!("[resource] failed to load {uri}: {msg}");
                if let Some(fb) = &fallback_mesh {
                    if std::any::TypeId::of::<T>() == std::any::TypeId::of::<Mesh>() {
                        *state.lock().unwrap() = ResourceState::Loaded(Box::new(fb.clone()));
                        ("mesh".into(), 0)
                    } else {
                        *state.lock().unwrap() = ResourceState::Failed(msg);
                        ("unknown".into(), 0)
                    }
                } else {
                    *state.lock().unwrap() = ResourceState::Failed(msg);
                    ("unknown".into(), 0)
                }
            }
            Some(Ok(LoadedAsset {
                data,
                size: sz,
                type_name: tn,
            })) => {
                *state.lock().unwrap() = ResourceState::Loaded(data);
                (tn.to_string(), sz)
            }
        };

        {
            let mut inner = self.inner.borrow_mut();
            if let Some(rec) = inner.assets.get_mut(&id) {
                rec.type_name = type_name;
                rec.size = size;
            }
        }

        ResourceHandle {
            id,
            inner: state,
            _phantom: std::marker::PhantomData,
        }
    }

    fn lookup_cached<T: Clone + Send + 'static>(
        &self,
        key: &str,
        opts: &LoadOpts,
    ) -> Option<ResourceHandle<T>> {
        let inner = self.inner.borrow();
        let &id = inner.keys.get(key)?;
        let failed = inner
            .assets
            .get(&id)
            .map(|rec| matches!(*rec.state.lock().unwrap(), ResourceState::Failed(_)))
            .unwrap_or(false);
        if failed {
            return None;
        }
        if let Some(alias) = opts.alias.as_ref() {
            drop(inner);
            self.bind_alias(id, alias);
        } else {
            drop(inner);
        }
        let inner = self.inner.borrow();
        let rec = inner.assets.get(&id)?;
        Some(ResourceHandle {
            id,
            inner: rec.state.clone(),
            _phantom: std::marker::PhantomData,
        })
    }

    /// Bind an extra key to an existing asset.
    pub fn bind_alias(&self, id: AssetId, alias: impl Into<String>) {
        let alias = alias.into();
        let mut inner = self.inner.borrow_mut();
        if let Some(rec) = inner.assets.get_mut(&id) {
            if !rec.aliases.iter().any(|a| a == &alias) && rec.uri != alias {
                rec.aliases.push(alias.clone());
            }
        }
        inner.keys.insert(alias, id);
    }

    /// Look up a previously loaded asset by id.
    pub fn handle<T: Clone + Send + 'static>(&self, id: AssetId) -> Option<ResourceHandle<T>> {
        let inner = self.inner.borrow();
        let rec = inner.assets.get(&id)?;
        Some(ResourceHandle {
            id,
            inner: rec.state.clone(),
            _phantom: std::marker::PhantomData,
        })
    }

    pub fn list_assets(&self) -> Vec<AssetInfo> {
        let inner = self.inner.borrow();
        let mut out: Vec<AssetInfo> = inner
            .assets
            .values()
            .map(|r| {
                let loaded = matches!(*r.state.lock().unwrap(), ResourceState::Loaded(_));
                AssetInfo {
                    id: r.id,
                    uri: r.uri.clone(),
                    aliases: r.aliases.clone(),
                    type_name: r.type_name.clone(),
                    size: r.size,
                    loaded,
                }
            })
            .collect();
        out.sort_by_key(|a| a.id.0);
        out
    }

    fn resolve_loader_id_inner(
        inner: &ResourceManagerInner,
        opts: &LoadOpts,
        protocol: &str,
        path: &str,
        query: &str,
    ) -> String {
        if let Some(id) = opts.loader {
            return id.to_string();
        }
        if file_extension(path).as_deref() == Some("bsp") && inner.loaders.contains_key("bsp") {
            return "bsp".to_string();
        }
        if file_extension(path).as_deref() == Some("wgsl") && inner.loaders.contains_key("shader") {
            return "shader".to_string();
        }
        if file_extension(path).as_deref() == Some("mdl") && inner.loaders.contains_key("studio") {
            return "studio".to_string();
        }
        if let Some(kind) = query.split(':').next() {
            if !kind.is_empty() && inner.loaders.contains_key(kind) {
                return kind.to_string();
            }
        }
        if let Some(ext) = file_extension(path) {
            if let Some(id) = inner.extensions.get(&ext) {
                return id.clone();
            }
        }
        protocol.to_string()
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve a URI path segment against gamedir (or use absolute path as-is).
pub fn resolve_content_path(base_dir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    if path.len() >= 2 && path.as_bytes().get(1) == Some(&b':') {
        return PathBuf::from(path);
    }
    base_dir.join(path)
}

fn file_extension(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() {
        None
    } else {
        Some(ext.to_ascii_lowercase())
    }
}

fn parse_uri(uri: &str) -> (String, String, String) {
    let (protocol, rest) = match uri.split_once("://") {
        Some((p, r)) => (p.to_string(), r),
        None => ("file".to_string(), uri),
    };
    let (path_and_query, fragment) = match rest.split_once('#') {
        Some((pq, frag)) => (pq, Some(frag)),
        None => (rest, None),
    };
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (path_and_query.to_string(), String::new()),
    };
    let path = path.trim_end_matches('/').to_string();
    let query = match fragment {
        Some(f) if query.is_empty() => f.to_string(),
        Some(f) => format!("{query}&{f}"),
        None => query,
    };
    (protocol, path, query)
}

/// Convert a filesystem path to a `file://` URI (slashes normalized).
pub fn path_to_file_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    if s.starts_with("file://") {
        s
    } else {
        format!("file://{s}")
    }
}
