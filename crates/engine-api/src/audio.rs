use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

pub struct AudioApi {
    ctx: *mut u8,
    play_2d_fn: extern "C" fn(*mut u8, *const u8, usize, u32) -> u32,
    play_3d_fn: extern "C" fn(*mut u8, *const u8, usize, f32, f32, f32, u32, *const u8, usize, f32) -> u32,
    start_attached_fn: extern "C" fn(*mut u8, u64, *const u8, usize, f32, f32, f32, u32) -> u32,
    stop_sound_fn: extern "C" fn(*mut u8, u32) -> i32,
    set_world_mesh_fn: extern "C" fn(*mut u8, u64) -> i32,
}

unsafe impl Send for AudioApi {}
unsafe impl Sync for AudioApi {}

impl AudioApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            play_2d_fn: unsafe { std::mem::transmute(resolve(r, "v1:audio:play_2d")) },
            play_3d_fn: unsafe { std::mem::transmute(resolve(r, "v1:audio:play_3d")) },
            start_attached_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:audio:start_attached"))
            },
            stop_sound_fn: unsafe { std::mem::transmute(resolve(r, "v1:audio:stop_sound")) },
            set_world_mesh_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:audio:set_world_mesh"))
            },
        }
    }

    /// One-shot 2D; returns server **`sound_id`** (for [`Self::stop_sound`] if needed).
    pub fn play_2d(&self, event: &str, volume: f32) -> u32 {
        let ev = event.as_bytes();
        (self.play_2d_fn)(
            self.ctx,
            ev.as_ptr(),
            ev.len(),
            (volume * 255.0).clamp(0.0, 255.0) as u32,
        )
    }

    /// One-shot 3D at `origin`.
    pub fn play_3d(
        &self,
        event: &str,
        origin: [f32; 3],
        volume: f32,
        params: &[(&str, f32)],
    ) -> u32 {
        let ev = event.as_bytes();
        let (pname, pvalue): (&[u8], f32) = match params.first() {
            Some((n, v)) => (n.as_bytes(), *v),
            None => (&[], 0.0),
        };
        (self.play_3d_fn)(
            self.ctx,
            ev.as_ptr(),
            ev.len(),
            origin[0],
            origin[1],
            origin[2],
            (volume * 255.0).clamp(0.0, 255.0) as u32,
            pname.as_ptr(),
            pname.len(),
            pvalue,
        )
    }

    /// Looping sound; `entity` is server entity for `net_id` follow. Returns **`sound_id`**.
    pub fn start_attached(
        &self,
        entity: u64,
        event: &str,
        origin: [f32; 3],
        volume: f32,
    ) -> u32 {
        let ev = event.as_bytes();
        (self.start_attached_fn)(
            self.ctx,
            entity,
            ev.as_ptr(),
            ev.len(),
            origin[0],
            origin[1],
            origin[2],
            (volume * 255.0).clamp(0.0, 255.0) as u32,
        )
    }

    pub fn stop_sound(&self, sound_id: u32) {
        let _ = (self.stop_sound_fn)(self.ctx, sound_id);
    }

    /// Install client-world entity mesh as the audio simulation scene (mesh-only).
    pub fn set_world_mesh(&self, entity: u64) -> bool {
        (self.set_world_mesh_fn)(self.ctx, entity) == 0
    }

    /// @deprecated — use [`Self::stop_sound`].
    pub fn stop_attached(&self, sound_id: u32) {
        self.stop_sound(sound_id);
    }
}
