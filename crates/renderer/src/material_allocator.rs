use std::collections::HashMap;
use deerpunk_engine_common::components::material::Material;
use deerpunk_engine_common::MeshTexture;

/// Material GPU data array — SSBO backing storage.
const INITIAL_CAP: usize = 64;

/// GPU-side material data matching WGSL MaterialData layout.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialGpuEntry {
    pub color: [f32; 4],
    pub params: [f32; 4],
    pub albedo_idx: u32,
    pub normal_idx: u32,
    pub orm_idx: u32,
    pub _pad: u32,
    pub emission: [f32; 4],
    pub sun_dir: [f32; 4],
    pub ambient: [f32; 4],
}

/// Invalid material ID sentinel.
pub const MATERIAL_WHITE_ID: u32 = 0;

pub struct MaterialAllocator {
    entries: Vec<MaterialGpuEntry>,
    gpu_buf: wgpu::Buffer,
    cache: HashMap<u64, u32>,
    dirty: bool,
    pub generation: u64,
}

impl MaterialAllocator {
    pub fn new(device: &wgpu::Device) -> Self {
        let cap = (INITIAL_CAP * std::mem::size_of::<MaterialGpuEntry>()) as u64;
        let gpu_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material_ssbo"),
            size: cap,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut allocator = Self {
            entries: Vec::with_capacity(INITIAL_CAP),
            gpu_buf,
            cache: HashMap::new(),
            dirty: false,
            generation: 0,
        };

        // Index 0 = white/default material (material_id 0 always valid).
        let default = MaterialGpuEntry {
            color: [1.0, 1.0, 1.0, 1.0],
            params: [0.0, 0.5, 0.0, 0.0],
            albedo_idx: MeshTexture::WHITE.0,
            normal_idx: MeshTexture::WHITE.0,
            orm_idx: MeshTexture::WHITE.0,
            _pad: 0,
            emission: [0.0, 0.0, 0.0, 0.0],
            sun_dir: [0.0, 0.0, 1.0, 0.0],
            ambient: [0.0, 0.0, 0.0, 0.0],
        };
        allocator.entries.push(default);
        allocator.dirty = true;

        allocator
    }

    /// Ensure a material is uploaded. Returns its material_id (stable across frames).
    pub fn ensure_uploaded(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, material: &Material) -> u32 {
        let hash = hash_material(material);

        if let Some(&id) = self.cache.get(&hash) {
            return id;
        }

        let id = self.entries.len() as u32;
        let entry = MaterialGpuEntry {
            color: material.color,
            params: [material.metallic, material.roughness, 0.0, 0.0],
            albedo_idx: material.albedo.0,
            normal_idx: material.normal.0,
            orm_idx: material.ao_roughness_metallic.0,
            _pad: 0,
            emission: material.emission,
            sun_dir: material.sun_dir,
            ambient: material.ambient,
        };

        self.upload_entry(device, queue, id, entry)
    }

    /// Upload a raw GPU entry (used by MaterialStore, bypasses Material struct).
    /// No dedup — caller is responsible for dedup.
    pub fn ensure_uploaded_raw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        entry: &MaterialGpuEntry,
    ) -> u32 {
        let id = self.entries.len() as u32;
        self.upload_entry(device, queue, id, *entry)
    }

    /// Internal: grow buffer if needed, push entry, write to GPU.
    fn upload_entry(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: u32,
        entry: MaterialGpuEntry,
    ) -> u32 {

        // Grow buffer if needed.
        let needed = self.entries.len() + 1;
        let cap = self.gpu_buf.size() as usize / std::mem::size_of::<MaterialGpuEntry>();
        if needed > cap {
            let new_cap = (cap * 2).max(needed);
            let new_size = (new_cap * std::mem::size_of::<MaterialGpuEntry>()) as u64;
            let new_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("material_ssbo"),
                size: new_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            // Copy old data.
            let old_size = self.entries.len() * std::mem::size_of::<MaterialGpuEntry>();
            queue.write_buffer(&new_buf, 0, bytemuck::cast_slice(&self.entries));
            self.gpu_buf = new_buf;
            self.generation += 1;
        }

        self.entries.push(entry);
        self.dirty = true;

        // Write just the new entry.
        let offset = (id as usize * std::mem::size_of::<MaterialGpuEntry>()) as u64;
        queue.write_buffer(&self.gpu_buf, offset, bytemuck::cast_slice(&[entry]));

        id
    }

    pub fn flush(&mut self, queue: &wgpu::Queue) {
        if self.dirty {
            queue.write_buffer(&self.gpu_buf, 0, bytemuck::cast_slice(&self.entries));
            self.dirty = false;
        }
    }

    pub fn gpu_buffer(&self) -> &wgpu::Buffer {
        &self.gpu_buf
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

fn hash_material(m: &Material) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytemuck::cast_slice::<f32, u8>(&m.color).hash(&mut hasher);
    m.metallic.to_bits().hash(&mut hasher);
    m.roughness.to_bits().hash(&mut hasher);
    m.albedo.hash(&mut hasher);
    m.normal.hash(&mut hasher);
    m.ao_roughness_metallic.hash(&mut hasher);
    bytemuck::cast_slice::<f32, u8>(&m.emission).hash(&mut hasher);
    bytemuck::cast_slice::<f32, u8>(&m.sun_dir).hash(&mut hasher);
    bytemuck::cast_slice::<f32, u8>(&m.ambient).hash(&mut hasher);
    hasher.finish()
}
