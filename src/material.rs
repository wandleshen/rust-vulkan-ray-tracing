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
    pub fn surface(
        base_color: Vec3A,
        roughness: f32,
        metallic: f32,
        transmission: f32,
        ior: f32,
    ) -> Self {
        Self {
            base_color: base_color.extend(1.0).into(),
            emission: vec4(0.0, 0.0, 0.0, 0.0),
            params: vec4(
                roughness.clamp(0.02, 1.0),
                metallic.clamp(0.0, 1.0),
                transmission.clamp(0.0, 1.0),
                ior.max(1.0),
            ),
            medium: Vec4::ZERO,
        }
    }

    pub fn diffuse(base_color: Vec3A) -> Self {
        Self::surface(base_color, 1.0, 0.0, 0.0, 1.5)
    }

    pub fn metal(base_color: Vec3A, roughness: f32) -> Self {
        Self::surface(base_color, roughness, 1.0, 0.0, 1.5)
    }

    pub fn dielectric(ior: f32) -> Self {
        Self::surface(Vec3A::ONE, 0.02, 0.0, 1.0, ior)
    }

    pub fn emissive(base_color: Vec3A, emission: Vec3A, intensity: f32) -> Self {
        Self {
            base_color: base_color.extend(1.0).into(),
            emission: (emission * intensity).extend(0.0).into(),
            params: vec4(1.0, 0.0, 0.0, 1.5),
            medium: Vec4::ZERO,
        }
    }

    pub fn invisible() -> Self {
        Self {
            base_color: vec4(0.0, 0.0, 0.0, 0.0),
            emission: Vec4::ZERO,
            params: vec4(1.0, 0.0, 0.0, 1.5),
            medium: vec4(0.0, 0.0, 0.0, -1.0),
        }
    }

    pub fn with_roughness(mut self, roughness: f32) -> Self {
        self.params.x = roughness.clamp(0.02, 1.0);
        self
    }

    pub fn with_specular(mut self, specular: f32) -> Self {
        self.base_color.w = specular.clamp(0.0, 1.0);
        self
    }

    pub fn with_clearcoat(mut self, clearcoat: f32) -> Self {
        self.emission.w = clearcoat.clamp(0.0, 1.0);
        self
    }

    pub fn with_absorption(mut self, absorption: Vec3A, density: f32) -> Self {
        self.medium = absorption.extend(density.max(0.0)).into();
        self
    }
}
