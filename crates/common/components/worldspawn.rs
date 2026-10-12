/// Marker / data for a world geometry entity loaded from a GoldSrc BSP.
/// Actual mesh is placed in [`super::Mesh3D`] by the scene loader.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Worldspawn {
    /// Map name without path/extension, e.g. `tfhouse` → `{gamedir}/maps/tfhouse.bsp`
    pub map: [u8; 64],
    pub map_len: u8,
    pub _pad: [u8; 3],
}

impl Default for Worldspawn {
    fn default() -> Self {
        Self {
            map: [0; 64],
            map_len: 0,
            _pad: [0; 3],
        }
    }
}

impl Worldspawn {
    pub fn new(map: &str) -> Self {
        let mut w = Self::default();
        w.set_map(map);
        w
    }

    pub fn set_map(&mut self, map: &str) {
        let bytes = map.as_bytes();
        let n = bytes.len().min(self.map.len());
        self.map[..n].copy_from_slice(&bytes[..n]);
        if n < self.map.len() {
            self.map[n..].fill(0);
        }
        self.map_len = n as u8;
    }

    pub fn map_str(&self) -> &str {
        let n = (self.map_len as usize).min(self.map.len());
        std::str::from_utf8(&self.map[..n]).unwrap_or("")
    }
}
