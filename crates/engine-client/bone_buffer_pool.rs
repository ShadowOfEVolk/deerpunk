//! CPU/GPU bone matrix palettes for skinned mesh draws.

use glam::Mat4;

pub const MAX_BONES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoneBufferHandle(pub u32);

pub struct BoneBufferPool {
    next_id: u32,
    matrices: Vec<Vec<Mat4>>,
}

impl BoneBufferPool {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            matrices: Vec::new(),
        }
    }

    pub fn alloc(&mut self, bone_count: usize) -> BoneBufferHandle {
        let count = bone_count.clamp(1, MAX_BONES);
        let id = self.next_id;
        self.next_id += 1;
        let idx = id as usize;
        if idx >= self.matrices.len() {
            self.matrices.resize(idx + 1, Vec::new());
        }
        self.matrices[idx] = vec![Mat4::IDENTITY; count];
        BoneBufferHandle(id)
    }

    pub fn free(&mut self, handle: BoneBufferHandle) {
        let idx = handle.0 as usize;
        if idx < self.matrices.len() {
            self.matrices[idx].clear();
        }
    }

    pub fn write(&mut self, handle: BoneBufferHandle, mats: &[Mat4]) -> bool {
        let idx = handle.0 as usize;
        if idx == 0 || idx >= self.matrices.len() {
            return false;
        }
        let slot = &mut self.matrices[idx];
        if mats.len() != slot.len() {
            slot.resize(mats.len().clamp(1, MAX_BONES), Mat4::IDENTITY);
        }
        for (i, m) in mats.iter().enumerate() {
            if i < slot.len() {
                slot[i] = *m;
            }
        }
        true
    }

    pub fn matrices(&self, handle: BoneBufferHandle) -> Option<&[Mat4]> {
        let idx = handle.0 as usize;
        if idx == 0 || idx >= self.matrices.len() {
            return None;
        }
        let slot = &self.matrices[idx];
        if slot.is_empty() {
            None
        } else {
            Some(slot.as_slice())
        }
    }
}
