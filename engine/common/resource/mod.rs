pub mod bsp_loader;
pub mod loader;
pub mod mesh;
pub mod obj;
pub mod fbx;
pub mod transform;
pub mod wad_loader;

use loader::Loader;
use mesh::Mesh;
use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use bsp_loader::BspLoader;
pub use loader::{LoadOpts, LoadedAsset};

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

pub struct ResourceManager {
    loaders: HashMap<&'static str, Box<dyn Loader>>,
    /// extension (no dot, lowercase) → loader id
    extensions: HashMap<String, &'static str>,
    /// uri / alias → AssetId
    keys: HashMap<String, AssetId>,
    assets: HashMap<AssetId, AssetRecord>,
    next_id: u64,
    base_dir: PathBuf,
    fallback_mesh: Option<Mesh>,
}

impl ResourceManager {
    pub fn new() -> Self {
        Self {
            loaders: HashMap::new(),
            extensions: HashMap::new(),
            keys: HashMap::new(),
            assets: HashMap::new(),
            next_id: 1,
            base_dir: PathBuf::new(),
            fallback_mesh: None,
        }
    }

    pub fn set_base_dir(&mut self, dir: impl Into<PathBuf>) {
        self.base_dir = dir.into();
        // Refresh loaders that embed gamedir.
        if self.loaders.contains_key("bsp") {
            self.register_loader(Box::new(BspLoader::new(&self.base_dir)));
        }
        if self.loaders.contains_key("wad") {
            self.register_loader(Box::new(wad_loader::WadLoader {
                root: self.base_dir.clone(),
            }));
        }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn register_loader(&mut self, loader: Box<dyn Loader>) {
        let id = loader.id();
        for ext in loader.extensions() {
            self.extensions
                .insert(ext.to_ascii_lowercase(), id);
        }
        self.loaders.insert(id, loader);
    }

    pub fn set_fallback_mesh(&mut self, mesh: Mesh) {
        self.fallback_mesh = Some(mesh);
    }

    /// Load with default options (extension / URI protocol selects loader).
    pub fn load<T: Clone + Send + 'static>(&mut self, uri: &str) -> ResourceHandle<T> {
        self.load_with(uri, LoadOpts::default())
    }

    /// Load with explicit loader / alias.
    pub fn load_with<T: Clone + Send + 'static>(
        &mut self,
        uri: &str,
        opts: LoadOpts,
    ) -> ResourceHandle<T> {
        let key = uri.to_string();

        if let Some(&id) = self.keys.get(&key) {
            if let Some(alias) = opts.alias.as_ref() {
                self.bind_alias(id, alias);
            }
            let rec = self.assets.get(&id).expect("asset missing for key");
            return ResourceHandle {
                id,
                inner: rec.state.clone(),
                _phantom: std::marker::PhantomData,
            };
        }

        let id = AssetId(self.next_id);
        self.next_id += 1;

        let state = Arc::new(Mutex::new(ResourceState::Pending));
        let (protocol, path, query) = parse_uri(uri);
        let loader_id = self.resolve_loader_id(&opts, &protocol, &path);

        let mut type_name = "unknown".to_string();
        let mut size = 0u64;

        if let Some(loader) = self.loaders.get(loader_id.as_str()) {
            match loader.load(&path, &query) {
                Ok(LoadedAsset {
                    data,
                    size: sz,
                    type_name: tn,
                }) => {
                    size = sz;
                    type_name = tn.to_string();
                    *state.lock().unwrap() = ResourceState::Loaded(data);
                }
                Err(msg) => {
                    eprintln!("[resource] failed to load {uri}: {msg}");
                    if let Some(fb) = &self.fallback_mesh {
                        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<Mesh>() {
                            size = 0;
                            type_name = "mesh".into();
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
            *state.lock().unwrap() =
                ResourceState::Failed(format!("no loader for id: {loader_id}"));
        }

        let mut aliases = Vec::new();
        if let Some(alias) = opts.alias.clone() {
            aliases.push(alias.clone());
            self.keys.insert(alias, id);
        }

        self.keys.insert(key.clone(), id);
        self.assets.insert(
            id,
            AssetRecord {
                id,
                uri: key,
                aliases,
                type_name,
                size,
                state: state.clone(),
            },
        );

        ResourceHandle {
            id,
            inner: state,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Bind an extra key to an existing asset.
    pub fn bind_alias(&mut self, id: AssetId, alias: impl Into<String>) {
        let alias = alias.into();
        if let Some(rec) = self.assets.get_mut(&id) {
            if !rec.aliases.iter().any(|a| a == &alias) && rec.uri != alias {
                rec.aliases.push(alias.clone());
            }
        }
        self.keys.insert(alias, id);
    }

    /// Look up a previously loaded asset by id.
    pub fn handle<T: Clone + Send + 'static>(&self, id: AssetId) -> Option<ResourceHandle<T>> {
        let rec = self.assets.get(&id)?;
        Some(ResourceHandle {
            id,
            inner: rec.state.clone(),
            _phantom: std::marker::PhantomData,
        })
    }

    pub fn list_assets(&self) -> Vec<AssetInfo> {
        let mut out: Vec<AssetInfo> = self
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

    fn resolve_loader_id(&self, opts: &LoadOpts, protocol: &str, path: &str) -> String {
        if let Some(id) = opts.loader {
            return id.to_string();
        }
        if let Some(ext) = file_extension(path) {
            if let Some(&id) = self.extensions.get(&ext) {
                return id.to_string();
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

/// Register built-in host loaders (`file`, `bsp`, `wad`) for `gamedir`.
pub fn register_default_loaders(rm: &mut ResourceManager, gamedir: &Path) {
    rm.set_base_dir(gamedir);
    rm.register_loader(Box::new(FileLoader));
    rm.register_loader(Box::new(BspLoader::new(gamedir)));
    rm.register_loader(Box::new(wad_loader::WadLoader {
        root: gamedir.to_path_buf(),
    }));
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
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (rest.to_string(), String::new()),
    };
    let path = path.trim_end_matches('/').to_string();
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

pub struct FileLoader;

impl Loader for FileLoader {
    fn id(&self) -> &'static str {
        "file"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["obj", "fbx", "wgsl"]
    }

    fn load(&self, path: &str, query: &str) -> Result<LoadedAsset, String> {
        if path.ends_with(".wgsl") {
            let source = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {path}: {e}"))?;
            let size = source.len() as u64;
            return Ok(LoadedAsset {
                data: Box::new(source),
                size,
                type_name: "shader",
            });
        }

        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let mut mesh = if path.ends_with(".obj") {
            let data = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {path}: {e}"))?;
            obj::parse_obj(&data)?
        } else if path.ends_with(".fbx") {
            let data =
                std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            fbx::parse_fbx(&data)?
        } else {
            return Err(format!("unrecognised file extension: {path}"));
        };
        let transform = transform::parse_mesh_transform(query);
        transform::apply_mesh_transform(&mut mesh, &transform);
        Ok(LoadedAsset {
            data: Box::new(mesh),
            size,
            type_name: "mesh",
        })
    }
}
