//! Константы координатной системы (Z-up: X=right, Y=forward, Z=up).

/// Ось X — вправо.
pub const RIGHT: [f32; 3] = [1.0, 0.0, 0.0];
/// Ось Y — вперёд.
pub const FORWARD: [f32; 3] = [0.0, 1.0, 0.0];
/// Ось Z — вверх.
pub const UP: [f32; 3] = [0.0, 0.0, 1.0];

/// Матрица4x4 (column-major, WGSL layout): view-projection.
///
/// Камера位于 `(pos_x, pos_y, pos_z)`,Yawaround Z, pitch around X.
/// pitch=0 → смотрит вперёд (+Y), pitch<0 → вниз, pitch>0 → вверх.
pub fn build_view_proj(
    pos_x: f32,
    pos_y: f32,
    pos_z: f32,
    yaw: f32,
    pitch: f32,
) -> [[f32; 4]; 4] {
    let aspect = crate::frame::frame_width() as f32 / crate::frame::frame_height() as f32;

    // pitch=0 → forward (+Y). Rx(π/2) поворачивает из -Z в +Y.
    let ep = pitch + std::f32::consts::FRAC_PI_2;
    let cp = ep.cos();
    let sp = ep.sin();
    let cy = yaw.cos();
    let sy = yaw.sin();

    // Translation matrix (column-major): T(-pos)
    let t: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-pos_x, -pos_y, -pos_z, 1.0],
    ];

    // Ry(yaw) rotation around Z axis (column-major, Z-up)
    let ry: [[f32; 4]; 4] = [
        [ cy, -sy, 0.0, 0.0],
        [ sy,  cy, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    // Rx(-ep) rotation around X axis (column-major)
    let rx: [[f32; 4]; 4] = [
        [1.0,  0.0,  0.0, 0.0],
        [0.0,  cp,  -sp,  0.0],
        [0.0,  sp,   cp,  0.0],
        [0.0,  0.0,  0.0, 1.0],
    ];

    let vt = mat4_mul(&rx, &mat4_mul(&ry, &t));

    // Perspective projection (column-major, wgpu [0..1] depth range)
    let fov_y_half = (60.0_f32).to_radians() * 0.5;
    let f = 1.0 / fov_y_half.tan();
    let near = 0.1;
    let far = 1024.0;
    let proj: [[f32; 4]; 4] = [
        [f / aspect, 0.0,  0.0,  0.0],
        [0.0,        f,    0.0,  0.0],
        [0.0,        0.0,  far / (near - far), -1.0],
        [0.0,        0.0,  far * near / (near - far), 0.0],
    ];

    mat4_mul(&proj, &vt)
}

/// Умножение4x4 матриц (column-major): C = A * B.
pub fn mat4_mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            out[col][row] = a[0][row] * b[col][0]
                + a[1][row] * b[col][1]
                + a[2][row] * b[col][2]
                + a[3][row] * b[col][3];
        }
    }
    out
}

