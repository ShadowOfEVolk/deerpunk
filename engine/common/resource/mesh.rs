#[derive(Clone, Debug)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn empty() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        }
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn index_count(&self) -> usize {
        self.indices.len()
    }

    pub fn scale(&mut self, s: f32) {
        for p in &mut self.positions {
            p[0] *= s;
            p[1] *= s;
            p[2] *= s;
        }
    }

    /// Инвертирует V текстурных координат (OBJ → wgpu: у wgpu (0,0) — верхний левый угол изображения).
    pub fn flip_uv_v(&mut self) {
        for uv in &mut self.uvs {
            uv[1] = 1.0 - uv[1];
        }
    }

    /// Создаёт простой куб как fallback-меш (центрирован, 2×2×2, 6 граней по 2 треугольника).
    pub fn cube() -> Self {
        let (p, n, u, i) = build_cube();
        Self { positions: p, normals: n, uvs: u, indices: i }
    }
}

fn build_cube() -> (Vec<[f32;3]>, Vec<[f32;3]>, Vec<[f32;2]>, Vec<u32>) {
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut uv = Vec::new();
    let mut idx = Vec::new();
    let faces: [([f32;3], [f32;3], [f32;3], [f32;3], [f32;3]); 6] = [
        ([ 0.0, 0.0,-1.0], [-1.0,-1.0,-1.0], [ 1.0,-1.0,-1.0], [ 1.0, 1.0,-1.0], [-1.0, 1.0,-1.0]),
        ([ 0.0, 0.0, 1.0], [ 1.0,-1.0, 1.0], [-1.0,-1.0, 1.0], [-1.0, 1.0, 1.0], [ 1.0, 1.0, 1.0]),
        ([ 1.0, 0.0, 0.0], [ 1.0,-1.0,-1.0], [ 1.0, 1.0,-1.0], [ 1.0, 1.0, 1.0], [ 1.0,-1.0, 1.0]),
        ([-1.0, 0.0, 0.0], [-1.0,-1.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, 1.0,-1.0], [-1.0,-1.0,-1.0]),
        ([ 0.0, 1.0, 0.0], [-1.0, 1.0,-1.0], [ 1.0, 1.0,-1.0], [ 1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]),
        ([ 0.0,-1.0, 0.0], [-1.0,-1.0, 1.0], [ 1.0,-1.0, 1.0], [ 1.0,-1.0,-1.0], [-1.0,-1.0,-1.0]),
    ];
    let uvs = [[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0]];
    for (n, v0, v1, v2, v3) in &faces {
        let base = pos.len() as u32;
        pos.extend_from_slice(&[*v0, *v1, *v2, *v3]);
        nrm.extend_from_slice(&[*n, *n, *n, *n]);
        uv.extend_from_slice(&uvs);
        idx.extend_from_slice(&[base, base+1, base+2, base, base+2, base+3]);
    }
    (pos, nrm, uv, idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_has_correct_counts() {
        let m = Mesh::cube();
        assert_eq!(m.positions.len(), 24);
        assert_eq!(m.indices.len(), 36);
    }
}