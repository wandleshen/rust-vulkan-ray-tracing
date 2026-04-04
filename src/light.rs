use glam::{Vec3, vec3};
use std::f32::consts::PI;

pub const MAX_LIGHTS: usize = 8;

pub fn point_light_position() -> Vec3 {
    vec3(4.0, 6.5, 2.0)
}

pub fn point_light_radius() -> f32 {
    0.35
}

pub fn point_light_emission() -> Vec3 {
    vec3(18.0, 16.0, 14.0)
}

pub fn area_light_position() -> Vec3 {
    vec3(0.0, 6.0, 0.0)
}

pub fn area_light_radius() -> f32 {
    1.75
}

pub fn area_light_emission() -> Vec3 {
    vec3(10.0, 9.4, 8.8)
}

pub fn sky_sun_direction() -> Vec3 {
    vec3(0.35, 0.88, 0.25).normalize()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightMode {
    Sky,
    Point,
    Directional,
    AreaSphere,
}

impl LightMode {
    pub fn label(self) -> &'static str {
        match self {
            LightMode::Sky => "Sky",
            LightMode::Point => "Point",
            LightMode::Directional => "Directional",
            LightMode::AreaSphere => "Area Sphere",
        }
    }

    pub fn type_id(self) -> f32 {
        match self {
            LightMode::Sky => 0.0,
            LightMode::Point => 1.0,
            LightMode::Directional => 2.0,
            LightMode::AreaSphere => 3.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DemoLightState {
    pub sky_enabled: bool,
    pub point_enabled: bool,
    pub directional_enabled: bool,
    pub area_enabled: bool,
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct GpuLight {
    pub position_radius: [f32; 4],
    pub direction_type: [f32; 4],
    pub emission_pmf: [f32; 4],
    pub params: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct LightUniform {
    pub meta: [f32; 4],
    pub lights: [GpuLight; MAX_LIGHTS],
}

impl Default for LightUniform {
    fn default() -> Self {
        Self {
            meta: [0.0; 4],
            lights: [GpuLight::default(); MAX_LIGHTS],
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LightBuildData {
    mode: LightMode,
    position: Vec3,
    radius: f32,
    direction: Vec3,
    emission: Vec3,
    visible_emissive: bool,
    power: f32,
}

fn luminance(color: Vec3) -> f32 {
    0.2126 * color.x + 0.7152 * color.y + 0.0722 * color.z
}

fn estimate_environment_power(average_luminance: f32) -> f32 {
    (average_luminance.max(0.02) * 4.0 * PI).max(1.0e-4)
}

fn estimate_point_power(emission: Vec3) -> f32 {
    (luminance(emission) * 4.0 * PI).max(1.0e-4)
}

fn estimate_directional_power(emission: Vec3) -> f32 {
    (luminance(emission) * 8.0).max(1.0e-4)
}

fn estimate_area_sphere_power(emission: Vec3, radius: f32) -> f32 {
    let area = 4.0 * PI * radius * radius;
    (luminance(emission) * area).max(1.0e-4)
}

impl DemoLightState {
    pub fn new() -> Self {
        Self {
            sky_enabled: true,
            point_enabled: true,
            directional_enabled: true,
            area_enabled: true,
        }
    }

    pub fn is_enabled(self, mode: LightMode) -> bool {
        match mode {
            LightMode::Sky => self.sky_enabled,
            LightMode::Point => self.point_enabled,
            LightMode::Directional => self.directional_enabled,
            LightMode::AreaSphere => self.area_enabled,
        }
    }

    pub fn toggle(&mut self, mode: LightMode) -> bool {
        match mode {
            LightMode::Sky => self.sky_enabled = !self.sky_enabled,
            LightMode::Point => self.point_enabled = !self.point_enabled,
            LightMode::Directional => self.directional_enabled = !self.directional_enabled,
            LightMode::AreaSphere => self.area_enabled = !self.area_enabled,
        }
        true
    }

    pub fn summary(self) -> String {
        format!(
            "Sky:{} Point:{} Directional:{} Area:{}",
            if self.sky_enabled { "on" } else { "off" },
            if self.point_enabled { "on" } else { "off" },
            if self.directional_enabled {
                "on"
            } else {
                "off"
            },
            if self.area_enabled { "on" } else { "off" },
        )
    }

    pub fn build_uniform(self, environment_average_luminance: f32) -> LightUniform {
        let mut light_builds = Vec::with_capacity(MAX_LIGHTS);

        if self.sky_enabled {
            light_builds.push(LightBuildData {
                mode: LightMode::Sky,
                position: Vec3::ZERO,
                radius: 0.0,
                direction: sky_sun_direction(),
                emission: Vec3::ONE,
                visible_emissive: false,
                power: estimate_environment_power(environment_average_luminance),
            });
        }

        if self.point_enabled {
            light_builds.push(LightBuildData {
                mode: LightMode::Point,
                position: point_light_position(),
                radius: point_light_radius(),
                direction: Vec3::ZERO,
                emission: point_light_emission(),
                visible_emissive: false,
                power: estimate_point_power(point_light_emission()),
            });
        }

        if self.directional_enabled {
            let emission = vec3(5.5, 5.2, 4.8);
            light_builds.push(LightBuildData {
                mode: LightMode::Directional,
                position: Vec3::ZERO,
                radius: 0.0,
                direction: vec3(0.35, -1.0, 0.25).normalize(),
                emission,
                visible_emissive: false,
                power: estimate_directional_power(emission),
            });
        }

        if self.area_enabled {
            light_builds.push(LightBuildData {
                mode: LightMode::AreaSphere,
                position: area_light_position(),
                radius: area_light_radius(),
                direction: Vec3::ZERO,
                emission: area_light_emission(),
                visible_emissive: true,
                power: estimate_area_sphere_power(area_light_emission(), area_light_radius()),
            });
        }

        let count = light_builds.len().min(MAX_LIGHTS);
        if count == 0 {
            return LightUniform::default();
        }

        let total_power = light_builds
            .iter()
            .take(count)
            .map(|light| light.power)
            .sum::<f32>()
            .max(1.0e-6);

        let mut cdf = 0.0f32;
        let mut lights = [GpuLight::default(); MAX_LIGHTS];
        for (index, light) in light_builds.iter().take(count).enumerate() {
            let pmf = (light.power / total_power).max(1.0e-6);
            cdf = (cdf + pmf).min(1.0);

            lights[index] = GpuLight {
                position_radius: [
                    light.position.x,
                    light.position.y,
                    light.position.z,
                    light.radius,
                ],
                direction_type: [
                    light.direction.x,
                    light.direction.y,
                    light.direction.z,
                    light.mode.type_id(),
                ],
                emission_pmf: [light.emission.x, light.emission.y, light.emission.z, pmf],
                params: [
                    cdf,
                    if light.visible_emissive { 1.0 } else { 0.0 },
                    light.power,
                    0.0,
                ],
            };
        }

        LightUniform {
            meta: [
                count as f32,
                total_power,
                environment_average_luminance,
                0.0,
            ],
            lights,
        }
    }
}

pub fn default_demo_light() -> DemoLightState {
    DemoLightState::new()
}

pub fn key_to_light_mode(key: glfw::Key) -> Option<LightMode> {
    match key {
        glfw::Key::Num1 => Some(LightMode::Sky),
        glfw::Key::Num2 => Some(LightMode::Point),
        glfw::Key::Num3 => Some(LightMode::Directional),
        glfw::Key::Num4 => Some(LightMode::AreaSphere),
        _ => None,
    }
}
