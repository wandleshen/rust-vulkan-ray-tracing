use crate::{
    area_light_position, area_light_radius, material::Material, point_light_position,
    point_light_radius,
};
use ash::vk;
use glam::{Vec3A, vec3a};
use rand::prelude::*;

pub struct SceneData {
    pub instances: Vec<vk::AccelerationStructureInstanceKHR>,
    pub materials: Vec<Material>,
    pub point_light_instance_index: usize,
    pub area_light_instance_index: usize,
    pub point_light_material_index: usize,
    pub area_light_material_index: usize,
}

pub fn create_sphere_instance(
    pos: Vec3A,
    size: f32,
    sphere_accel_handle: u64,
) -> vk::AccelerationStructureInstanceKHR {
    vk::AccelerationStructureInstanceKHR {
        transform: vk::TransformMatrixKHR {
            matrix: [
                size, 0.0, 0.0, pos.x, 0.0, size, 0.0, pos.y, 0.0, 0.0, size, pos.z,
            ],
        },
        instance_custom_index_and_mask: vk::Packed24_8::new(0, 0xff),
        instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
            0,
            vk::GeometryInstanceFlagsKHR::empty().as_raw() as u8,
        ),
        acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
            device_handle: sphere_accel_handle,
        },
    }
}

pub fn sample_scene(sphere_accel_handle: u64) -> SceneData {
    let mut rng = StdRng::from_os_rng();
    let mut world = Vec::new();

    // 地面
    world.push((
        create_sphere_instance(vec3a(0.0, -1000.0, 0.0), 1000.0, sphere_accel_handle),
        Material::diffuse(vec3a(0.5, 0.5, 0.5)),
    ));

    // 随机小球
    for a in -11..11 {
        for b in -11..11 {
            let center = vec3a(
                a as f32 + 0.9 * rng.random::<f32>(),
                0.2,
                b as f32 + 0.9 * rng.random::<f32>(),
            );

            let choose_mat: f32 = rng.random();

            if (center - vec3a(4.0, 0.2, 0.0)).length() > 0.9 {
                match choose_mat {
                    x if x < 0.8 => {
                        let albedo = vec3a(rng.random(), rng.random(), rng.random())
                            * vec3a(rng.random(), rng.random(), rng.random());

                        world.push((
                            create_sphere_instance(center, 0.3, sphere_accel_handle),
                            Material::diffuse(albedo),
                        ));
                    }
                    x if x < 0.95 => {
                        let albedo = vec3a(
                            rng.random_range(0.5..1.0),
                            rng.random_range(0.5..1.0),
                            rng.random_range(0.5..1.0),
                        );
                        let fuzz = rng.random_range(0.0..0.5);

                        world.push((
                            create_sphere_instance(center, 0.2, sphere_accel_handle),
                            Material::metal(albedo, fuzz),
                        ));
                    }
                    _ => world.push((
                        create_sphere_instance(center, 0.2, sphere_accel_handle),
                        Material::dielectric(1.5),
                    )),
                }
            }
        }
    }

    // 三个大球
    world.push((
        create_sphere_instance(vec3a(0.0, 1.0, 0.0), 1.0, sphere_accel_handle),
        Material::dielectric(1.5),
    ));

    world.push((
        create_sphere_instance(vec3a(-4.0, 1.0, 0.0), 1.0, sphere_accel_handle),
        Material::diffuse(vec3a(0.4, 0.2, 0.1)).with_clearcoat(0.35),
    ));

    world.push((
        create_sphere_instance(vec3a(4.0, 1.0, 0.0), 1.0, sphere_accel_handle),
        Material::metal(vec3a(0.7, 0.6, 0.5), 0.02),
    ));

    let point_light_instance_index = world.len();
    let point_light_material_index = world.len();
    world.push((
        create_sphere_instance(
            point_light_position().into(),
            point_light_radius(),
            sphere_accel_handle,
        ),
        Material::dielectric(1.0),
    ));

    let area_light_instance_index = world.len();
    let area_light_material_index = world.len();
    world.push((
        create_sphere_instance(
            area_light_position().into(),
            area_light_radius(),
            sphere_accel_handle,
        ),
        Material::dielectric(1.0),
    ));

    // 分离实例和材质
    let mut spheres = Vec::new();
    let mut materials = Vec::new();

    for (i, (mut sphere, material)) in world.into_iter().enumerate() {
        sphere.instance_custom_index_and_mask =
            vk::Packed24_8::new(i as u32, sphere.instance_custom_index_and_mask.high_8());
        spheres.push(sphere);
        materials.push(material);
    }

    SceneData {
        instances: spheres,
        materials,
        point_light_instance_index,
        area_light_instance_index,
        point_light_material_index,
        area_light_material_index,
    }
}
