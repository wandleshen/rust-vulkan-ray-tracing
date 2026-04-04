use glam::{Vec3A, Vec4, vec4};

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Material {
    pub base_color: Vec4,
    pub emission: Vec4,
    pub params: Vec4,
    pub medium: Vec4,
}

impl Material {
    pub fn diffuse(base_color: Vec3A) -> Self {
        Self {
            base_color: base_color.extend(1.0).into(),
            emission: Vec4::ZERO,
            params: vec4(1.0, 0.0, 0.0, 1.5),
            medium: Vec4::ZERO,
        }
    }

    pub fn metal(base_color: Vec3A, roughness: f32) -> Self {
        Self {
            base_color: base_color.extend(1.0).into(),
            emission: Vec4::ZERO,
            params: vec4(roughness.clamp(0.02, 1.0), 1.0, 0.0, 1.5),
            medium: Vec4::ZERO,
        }
    }

    pub fn dielectric(ior: f32) -> Self {
        Self {
            base_color: vec4(1.0, 1.0, 1.0, 1.0),
            emission: Vec4::ZERO,
            params: vec4(0.0, 0.0, 1.0, ior),
            medium: Vec4::ZERO,
        }
    }

    pub fn emissive(base_color: Vec3A, emission: Vec3A, intensity: f32) -> Self {
        Self {
            base_color: base_color.extend(1.0).into(),
            emission: (emission * intensity).extend(1.0).into(),
            params: vec4(1.0, 0.0, 0.0, 1.5),
            medium: Vec4::ZERO,
        }
    }
}
