use crate::{EntityId, Resolver};

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

/// C ABI handle for a loaded BSP asset (mesh + materials + entities lump).
///
/// Opaque: data lives in the host ResourceManager. Use [`ResourcesApi::place_bsp`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BspMap {
    pub id: u64,
}

type LoadBspFn = extern "C" fn(*mut u8, *const u8, usize, *mut u64) -> i32;
type PlaceBspFn = extern "C" fn(*mut u8, u64, *const u8, usize) -> u64;

pub struct ResourcesApi {
    ctx: *mut u8,
    load_bsp_fn: LoadBspFn,
    place_bsp_fn: PlaceBspFn,
}

unsafe impl Send for ResourcesApi {}
unsafe impl Sync for ResourcesApi {}

impl ResourcesApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            load_bsp_fn: unsafe { std::mem::transmute(resolve(r, "v1:resources:load_bsp")) },
            place_bsp_fn: unsafe { std::mem::transmute(resolve(r, "v1:resources:place_bsp")) },
        }
    }

    /// Load BSP via AssetManager / ResourceManager.
    ///
    /// `path`: bare map name (`oldlifehome`), `maps/foo.bsp`, or absolute path.
    pub fn load_bsp(&self, path: &str) -> Result<BspMap, String> {
        let bytes = path.as_bytes();
        let mut id = 0u64;
        let rc = (self.load_bsp_fn)(self.ctx, bytes.as_ptr(), bytes.len(), &mut id);
        if rc == 0 && id != 0 {
            Ok(BspMap { id })
        } else {
            Err(format!("resources.load_bsp failed: {path}"))
        }
    }

    /// Place loaded BSP into the world (geometry + map entities). Returns worldspawn entity.
    pub fn place_bsp(&self, map: BspMap, map_name: &str) -> Result<EntityId, String> {
        let bytes = map_name.as_bytes();
        let entity = (self.place_bsp_fn)(self.ctx, map.id, bytes.as_ptr(), bytes.len());
        if entity == 0 {
            Err(format!("resources.place_bsp failed: id={} name={map_name}", map.id))
        } else {
            Ok(entity)
        }
    }
}
