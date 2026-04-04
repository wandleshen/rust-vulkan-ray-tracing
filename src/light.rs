use glam::{Vec3, Vec4, vec3, vec4};

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
    pub mode: LightMode,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct LightUniform {
    pub mode_and_params: [f32; 4],
    pub position_radius: [f32; 4],
    pub direction_range: [f32; 4],
    pub color_intensity: [f32; 4],
}

impl DemoLightState {
    pub fn new(mode: LightMode) -> Self {
        Self { mode }
    }

    pub fn build_uniform(self) -> LightUniform {
        match self.mode {
            LightMode::Sky => LightUniform {
                mode_and_params: [self.mode.type_id(), 1.0, 0.0, 0.0],
                position_radius: [0.0, 0.0, 0.0, 0.0],
                direction_range: [0.0, 0.0, 0.0, 0.0],
                color_intensity: [1.0, 1.0, 1.0, 1.0],
            },
            LightMode::Point => LightUniform {
                mode_and_params: [self.mode.type_id(), 1.0, 0.0, 0.0],
                position_radius: [4.0, 6.5, 2.0, 0.0],
                direction_range: [0.0, 0.0, 0.0, 0.0],
                color_intensity: [18.0, 16.0, 14.0, 1.0],
            },
            LightMode::Directional => LightUniform {
                mode_and_params: [self.mode.type_id(), 1.0, 0.0, 0.0],
                position_radius: [0.0, 0.0, 0.0, 0.0],
                direction_range: [0.35, -1.0, 0.25, 0.0],
                color_intensity: [5.5, 5.2, 4.8, 1.0],
            },
            LightMode::AreaSphere => LightUniform {
                mode_and_params: [self.mode.type_id(), 1.0, 0.0, 0.0],
                position_radius: [0.0, 6.0, 0.0, 1.75],
                direction_range: [0.0, 0.0, 0.0, 0.0],
                color_intensity: [10.0, 9.4, 8.8, 1.0],
            },
        }
    }
}

pub fn default_demo_light() -> DemoLightState {
    DemoLightState::new(LightMode::Sky)
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

#[allow(dead_code)]
fn _keep_glam_imports_used() -> (Vec3, Vec4) {
    (vec3(0.0, 0.0, 0.0), vec4(0.0, 0.0, 0.0, 0.0))
}
