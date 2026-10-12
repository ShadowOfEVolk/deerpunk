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
    header: FrameHeader,
    write_file: std::fs::File,
    read_file: std::fs::File,
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

        let header = FrameHeader {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            stride: DEFAULT_WIDTH * BYTES_PER_PIXEL as u32,
            frame_number: 0,
            dirty: 0,
            _pad: [0; 3],
        };

        let header_bytes = *header.as_bytes();
        use std::io::Write;
        let mut wf = file.try_clone().map_err(|e| format!("[frame_buffer] clone write: {e}"))?;
        wf.write_all(&header_bytes).map_err(|e| format!("[frame_buffer] write header: {e}"))?;

        let rf = file.try_clone().map_err(|e| format!("[frame_buffer] clone read: {e}"))?;

        Ok(Self { path, header, write_file: wf, read_file: rf })
    }

    pub fn open(name: &str) -> Result<Self, String> {
        let path = shm_path(name);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("[frame_buffer] open {}: {e}", path.display()))?;

        let mut header = if file.metadata().map(|m| m.len() as usize).unwrap_or(0) >= FrameHeader::SIZE {
            let mut hdr_buf = [0u8; FrameHeader::SIZE];
            let mut rf = file.try_clone().map_err(|e| format!("[frame_buffer] clone read: {e}"))?;
            rf.read_exact(&mut hdr_buf).map_err(|e| format!("[frame_buffer] read header: {e}"))?;
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
        header.dirty = 0;

        let wf = file.try_clone().map_err(|e| format!("[frame_buffer] clone write: {e}"))?;
        let rf = file.try_clone().map_err(|e| format!("[frame_buffer] clone read: {e}"))?;

        Ok(Self { path, header, write_file: wf, read_file: rf })
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

    pub fn dirty(&mut self) -> bool {
        let offset = 4 + 4 + 4 + 4;
        let _ = self.read_file.seek(SeekFrom::Start(0));
        let mut header_buf = [0u8; FrameHeader::SIZE];
        if self.read_file.read_exact(&mut header_buf).is_ok() {
            let mut buf = [0u8; 4];
            buf.copy_from_slice(&header_buf[offset..offset + 4]);
            u32::from_ne_bytes(buf) != 0
        } else {
            false
        }
    }

    pub fn write_frame(&mut self, rgba: &[u8]) {
        let w = self.header.width as usize;
        let h = self.header.height as usize;
        let expected = w * h * BYTES_PER_PIXEL;
        let copy_len = rgba.len().min(expected);

        self.header.frame_number = self.header.frame_number.wrapping_add(1);
        self.header.dirty = 1;

        let _ = self.write_file.seek(SeekFrom::Start(0));
        let header_bytes = *self.header.as_bytes();
        let _ = self.write_file.write_all(&header_bytes);
        if copy_len > 0 {
            let _ = self.write_file.write_all(&rgba[..copy_len]);
        }
        if self.header.frame_number % 60 == 1 {
            eprintln!("[frame_buffer] write_frame #{} ({}x{}, {} bytes)", self.header.frame_number, w, h, copy_len);
        }
    }

    pub fn read_frame(&mut self) -> Vec<u8> {
        let w = self.header.width as usize;
        let h = self.header.height as usize;
        let expected = w * h * BYTES_PER_PIXEL;
        let mut pixels = vec![0u8; expected];

        let _ = self.read_file.seek(SeekFrom::Start(0));
        let mut hdr_buf = [0u8; FrameHeader::SIZE];
        if self.read_file.read_exact(&mut hdr_buf).is_ok() {
            self.header = unsafe { std::ptr::read(hdr_buf.as_ptr() as *const FrameHeader) };
        }
        let _ = self.read_file.seek(SeekFrom::Start(FrameHeader::SIZE as u64));
        let _ = self.read_file.read_exact(&mut pixels);

        pixels
    }

    pub fn clear_dirty(&mut self) {
        self.header.dirty = 0;
        let offset = 4 + 4 + 4 + 4;
        let _ = self.write_file.seek(SeekFrom::Start(offset as u64));
        let _ = self.write_file.write_all(&0u32.to_ne_bytes());
    }
}