/// Умножение матрицы4x4 на вектор (column-major).
pub fn mat4_mul_vec4(m: &[[f32; 4]; 4], v: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0f32; 4];
    for row in 0..4 {
        out[row] = m[0][row] * v[0] + m[1][row] * v[1] + m[2][row] * v[2] + m[3][row] * v[3];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_recovers_identity() {
        let m = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let inv = mat4_inverse(m);
        let prod = mat4_mul(&m, &inv);
        for col in 0..4 {
            for row in 0..4 {
                let expected = if col == row { 1.0 } else { 0.0 };
                assert!(
                    (prod[col][row] - expected).abs() < 1e-5,
                    "prod[{col}][{row}] = {}",
                    prod[col][row]
                );
            }
        }
    }

    #[test]
    fn inverse_of_scale() {
        let m = [
            [2.0, 0.0, 0.0, 0.0],
            [0.0, 3.0, 0.0, 0.0],
            [0.0, 0.0, 4.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let inv = mat4_inverse(m);
        let prod = mat4_mul(&m, &inv);
        for col in 0..4 {
            for row in 0..4 {
                let expected = if col == row { 1.0 } else { 0.0 };
                assert!(
                    (prod[col][row] - expected).abs() < 1e-5,
                    "prod[{col}][{row}] = {}",
                    prod[col][row]
                );
            }
        }
    }

    /// Ground points in front of the camera must project to clip-space w > 0.
    #[test]
    fn view_proj_ground_in_front() {
        let vp = build_view_proj(64.0, 64.0, 20.0, 0.0, 0.0);
        // Point 10 m ahead on the ground (yaw=0 → +Y is forward)
        let p = [64.0, 74.0, 0.0, 1.0];
        let clip = mat4_mul_vec4(&vp, p);
        let w = clip[3];
        assert!(w > 0.0, "w must be positive, got {w}");
        let ndc_z = clip[2] / w;
        assert!(
            ndc_z >= 0.0 && ndc_z <= 1.0,
            "ground ndc_z must be in [0,1], got {ndc_z}"
        );
    }
}

/// Умножение матрицы4x4 на вектор (column-major).
/// Алгоритм — cofactor (glm::inverse), column-major `m[col][row]`.
pub fn mat4_inverse(m: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let coef00 = m[2][2] * m[3][3] - m[3][2] * m[2][3];
    let coef02 = m[1][2] * m[3][3] - m[3][2] * m[1][3];
    let coef03 = m[1][2] * m[2][3] - m[2][2] * m[1][3];
    let coef04 = m[2][1] * m[3][3] - m[3][1] * m[2][3];
    let coef06 = m[1][1] * m[3][3] - m[3][1] * m[1][3];
    let coef07 = m[1][1] * m[2][3] - m[2][1] * m[1][3];
    let coef08 = m[2][1] * m[3][2] - m[3][1] * m[2][2];
    let coef10 = m[1][1] * m[3][2] - m[3][1] * m[1][2];
    let coef11 = m[1][1] * m[2][2] - m[2][1] * m[1][2];
    let coef12 = m[2][0] * m[3][3] - m[3][0] * m[2][3];
    let coef14 = m[1][0] * m[3][3] - m[3][0] * m[1][3];
    let coef15 = m[1][0] * m[2][3] - m[2][0] * m[1][3];
    let coef16 = m[2][0] * m[3][2] - m[3][0] * m[2][2];
    let coef18 = m[1][0] * m[3][2] - m[3][0] * m[1][2];
    let coef19 = m[1][0] * m[2][2] - m[2][0] * m[1][2];
    let coef20 = m[2][0] * m[3][1] - m[3][0] * m[2][1];
    let coef22 = m[1][0] * m[3][1] - m[3][0] * m[1][1];
    let coef23 = m[1][0] * m[2][1] - m[2][0] * m[1][1];

    let fac0 = [coef00, coef00, coef02, coef03];
    let fac1 = [coef04, coef04, coef06, coef07];
    let fac2 = [coef08, coef08, coef10, coef11];
    let fac3 = [coef12, coef12, coef14, coef15];
    let fac4 = [coef16, coef16, coef18, coef19];
    let fac5 = [coef20, coef20, coef22, coef23];

    let vec0 = [m[1][0], m[0][0], m[0][0], m[0][0]];
    let vec1 = [m[1][1], m[0][1], m[0][1], m[0][1]];
    let vec2 = [m[1][2], m[0][2], m[0][2], m[0][2]];
    let vec3 = [m[1][3], m[0][3], m[0][3], m[0][3]];

    let inv0 = [
        vec1[0] * fac0[0] - vec2[0] * fac1[0] + vec3[0] * fac2[0],
        vec1[1] * fac0[1] - vec2[1] * fac1[1] + vec3[1] * fac2[1],
        vec1[2] * fac0[2] - vec2[2] * fac1[2] + vec3[2] * fac2[2],
        vec1[3] * fac0[3] - vec2[3] * fac1[3] + vec3[3] * fac2[3],
    ];
    let inv1 = [
        vec0[0] * fac0[0] - vec2[0] * fac3[0] + vec3[0] * fac4[0],
        vec0[1] * fac0[1] - vec2[1] * fac3[1] + vec3[1] * fac4[1],
        vec0[2] * fac0[2] - vec2[2] * fac3[2] + vec3[2] * fac4[2],
        vec0[3] * fac0[3] - vec2[3] * fac3[3] + vec3[3] * fac4[3],
    ];
    let inv2 = [
        vec0[0] * fac1[0] - vec1[0] * fac3[0] + vec3[0] * fac5[0],
        vec0[1] * fac1[1] - vec1[1] * fac3[1] + vec3[1] * fac5[1],
        vec0[2] * fac1[2] - vec1[2] * fac3[2] + vec3[2] * fac5[2],
        vec0[3] * fac1[3] - vec1[3] * fac3[3] + vec3[3] * fac5[3],
    ];
    let inv3 = [
        vec0[0] * fac2[0] - vec1[0] * fac4[0] + vec2[0] * fac5[0],
        vec0[1] * fac2[1] - vec1[1] * fac4[1] + vec2[1] * fac5[1],
        vec0[2] * fac2[2] - vec1[2] * fac4[2] + vec2[2] * fac5[2],
        vec0[3] * fac2[3] - vec1[3] * fac4[3] + vec2[3] * fac5[3],
    ];

    let sign_a = [1.0, -1.0, 1.0, -1.0];
    let sign_b = [-1.0, 1.0, -1.0, 1.0];

    // Inverse columns: col0 = sign_a*inv0, col1 = sign_b*inv1, ...
    let mut inverse = [[0.0f32; 4]; 4];
    for i in 0..4 {
        inverse[0][i] = sign_a[i] * inv0[i];
        inverse[1][i] = sign_b[i] * inv1[i];
        inverse[2][i] = sign_a[i] * inv2[i];
        inverse[3][i] = sign_b[i] * inv3[i];
    }

    // Row0 = (Inverse[0][0], Inverse[1][0], Inverse[2][0], Inverse[3][0])
    let det = m[0][0] * inverse[0][0]
        + m[0][1] * inverse[1][0]
        + m[0][2] * inverse[2][0]
        + m[0][3] * inverse[3][0];
    if det == 0.0 {
        return m;
    }
    let one_over_det = 1.0 / det;
    for col in 0..4 {
        for row in 0..4 {
            inverse[col][row] *= one_over_det;
        }
    }
    inverse
}

/// Rebase `inv(view*proj)` so unprojected points are camera-relative (stable at large world XY).
///
/// Prefer [`inv_view_proj_sky_at_origin`] for fullscreen sky rays: multiplying by `T(-eye)` after
/// `inv(VP)` amplifies f32 error when `eye` is far from the origin, which shows up as sky-only shake.
pub fn inv_view_proj_camera_relative(
    inv_view_proj: [[f32; 4]; 4],
    camera_world: [f32; 3],
) -> [[f32; 4]; 4] {
    let t: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-camera_world[0], -camera_world[1], -camera_world[2], 1.0],
    ];
    mat4_mul(&t, &inv_view_proj)
}

/// Inverse view-projection for sky / cloud fullscreen passes: same rotation as the camera, eye at origin.
pub fn inv_view_proj_sky_at_origin(view: glam::Mat4, projection: glam::Mat4) -> [[f32; 4]; 4] {
    let mut view_at_origin = view;
    view_at_origin.w_axis = glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
    (projection * view_at_origin)
        .inverse()
        .to_cols_array_2d()
}
