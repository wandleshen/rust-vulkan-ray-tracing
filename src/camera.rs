use glam::{Vec3, vec3};

const WORLD_UP: Vec3 = Vec3::Y;
const PITCH_LIMIT: f32 = 1.55334306;

#[derive(Clone, Copy, Debug)]
pub struct CameraState {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub vertical_fov_radians: f32,
    pub aperture: f32,
    pub focus_distance: f32,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct FrameUniform {
    pub origin: [f32; 4],
    pub lower_left_corner: [f32; 4],
    pub horizontal: [f32; 4],
    pub vertical: [f32; 4],
    pub basis_u_lens_radius: [f32; 4],
    pub basis_v_padding: [f32; 4],
}

impl CameraState {
    pub fn from_look_at(
        look_from: Vec3,
        look_at: Vec3,
        vertical_fov_degrees: f32,
        aperture: f32,
    ) -> Self {
        let forward = (look_at - look_from).normalize();

        Self {
            position: look_from,
            yaw: forward.z.atan2(forward.x),
            pitch: forward.y.asin(),
            vertical_fov_radians: vertical_fov_degrees.to_radians(),
            aperture,
            focus_distance: (look_at - look_from).length(),
        }
    }

    pub fn forward(self) -> Vec3 {
        let cos_pitch = self.pitch.cos();
        vec3(
            cos_pitch * self.yaw.cos(),
            self.pitch.sin(),
            cos_pitch * self.yaw.sin(),
        )
        .normalize()
    }

    pub fn right(self) -> Vec3 {
        self.forward().cross(WORLD_UP).normalize()
    }

    pub fn up(self) -> Vec3 {
        self.right().cross(self.forward()).normalize()
    }

    pub fn translate(&mut self, delta: Vec3) {
        self.position += delta;
    }

    pub fn rotate(&mut self, yaw_delta: f32, pitch_delta: f32) {
        self.yaw += yaw_delta;
        self.pitch = (self.pitch + pitch_delta).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    pub fn build_uniform(self, aspect_ratio: f32) -> FrameUniform {
        let half_height = (self.vertical_fov_radians * 0.5).tan();
        let viewport_height = 2.0 * half_height;
        let viewport_width = aspect_ratio * viewport_height;

        let forward = self.forward();
        let basis_u = self.right();
        let basis_v = self.up();

        let horizontal = self.focus_distance * viewport_width * basis_u;
        let vertical = self.focus_distance * viewport_height * basis_v;
        let lower_left_corner =
            self.position - horizontal * 0.5 - vertical * 0.5 + self.focus_distance * forward;

        FrameUniform {
            origin: [self.position.x, self.position.y, self.position.z, 0.0],
            lower_left_corner: [
                lower_left_corner.x,
                lower_left_corner.y,
                lower_left_corner.z,
                0.0,
            ],
            horizontal: [horizontal.x, horizontal.y, horizontal.z, 0.0],
            vertical: [vertical.x, vertical.y, vertical.z, 0.0],
            basis_u_lens_radius: [basis_u.x, basis_u.y, basis_u.z, self.aperture * 0.5],
            basis_v_padding: [basis_v.x, basis_v.y, basis_v.z, 0.0],
        }
    }
}
