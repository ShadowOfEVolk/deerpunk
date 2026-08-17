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
}
