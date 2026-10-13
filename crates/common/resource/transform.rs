use crate::resource::mesh::Mesh;

/// Трансформация, применяемая к мешу при загрузке.
/// Задаётся в query-строке URI: `model.obj?scale=0.01&translate=1,2,3&rotate=90,0,0`.
#[derive(Debug, Clone, Copy)]
pub struct MeshTransform {
    pub scale: f32,
    pub translate: [f32; 3],
    /// Эйлеровы углы в градусах вокруг осей X, Y, Z. Применяются в порядке X→Y→Z.
    pub rotate_euler: [f32; 3],
}

impl Default for MeshTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            translate: [0.0; 3],
            rotate_euler: [0.0; 3],
        }
    }
}

/// Разбирает query-строку (`key=value&key=value`) в `MeshTransform`.
/// Неизвестные ключи и кривые значения игнорируются.
pub fn parse_mesh_transform(query: &str) -> MeshTransform {
    let mut t = MeshTransform::default();
    if query.is_empty() {
        return t;
    }
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "scale" => {
                if let Ok(s) = value.parse::<f32>() {
                    t.scale = s;
                }
            }
            "translate" => {
                let parts: Vec<f32> = value
                    .split(',')
                    .filter_map(|v| v.trim().parse::<f32>().ok())
                    .collect();
                if parts.len() >= 3 {
                    t.translate = [parts[0], parts[1], parts[2]];
                }
            }
            "rotate" => {
                let parts: Vec<f32> = value
                    .split(',')
                    .filter_map(|v| v.trim().parse::<f32>().ok())
                    .collect();
                if parts.len() >= 3 {
                    t.rotate_euler = [parts[0], parts[1], parts[2]];
                } else if parts.len() == 1 {
                    t.rotate_euler[2] = parts[0];
                }
            }
            _ => {}
        }
    }
    t
}

/// Поворот вектора вокруг заданной оси по Rodrigues (axis должен быть нормализован).
fn rotate_vec(v: [f32; 3], axis: [f32; 3], deg: f32) -> [f32; 3] {
    if deg == 0.0 {
        return v;
    }
    let rad = deg.to_radians();
    let (sin, cos) = rad.sin_cos();
    let dot = axis[0] * v[0] + axis[1] * v[1] + axis[2] * v[2];
    let cross = [
        axis[1] * v[2] - axis[2] * v[1],
        axis[2] * v[0] - axis[0] * v[2],
        axis[0] * v[1] - axis[1] * v[0],
    ];
    [
        v[0] * cos + cross[0] * sin + axis[0] * dot * (1.0 - cos),
        v[1] * cos + cross[1] * sin + axis[1] * dot * (1.0 - cos),
        v[2] * cos + cross[2] * sin + axis[2] * dot * (1.0 - cos),
    ]
}

/// Применяет трансформацию к мешу: позиции = R·(scale·p)+translate, нормали = R·n (нормализованные).
pub fn apply_mesh_transform(mesh: &mut Mesh, t: &MeshTransform) {
    let [rx, ry, rz] = t.rotate_euler;
    if t.scale == 1.0 && t.translate == [0.0; 3] && rx == 0.0 && ry == 0.0 && rz == 0.0 {
        return;
    }

    let rotate = |v: [f32; 3]| -> [f32; 3] {
        let v = rotate_vec(v, [1.0, 0.0, 0.0], rx);
        let v = rotate_vec(v, [0.0, 1.0, 0.0], ry);
        rotate_vec(v, [0.0, 0.0, 1.0], rz)
    };

    for p in &mut mesh.positions {
        let scaled = [p[0] * t.scale, p[1] * t.scale, p[2] * t.scale];
        let r = rotate(scaled);
        p[0] = r[0] + t.translate[0];
        p[1] = r[1] + t.translate[1];
        p[2] = r[2] + t.translate[2];
    }

    for n in &mut mesh.normals {
        let r = rotate(*n);
        let len = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
        if len > 1e-6 {
            *n = [r[0] / len, r[1] / len, r[2] / len];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scale_only() {
        let t = parse_mesh_transform("scale=0.01");
        assert_eq!(t.scale, 0.01);
        assert_eq!(t.translate, [0.0; 3]);
        assert_eq!(t.rotate_euler, [0.0; 3]);
    }

    #[test]
    fn parse_full_query() {
        let t = parse_mesh_transform("scale=0.01&translate=1,2,3&rotate=90,0,0");
        assert_eq!(t.scale, 0.01);
        assert_eq!(t.translate, [1.0, 2.0, 3.0]);
        assert_eq!(t.rotate_euler, [90.0, 0.0, 0.0]);
    }

    #[test]
    fn parse_single_rotates_z() {
        let t = parse_mesh_transform("rotate=45");
        assert_eq!(t.rotate_euler, [0.0, 0.0, 45.0]);
    }

    #[test]
    fn parse_ignores_bad_values() {
        let t = parse_mesh_transform("scale=abc&translate=1&rotate=xyz");
        assert_eq!(t.scale, 1.0);
        assert_eq!(t.translate, [0.0; 3]);
        assert_eq!(t.rotate_euler, [0.0; 3]);
    }

    #[test]
    fn apply_scale_and_translate() {
        let mut m = Mesh {
            positions: vec![[1.0, 0.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]],
            uvs: vec![[0.0, 0.0]],
            lightmap_uvs: Vec::new(),
            lightmap_styles: Vec::new(),
            atlas_rects: Vec::new(),
            indices: vec![0],
            content_id: 0,
        };
        apply_mesh_transform(&mut m, &MeshTransform {
            scale: 2.0,
            translate: [10.0, 0.0, 0.0],
            rotate_euler: [0.0; 3],
        });
        assert_eq!(m.positions[0], [12.0, 0.0, 0.0]);
        assert_eq!(m.normals[0], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn apply_rotate_90_x() {
        let mut m = Mesh {
            positions: vec![[0.0, 0.0, 1.0]],
            normals: vec![[0.0, 0.0, 1.0]],
            uvs: vec![[0.0, 0.0]],
            lightmap_uvs: Vec::new(),
            lightmap_styles: Vec::new(),
            atlas_rects: Vec::new(),
            indices: vec![0],
            content_id: 0,
        };
        apply_mesh_transform(&mut m, &MeshTransform {
            scale: 1.0,
            translate: [0.0; 3],
            rotate_euler: [90.0, 0.0, 0.0],
        });
        assert!((m.positions[0][0] - 0.0).abs() < 1e-5);
        assert!((m.positions[0][1] - -1.0).abs() < 1e-5);
        assert!((m.positions[0][2] - 0.0).abs() < 1e-5);
        assert!((m.normals[0][1] - -1.0).abs() < 1e-5);
    }
}

