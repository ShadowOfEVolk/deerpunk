use crate::resource::mesh::Mesh;

#[repr(C)]
#[derive(Clone, Debug)]
pub struct Terrain {
    pub heights: Vec<f32>,
    pub rows: usize,
    pub cols: usize,
    pub cell_size: f32,
    pub height_scale: f32,
}

impl Terrain {
    pub fn new(heights: Vec<f32>, rows: usize, cols: usize, cell_size: f32) -> Self {
        Self {
            heights,
            rows,
            cols,
            cell_size,
            height_scale: 1.0,
        }
    }

    pub fn height_at(&self, col: usize, row: usize) -> f32 {
        if col < self.cols && row < self.rows {
            self.heights[row * self.cols + col]
        } else {
            0.0
        }
    }

    pub fn world_height_at(&self, col: usize, row: usize) -> f32 {
        self.height_at(col, row) * self.height_scale
    }

    pub fn sample_bilinear(&self, x: f32, z: f32) -> f32 {
        let fx = x / self.cell_size;
        let fz = z / self.cell_size;
        let x0 = fx.floor() as isize;
        let z0 = fz.floor() as isize;
        let x1 = x0 + 1;
        let z1 = z0 + 1;
        let tx = fx - fx.floor();
        let tz = fz - fz.floor();

        let h00 = self.sample_clamped(x0, z0);
        let h10 = self.sample_clamped(x1, z0);
        let h01 = self.sample_clamped(x0, z1);
        let h11 = self.sample_clamped(x1, z1);

        let h0 = h00 * (1.0 - tx) + h10 * tx;
        let h1 = h01 * (1.0 - tx) + h11 * tx;
        (h0 * (1.0 - tz) + h1 * tz) * self.height_scale
    }

    fn sample_clamped(&self, x: isize, z: isize) -> f32 {
        let x = x.max(0).min(self.cols as isize - 1) as usize;
        let z = z.max(0).min(self.rows as isize - 1) as usize;
        self.heights[z * self.cols + x]
    }

    pub fn build_mesh(&self) -> Mesh {
        let mut positions = Vec::with_capacity(self.rows * self.cols * 4);
        let mut normals = Vec::with_capacity(self.rows * self.cols * 4);
        let mut uvs = Vec::with_capacity(self.rows * self.cols * 4);
        let mut indices = Vec::with_capacity((self.rows - 1) * (self.cols - 1) * 6);

        for z in 0..self.rows {
            for x in 0..self.cols {
                let px = x as f32 * self.cell_size;
                let py = z as f32 * self.cell_size;
                let h = self.world_height_at(x, z);
                positions.push([px, py, h]);
                uvs.push([x as f32 / (self.cols - 1) as f32, z as f32 / (self.rows - 1) as f32]);
            }
        }

        for z in 0..self.rows {
            for x in 0..self.cols {
                let h_l = self.world_height_at(x.saturating_sub(1), z);
                let h_r = self.world_height_at((x + 1).min(self.cols - 1), z);
                let h_d = self.world_height_at(x, z.saturating_sub(1));
                let h_u = self.world_height_at(x, (z + 1).min(self.rows - 1));
                let dx = h_r - h_l;
                let dy = h_u - h_d;
                let len = (dx * dx + dy * dy + 4.0 * self.cell_size * self.cell_size).sqrt();
                normals.push([-dx / len, -dy / len, 2.0 * self.cell_size / len]);
            }
        }

        for z in 0..(self.rows - 1) {
            for x in 0..(self.cols - 1) {
                let tl = (z * self.cols + x) as u32;
                let tr = (z * self.cols + x + 1) as u32;
                let bl = ((z + 1) * self.cols + x) as u32;
                let br = ((z + 1) * self.cols + x + 1) as u32;
                indices.extend_from_slice(&[tl, tr, bl, tr, br, bl]);
            }
        }

        let mut mesh = Mesh {
            positions,
            normals,
            uvs,
            lightmap_uvs: Vec::new(),
            lightmap_styles: Vec::new(),
            atlas_rects: Vec::new(),
            indices,
            joint_indices: Vec::new(),
            joint_weights: Vec::new(),
            content_id: 0,
            allocator_id: 0,
            bounds_min: [f32::INFINITY, f32::INFINITY, f32::INFINITY],
            bounds_max: [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY],
        };
        mesh.ensure_content_id();
        mesh.ensure_local_bounds();
        mesh
    }
}
