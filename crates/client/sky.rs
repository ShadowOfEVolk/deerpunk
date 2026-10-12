//! Hosek-Wilkie Skylight Model (RGB), порт с `hw-skymodel` crate.
//!
//! Коэффициенты (`params-*`, `radiances-*`) и формулы из статьи
//! "An Analytic Model for Full Spectral Sky-Dome Radiance" (Hosek & Wilkie),
//! данные — из референс-реализации (см. `sky/hosek-wilkie-license.txt`).
//!
//! Модель: 27 параметров + 3 масштаба радианса считаются на CPU из
//! (elevation, turbidity, albedo). Радианс для направления (`theta` — угол от
//! зенита, `gamma` — угол от солнца) считается простой формулой — её удобно
//! выносить в шейдер, а параметры передавать юниформом.

use std::f32::consts::{FRAC_PI_2, PI};

/// Масштаб радианса: HW выдаёт значения ~1..130 cd/м², приводим к [0..1] после tonemap.
pub const SKY_EXPOSURE: f32 = 0.03;

/// Параметры неба для `SkyState::new`.
#[derive(Clone, Copy, Debug)]
pub struct SkyParams {
    /// Высота солнца над горизонтом, радианы. 0..=π/2.
    pub elevation: f32,
    /// Мутность атмосферы, 1..=10.
    pub turbidity: f32,
    /// Альбедо земли, 0..=1 (RGB).
    pub albedo: [f32; 3],
}

impl Default for SkyParams {
    fn default() -> Self {
        Self {
            elevation: 0.5,
            turbidity: 3.0,
            albedo: [0.2, 0.25, 0.3],
        }
    }
}

/// Состояние модели: 27 параметров (9 на канал R,G,B) + 3 радианса.
#[derive(Clone, Copy, Debug)]
pub struct SkyState {
    params: [f32; 27],
    radiances: [f32; 3],
}

fn load_dataset(path: &str) -> &'static [f32] {
    static PARAMS_R: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    static PARAMS_G: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    static PARAMS_B: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    static RADI_R: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    static RADI_G: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    static RADI_B: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();

    let parse = |bytes: &'static [u8]| -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    };

    let slot: &std::sync::OnceLock<Vec<f32>> = match path {
        "params-r" => &PARAMS_R,
        "params-g" => &PARAMS_G,
        "params-b" => &PARAMS_B,
        "radiances-r" => &RADI_R,
        "radiances-g" => &RADI_G,
        _ => &RADI_B,
    };
    slot.get_or_init(|| {
        let bytes: &'static [u8] = match path {
            "params-r" => include_bytes!("sky/params-r"),
            "params-g" => include_bytes!("sky/params-g"),
            "params-b" => include_bytes!("sky/params-b"),
            "radiances-r" => include_bytes!("sky/radiances-r"),
            "radiances-g" => include_bytes!("sky/radiances-g"),
            _ => include_bytes!("sky/radiances-b"),
        };
        parse(bytes)
    })
}

impl SkyState {
    pub fn new(p: &SkyParams) -> Self {
        let elevation = p.elevation.clamp(0.0, FRAC_PI_2);
        let turbidity = p.turbidity.clamp(1.0, 10.0);
        let albedo = p.albedo;
        let t = (elevation / (0.5 * PI)).powf(1.0 / 3.0);

        let params_r = load_dataset("params-r");
        let params_g = load_dataset("params-g");
        let params_b = load_dataset("params-b");
        let radiances_r = load_dataset("radiances-r");
        let radiances_g = load_dataset("radiances-g");
        let radiances_b = load_dataset("radiances-b");

        let mut params = [0.0f32; 27];
        init_params(&mut params[..], params_r, turbidity, albedo[0], t);
        init_params(&mut params[9..], params_g, turbidity, albedo[1], t);
        init_params(&mut params[18..], params_b, turbidity, albedo[2], t);

        let mut radiances = [0.0f32; 3];
        init_radiance(&mut radiances[0], radiances_r, turbidity, albedo[0], t);
        init_radiance(&mut radiances[1], radiances_g, turbidity, albedo[1], t);
        init_radiance(&mut radiances[2], radiances_b, turbidity, albedo[2], t);

        Self { params, radiances }
    }

    /// Радианс канала для направления: `theta` — угол от зенита, `gamma` — угол от солнца.
    pub fn radiance(&self, theta: f32, gamma: f32, channel: usize) -> f32 {
        let p = &self.params[(9 * channel)..];
        let r = self.radiances[channel];
        let cos_gamma = gamma.cos();
        let cos_gamma2 = cos_gamma * cos_gamma;
        let cos_theta = theta.cos().abs();

        let exp_m = (p[4] * gamma).exp();
        let ray_m = cos_gamma2;
        let mie_m_lhs = 1.0 + cos_gamma2;
        let mie_m_rhs = (1.0 + p[8] * p[8] - 2.0 * p[8] * cos_gamma).powf(1.5);
        let mie_m = mie_m_lhs / mie_m_rhs;
        let zenith = cos_theta.sqrt();
        let radiance_lhs = 1.0 + p[0] * (p[1] / (cos_theta + 0.01)).exp();
        let radiance_rhs = p[2] + p[3] * exp_m + p[5] * ray_m + p[6] * mie_m + p[7] * zenith;
        r * radiance_lhs * radiance_rhs
    }

