use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub struct HostAccessApi {
    ctx: *mut u8,
    flush_albedos_fn: extern "C" fn(*mut u8) -> i32,
    client_track_static_fn: extern "C" fn(*mut u8, u64) -> i32,
    refresh_stream_meshes_fn: extern "C" fn(*mut u8) -> i32,
    client_apply_mesh_uri_fn: extern "C" fn(*mut u8, u64, *const u8, usize) -> i32,
    register_main_console_fn: extern "C" fn(*mut u8, *const u8, usize) -> i32,
    start_listen_map_fn: extern "C" fn(*mut u8, *const u8, usize, i32) -> i32,
    loader_queue_set_fn: extern "C" fn(
        *mut u8,
        i32,
        i32,
        i32,
        *const u8,
        usize,
        *const u8,
        usize,
    ),
    loader_queue_clear_fn: extern "C" fn(*mut u8),
}

unsafe impl Send for HostAccessApi {}
unsafe impl Sync for HostAccessApi {}

impl HostAccessApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            flush_albedos_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:flush_pending_albedos"))
            },
            client_track_static_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:client_track_static"))
            },
            refresh_stream_meshes_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:refresh_stream_meshes"))
            },
            client_apply_mesh_uri_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:client_apply_mesh_uri"))
            },
            register_main_console_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:register_main_console"))
            },
            start_listen_map_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:start_listen_map"))
            },
            loader_queue_set_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:loader_queue_set"))
            },
            loader_queue_clear_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:host:loader_queue_clear"))
            },
        }
    }

    pub fn flush_pending_albedos(&self) -> u32 {
        (self.flush_albedos_fn)(self.ctx) as u32
    }

    pub fn client_track_static_entity(&self, entity: u64) {
        (self.client_track_static_fn)(self.ctx, entity);
    }

    pub fn refresh_stream_mesh_overrides(&self) {
        (self.refresh_stream_meshes_fn)(self.ctx);
    }

    /// Client-only: resolve URI on host [`ResourceManager`] and set `Mesh3D` + GPU adopt.
    pub fn client_apply_mesh_uri(&self, entity: u64, uri: &str) -> bool {
        (self.client_apply_mesh_uri_fn)(self.ctx, entity, uri.as_ptr(), uri.len()) != 0
    }

    /// Listen client: register a main-thread console alias for a server gamelib command.
    pub fn register_main_console(&self, name: &str) -> i32 {
        (self.register_main_console_fn)(self.ctx, name.as_ptr(), name.len())
    }

    /// Listen client: run on main thread; loads world on the server simulation thread.
    pub fn start_listen_map(&self, world: &str, seed: i32) -> i32 {
        (self.start_listen_map_fn)(self.ctx, world.as_ptr(), world.len(), seed)
    }

    /// Report startup / map load progress for engine UI hooks (also used by gamelib loading screens).
    pub fn loader_queue_set(
        &self,
        active: bool,
        done: u32,
        total: u32,
        status: &str,
        hint: &str,
    ) {
        (self.loader_queue_set_fn)(
            self.ctx,
            if active { 1 } else { 0 },
            done as i32,
            total as i32,
            status.as_ptr(),
            status.len(),
            hint.as_ptr(),
            hint.len(),
        );
    }

    pub fn loader_queue_clear(&self) {
        (self.loader_queue_clear_fn)(self.ctx);
    }
}
