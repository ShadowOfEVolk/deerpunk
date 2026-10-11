use crate::snapshot::RenderSnapshot;

#[derive(Clone, Copy, Debug)]
pub struct DebugColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl DebugColor {
    pub const RED: Self = Self { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const GREEN: Self = Self { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
    pub const BLUE: Self = Self { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
    pub const YELLOW: Self = Self { r: 1.0, g: 1.0, b: 0.0, a: 1.0 };
    pub const CYAN: Self = Self { r: 0.0, g: 1.0, b: 1.0, a: 1.0 };
}

#[derive(Clone, Debug)]
pub enum DebugPrimitive {
    Line { start: [f32; 3], end: [f32; 3], color: DebugColor },
}

#[derive(Clone, Default)]
pub struct DebugDraw {
    pub primitives: Vec<DebugPrimitive>,
}

impl DebugDraw {
    pub fn new() -> Self { Self { primitives: Vec::with_capacity(256) } }
    pub fn clear(&mut self) { self.primitives.clear(); }
    pub fn line(&mut self, start: [f32; 3], end: [f32; 3], color: DebugColor) {
        self.primitives.push(DebugPrimitive::Line { start, end, color });
    }

    /// Axis-aligned box from min/max corners.
    pub fn aabb(&mut self, min: [f32; 3], max: [f32; 3], color: DebugColor) {
        let c = [
            [min[0], min[1], min[2]],
            [max[0], min[1], min[2]],
            [max[0], max[1], min[2]],
            [min[0], max[1], min[2]],
            [min[0], min[1], max[2]],
            [max[0], min[1], max[2]],
            [max[0], max[1], max[2]],
            [min[0], max[1], max[2]],
        ];
        const EDGES: [[usize; 2]; 12] = [
            [0, 1], [1, 2], [2, 3], [3, 0],
            [4, 5], [5, 6], [6, 7], [7, 4],
            [0, 4], [1, 5], [2, 6], [3, 7],
        ];
        for [a, b] in EDGES {
            self.line(c[a], c[b], color);
        }
    }

    /// Oriented box: `center` + `half_extents`, rotation as unit quaternion xyzw.
    pub fn obb(
        &mut self,
        center: [f32; 3],
        half_extents: [f32; 3],
        rotation_xyzw: [f32; 4],
        color: DebugColor,
    ) {
        let (x, y, z, w) = (
            rotation_xyzw[0],
            rotation_xyzw[1],
            rotation_xyzw[2],
            rotation_xyzw[3],
        );
        let mut corners = [[0.0f32; 3]; 8];
        let mut i = 0;
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                for sz in [-1.0f32, 1.0] {
                    let lx = sx * half_extents[0];
                    let ly = sy * half_extents[1];
                    let lz = sz * half_extents[2];
                    // q * v
                    let ix = w * lx + y * lz - z * ly;
                    let iy = w * ly + z * lx - x * lz;
                    let iz = w * lz + x * ly - y * lx;
                    let iw = -x * lx - y * ly - z * lz;
                    let rx = ix * w + iw * -x + iy * -z - iz * -y;
                    let ry = iy * w + iw * -y + iz * -x - ix * -z;
                    let rz = iz * w + iw * -z + ix * -y - iy * -x;
                    corners[i] = [center[0] + rx, center[1] + ry, center[2] + rz];
                    i += 1;
                }
            }
        }
        const EDGES: [[usize; 2]; 12] = [
            [0, 1], [1, 3], [3, 2], [2, 0],
            [4, 5], [5, 7], [7, 6], [6, 4],
            [0, 4], [1, 5], [2, 6], [3, 7],
        ];
        for [a, b] in EDGES {
            self.line(corners[a], corners[b], color);
        }
    }

    /// Wireframe for a point cloud: AABB of verts + all unique hull-ish edges (complete graph if ≤12).
    pub fn convex_wire(&mut self, vertices: &[[f32; 3]], color: DebugColor) {
        if vertices.is_empty() {
            return;
        }
        let mut min = vertices[0];
        let mut max = vertices[0];
        for v in vertices {
            for i in 0..3 {
                min[i] = min[i].min(v[i]);
                max[i] = max[i].max(v[i]);
            }
        }
        self.aabb(min, max, color);
        if vertices.len() <= 12 {
            for i in 0..vertices.len() {
                for j in (i + 1)..vertices.len() {
                    self.line(vertices[i], vertices[j], color);
                }
            }
        }
    }

    pub fn sphere(&mut self, center: [f32; 3], radius: f32, color: DebugColor) {
        self.circle(center, radius, [1.0, 0.0, 0.0], 24, color);
        self.circle(center, radius, [0.0, 1.0, 0.0], 24, color);
        self.circle(center, radius, [0.0, 0.0, 1.0], 24, color);
    }
    pub fn label(&mut self, _pos: [f32; 3], _text: &str, _color: DebugColor) {}
    pub fn circle(&mut self, center: [f32; 3], radius: f32, normal: [f32; 3], segments: u32, color: DebugColor) {
        let (nx, ny, nz) = (normal[0], normal[1], normal[2]);
        // Pick an arbitrary tangent not parallel to normal
        let (ux, uy, uz) = if nx.abs() < 0.9 {
            let t = (1.0 - nx * nx).sqrt();
            (0.0, nz / t, -ny / t)
        } else {
            let t = (1.0 - ny * ny).sqrt();
            (nz / t, 0.0, -nx / t)
        };
        // Second tangent = normal × u
        let vx = ny * uz - nz * uy;
        let vy = nz * ux - nx * uz;
        let vz = nx * uy - ny * ux;

        let mut prev = None;
        for i in 0..=segments {
            let angle = (i as f32 / segments as f32) * std::f32::consts::TAU;
            let c = angle.cos();
            let s = angle.sin();
            let px = center[0] + (ux * c + vx * s) * radius;
            let py = center[1] + (uy * c + vy * s) * radius;
            let pz = center[2] + (uz * c + vz * s) * radius;
            if let Some((ppx, ppy, ppz)) = prev {
                self.line([ppx, ppy, ppz], [px, py, pz], color);
            }
            prev = Some((px, py, pz));
        }
    }
}

/// Вершина для debug линий: позиция + цвет.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DebugVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

/// Конвертирует линии в квады (2 треугольника каждая).
/// Каждая линия → 6 вершин (4 угла квада, 2 треугольника).
pub fn collect_vertices(snap: &RenderSnapshot, line_width: f32, max_verts: usize) -> Vec<DebugVertex> {
    let mut verts = Vec::with_capacity(max_verts.min(65536));
    let up = [0.0f32, 0.0, 1.0]; // Z-up

    for prim in &snap.debug_draw {
        if verts.len() + 6 > max_verts { break; }
        match prim {
            DebugPrimitive::Line { start, end, color } => {
                let dx = end[0] - start[0];
                let dy = end[1] - start[1];
                let dz = end[2] - start[2];
                let len = (dx * dx + dy * dy + dz * dz).sqrt();
                if len < 0.001 { continue; }

                // Направление и нормаль (cross с up)
                let dir = [dx / len, dy / len, dz / len];
                let nx = dir[1] * up[2] - dir[2] * up[1];
                let ny = dir[2] * up[0] - dir[0] * up[2];
                let nz = dir[0] * up[1] - dir[1] * up[0];
                let nl = (nx * nx + ny * ny + nz * nz).sqrt();
                let (nx, ny, nz) = if nl > 0.001 {
                    (nx / nl, ny / nl, nz / nl)
                } else {
                    (1.0, 0.0, 0.0) // fallback
                };

                let hw = line_width * 0.5;
                let c = [color.r, color.g, color.b, color.a];

                // 4 угла квада
                let v0 = [start[0] - nx * hw, start[1] - ny * hw, start[2] - nz * hw];
                let v1 = [start[0] + nx * hw, start[1] + ny * hw, start[2] + nz * hw];
                let v2 = [end[0] - nx * hw, end[1] - ny * hw, end[2] - nz * hw];
                let v3 = [end[0] + nx * hw, end[1] + ny * hw, end[2] + nz * hw];

                // 2 треугольника: (v0, v1, v2) и (v1, v3, v2)
                verts.push(DebugVertex { position: v0, color: c });
                verts.push(DebugVertex { position: v1, color: c });
                verts.push(DebugVertex { position: v2, color: c });
                verts.push(DebugVertex { position: v1, color: c });
                verts.push(DebugVertex { position: v3, color: c });
                verts.push(DebugVertex { position: v2, color: c });
            }
        }
    }
    verts
}