    /// 30 чисел для шейдера: [27 параметров R,G,B...], затем [3 радианса].
    pub fn to_uniform(&self) -> [f32; 30] {
        let mut out = [0.0f32; 30];
        out[..27].copy_from_slice(&self.params);
        out[27..30].copy_from_slice(&self.radiances);
        out
    }
}

fn init_params(out_params: &mut [f32], dataset: &[f32], turbidity: f32, albedo: f32, t: f32) {
    let turbidity_int = turbidity.trunc() as usize;
    let turbidity_rem = turbidity.fract();
    let turbidity_min = turbidity_int.saturating_sub(1);
    let turbidity_max = turbidity_int.min(9);
    let p0 = &dataset[(9 * 6 * turbidity_min)..];
    let p1 = &dataset[(9 * 6 * turbidity_max)..];
    let p2 = &dataset[(9 * 6 * 10 + 9 * 6 * turbidity_min)..];
    let p3 = &dataset[(9 * 6 * 10 + 9 * 6 * turbidity_max)..];
    let s0 = (1.0 - albedo) * (1.0 - turbidity_rem);
    let s1 = (1.0 - albedo) * turbidity_rem;
    let s2 = albedo * (1.0 - turbidity_rem);
    let s3 = albedo * turbidity_rem;

    for i in 0..9 {
        out_params[i] += s0 * quintic::<9>(&p0[i..], t);
        out_params[i] += s1 * quintic::<9>(&p1[i..], t);
        out_params[i] += s2 * quintic::<9>(&p2[i..], t);
        out_params[i] += s3 * quintic::<9>(&p3[i..], t);
    }
}

fn init_radiance(out_radiance: &mut f32, dataset: &[f32], turbidity: f32, albedo: f32, t: f32) {
    let turbidity_int = turbidity.trunc() as usize;
    let turbidity_rem = turbidity.fract();
    let turbidity_min = turbidity_int.saturating_sub(1);
    let turbidity_max = turbidity_int.min(9);
    let p0 = &dataset[(6 * turbidity_min)..];
    let p1 = &dataset[(6 * turbidity_max)..];
    let p2 = &dataset[(6 * 10 + 6 * turbidity_min)..];
    let p3 = &dataset[(6 * 10 + 6 * turbidity_max)..];
    let s0 = (1.0 - albedo) * (1.0 - turbidity_rem);
    let s1 = (1.0 - albedo) * turbidity_rem;
    let s2 = albedo * (1.0 - turbidity_rem);
    let s3 = albedo * turbidity_rem;

    *out_radiance += s0 * quintic::<1>(p0, t);
    *out_radiance += s1 * quintic::<1>(p1, t);
    *out_radiance += s2 * quintic::<1>(p2, t);
    *out_radiance += s3 * quintic::<1>(p3, t);
}

fn quintic<const STRIDE: usize>(p: &[f32], t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let t4 = t2 * t2;
    let t5 = t4 * t;

    let inv_t = 1.0 - t;
    let inv_t2 = inv_t * inv_t;
    let inv_t3 = inv_t2 * inv_t;
    let inv_t4 = inv_t2 * inv_t2;
    let inv_t5 = inv_t4 * inv_t;

    let m0 = p[0] * inv_t5;
    let m1 = p[STRIDE] * 5.0 * inv_t4 * t;
    let m2 = p[2 * STRIDE] * 10.0 * inv_t3 * t2;
    let m3 = p[3 * STRIDE] * 10.0 * inv_t2 * t3;
    let m4 = p[4 * STRIDE] * 5.0 * inv_t * t4;
    let m5 = p[5 * STRIDE] * t5;

    m0 + m1 + m2 + m3 + m4 + m5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radiance_finite_and_positive() {
        for elevation in [0.05, 0.3, 0.8, 1.3] {
            let state = SkyState::new(&SkyParams {
                elevation,
                turbidity: 3.0,
                albedo: [0.2, 0.25, 0.3],
            });
            for channel in 0..3 {
                for &(theta, gamma) in &[(0.1, 0.1), (1.0, 0.5), (1.5, 1.5)] {
                    let v = state.radiance(theta, gamma, channel);
                    assert!(v.is_finite(), "theta={theta} gamma={gamma} ch={channel}");
                    assert!(v > 0.0, "theta={theta} gamma={gamma} ch={channel} v={v}");
                }
            }
        }
    }

    #[test]
    fn uniform_layout_has_no_nan() {
        let state = SkyState::new(&SkyParams::default());
        let u = state.to_uniform();
        assert!(u.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn sun_is_brighter_than_sky() {
        // Рядом с солнцем (gamma мал) ярче, чем на 60° от солнца.
        let state = SkyState::new(&SkyParams {
            elevation: 1.0,
            turbidity: 2.0,
            albedo: [0.2, 0.25, 0.3],
        });
        let near_sun = state.radiance(1.0, 0.05, 0);
        let far = state.radiance(1.0, 1.5, 0);
        assert!(near_sun > far, "near={near_sun} far={far}");
    }


}
