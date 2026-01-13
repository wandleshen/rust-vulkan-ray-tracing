use glam::Vec3A;

/// 材质类型枚举
#[derive(Clone, Copy)]
#[repr(C)]
pub struct EnumMaterial {
    pub t: u32,
    pub data: glam::Vec4,
}

impl EnumMaterial {
    /// 创建漫反射材质
    pub fn lambertian(albedo: Vec3A) -> Self {
        Self {
            t: 0,
            data: albedo.extend(0.0).into(),
        }
    }

    /// 创建金属材质
    pub fn metal(albedo: Vec3A, fuzz: f32) -> Self {
        Self {
            t: 1,
            data: albedo.extend(fuzz).into(),
        }
    }

    /// 创建电介质材质（玻璃）
    pub fn dielectric(ir: f32) -> Self {
        Self {
            t: 2,
            data: glam::vec4(ir, 0.0, 0.0, 0.0),
        }
    }
}
