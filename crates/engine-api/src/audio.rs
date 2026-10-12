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
    play_2d_fn: extern "C" fn(*mut u8, *const u8, usize, u32) -> i32,
    play_3d_fn: extern "C" fn(*mut u8, *const u8, usize, f32, f32, f32, u32, *const u8, usize, f32) -> i32,
    start_attached_fn: extern "C" fn(*mut u8, u64, *const u8, usize, f32, f32, f32, u32) -> i32,
    stop_attached_fn: extern "C" fn(*mut u8, u64) -> i32,
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
            stop_attached_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:audio:stop_attached"))
            },
        }
    }

    /// Fire-and-forget 2D (non-positional) sound event.
    pub fn play_2d(&self, event: &str, volume: f32) -> i32 {
        let ev = event.as_bytes();
        (self.play_2d_fn)(self.ctx, ev.as_ptr(), ev.len(), (volume * 255.0).clamp(0.0, 255.0) as u32)
    }

    /// Fire-and-forget 3D sound event at a world position, with one optional
    /// named float parameter (`param_name` empty = no params).
    pub fn play_3d(
        &self,
        event: &str,
        origin: [f32; 3],
        volume: f32,
        params: &[(&str, f32)],
    ) -> i32 {
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

    /// Start a sound attached to a world position, tracked by `key`.
    pub fn start_attached(
        &self,
        key: u64,
        event: &str,
        origin: [f32; 3],
        volume: f32,
    ) -> i32 {
        let ev = event.as_bytes();
        (self.start_attached_fn)(
            self.ctx,
            key,
            ev.as_ptr(),
            ev.len(),
            origin[0],
            origin[1],
            origin[2],
            (volume * 255.0).clamp(0.0, 255.0) as u32,
        )
    }

    /// Stop a previously started attached sound.
    pub fn stop_attached(&self, key: u64) -> i32 {
        (self.stop_attached_fn)(self.ctx, key)
    }
}
