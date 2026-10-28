//! Opaque mesh id for ECS components and FFI.
//!
//! Packed `(idx, gen)` so a stale handle (slot reused) returns `None` from the
//! registry instead of silently resolving to another mesh. `0` is the invalid
//! handle (slot 0 is reserved and never holds a mesh).

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub usize);

impl MeshHandle {
    /// Packed `(idx=0, gen=0)` — never resolves to a real mesh.
    pub const INVALID: MeshHandle = MeshHandle(0);

    #[inline]
    pub const fn from_parts(idx: u32, gen: u32) -> Self {
        Self((idx as u64 | ((gen as u64) << 32)) as usize)
    }

    #[inline]
    pub const fn idx(self) -> u32 {
        (self.0 as u64 & 0xFFFF_FFFF) as u32
    }

    #[inline]
    pub const fn gen(self) -> u32 {
        ((self.0 as u64) >> 32) as u32
    }

    #[inline]
    pub fn is_valid(self) -> bool {
        self.0 != 0
    }
}

impl std::fmt::Debug for MeshHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_valid() {
            f.debug_tuple("MeshHandle")
                .field(&format_args!("idx={} gen={}", self.idx(), self.gen()))
                .finish()
        } else {
            write!(f, "MeshHandle(INVALID)")
        }
    }
}
