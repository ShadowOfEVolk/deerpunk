use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

const BYTES_PER_PIXEL: usize = 4;
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub frame_number: u32,
    pub dirty: u32,
    pub _pad: [u32; 3],
}

impl FrameHeader {
    pub const SIZE: usize = std::mem::size_of::<FrameHeader>();

    pub fn as_bytes(&self) -> &[u8; FrameHeader::SIZE] {
        unsafe { std::mem::transmute(self) }
    }
}

fn shm_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{}.bin", name))
}

pub struct SharedFrameBuffer {
    path: PathBuf,
    map: memmap2::MmapMut,
    header: FrameHeader,
}

impl SharedFrameBuffer {
    pub fn create(name: &str) -> Result<Self, String> {
        let path = shm_path(name);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| format!("[frame_buffer] create {}: {e}", path.display()))?;

        let total = FrameHeader::SIZE
            + DEFAULT_WIDTH as usize * DEFAULT_HEIGHT as usize * BYTES_PER_PIXEL;
        file.set_len(total as u64)
            .map_err(|e| format!("[frame_buffer] set_len: {e}"))?;

        let mut map = unsafe {
            memmap2::MmapMut::map_mut(&file)
                .map_err(|e| format!("[frame_buffer] mmap: {e}"))?
        };

        let header = FrameHeader {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            stride: DEFAULT_WIDTH * BYTES_PER_PIXEL as u32,
            frame_number: 0,
            dirty: 0,
            _pad: [0; 3],
        };

        let header_bytes = *header.as_bytes();
        map[..FrameHeader::SIZE].copy_from_slice(&header_bytes);
        let _ = map.flush();

        Ok(Self { path, map, header })
    }

    pub fn open(name: &str) -> Result<Self, String> {
        let path = shm_path(name);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("[frame_buffer] open {}: {e}", path.display()))?;

        let map = unsafe {
            memmap2::MmapMut::map_mut(&file)
                .map_err(|e| format!("[frame_buffer] mmap: {e}"))?
        };

        let header = if map.len() >= FrameHeader::SIZE {
            let mut hdr_buf = [0u8; FrameHeader::SIZE];
            hdr_buf.copy_from_slice(&map[..FrameHeader::SIZE]);
            unsafe { std::ptr::read(hdr_buf.as_ptr() as *const FrameHeader) }
        } else {
            FrameHeader {
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
                stride: DEFAULT_WIDTH * 4,
                frame_number: 0,
                dirty: 0,
                _pad: [0; 3],
            }
        };

        Ok(Self { path, map, header })
    }

    pub fn width(&self) -> u32 {
        self.header.width
    }

    pub fn height(&self) -> u32 {
        self.header.height
    }

    pub fn stride(&self) -> u32 {
        self.header.stride
    }

    pub fn frame_number(&self) -> u32 {
        self.header.frame_number
    }

    pub fn dirty(&self) -> bool {
        match std::fs::File::open(&self.path) {
            Ok(mut f) => {
                let offset = 4 + 4 + 4 + 4;
                let mut buf = [0u8; 4];
                let mut header_buf = [0u8; FrameHeader::SIZE];
                if f.read_exact(&mut header_buf).is_ok() {
                    buf.copy_from_slice(&header_buf[offset..offset + 4]);
                    u32::from_ne_bytes(buf) != 0
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    }

    pub fn write_frame(&mut self, rgba: &[u8]) {
        let w = self.header.width as usize;
        let h = self.header.height as usize;
        let expected = w * h * BYTES_PER_PIXEL;
        let copy_len = rgba.len().min(expected);

        self.header.frame_number = self.header.frame_number.wrapping_add(1);
        self.header.dirty = 1;

        let header_bytes = *self.header.as_bytes();

        if self.map.len() >= FrameHeader::SIZE + expected {
            let (head, body) = self.map.split_at_mut(FrameHeader::SIZE);
            head[..FrameHeader::SIZE].copy_from_slice(&header_bytes);
            body[..copy_len].copy_from_slice(&rgba[..copy_len]);
        }

        let _ = self.map.flush();
    }

    pub fn read_frame(&mut self) -> Vec<u8> {
        let w = self.header.width as usize;
        let h = self.header.height as usize;
        let expected = w * h * BYTES_PER_PIXEL;
        let mut pixels = vec![0u8; expected];

        if let Ok(mut f) = std::fs::File::open(&self.path) {
            let mut hdr_buf = [0u8; FrameHeader::SIZE];
            if f.read_exact(&mut hdr_buf).is_ok() {
                self.header = unsafe { std::ptr::read(hdr_buf.as_ptr() as *const FrameHeader) };
            }
            let _ = f.seek(SeekFrom::Start(FrameHeader::SIZE as u64));
            let _ = f.read_exact(&mut pixels);
        }

        pixels
    }

    pub fn clear_dirty(&mut self) {
        self.header.dirty = 0;
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(&self.path) {
            let offset = 4 + 4 + 4 + 4;
            let _ = f.seek(SeekFrom::Start(offset as u64));
            let _ = f.write_all(&0u32.to_ne_bytes());
        }
    }
}
