/// ECS marker: one entity = one RML document (path relative to `{gamedir}/ui`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct UiDocumentComponent {
    /// UTF-8 path, e.g. `widgets/hello.rml`
    pub path: [u8; 128],
    pub path_len: u8,
    /// 0 = closed/hidden, nonzero = load & show
    pub visible: u8,
    pub _pad: [u8; 6],
}

impl Default for UiDocumentComponent {
    fn default() -> Self {
        Self {
            path: [0; 128],
            path_len: 0,
            visible: 1,
            _pad: [0; 6],
        }
    }
}

impl UiDocumentComponent {
    pub fn new(path: &str) -> Self {
        let mut c = Self::default();
        c.set_path(path);
        c.visible = 1;
        c
    }

    pub fn set_path(&mut self, path: &str) {
        let bytes = path.as_bytes();
        let n = bytes.len().min(self.path.len());
        self.path[..n].copy_from_slice(&bytes[..n]);
        if n < self.path.len() {
            self.path[n..].fill(0);
        }
        self.path_len = n as u8;
    }

    pub fn path_str(&self) -> &str {
        let n = (self.path_len as usize).min(self.path.len());
        std::str::from_utf8(&self.path[..n]).unwrap_or("")
    }
}
