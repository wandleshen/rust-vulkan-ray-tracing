//! Vulkan Ray Tracing 娑撹崵鈻兼惔?
use vulkan_raytracing::*;

use std::{
    thread,
    time::{Duration, Instant},
};

use ash::{khr, vk};
use glam::{Vec3, vec3, vec3a};
use glfw::{Action, CursorMode, Key};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

const HEADLESS_MODE: bool = false;
const FRAME_DELAY_MS: u64 = 0;

const WIDTH: u32 = 1200;
const HEIGHT: u32 = 800;
const COLOR_FORMAT: vk::Format = vk::Format::R32G32B32A32_SFLOAT;

const N_SAMPLES: u32 = 5000;
const N_SAMPLES_ITER: u32 = 100;
const WINDOW_SAMPLES_PER_FRAME: u32 = 1;
const DENOISE_ATROUS_PASSES: u32 = 1;

const CAMERA_MOVE_SPEED: f32 = 4.0;
const CAMERA_BOOST_MULTIPLIER: f32 = 3.0;
const CAMERA_MOUSE_SENSITIVITY: f32 = 0.0025;

fn is_key_down(window: &glfw::PWindow, key: Key) -> bool {
    matches!(window.get_key(key), Action::Press | Action::Repeat)
}

fn update_light_from_input(
    window: &glfw::PWindow,
    light_state: &mut DemoLightState,
    key_latches: &mut [bool; 4],
) -> bool {
    let key_modes = [
        (Key::Num1, LightMode::Sky),
        (Key::Num2, LightMode::Point),
        (Key::Num3, LightMode::Directional),
        (Key::Num4, LightMode::AreaSphere),
    ];

    let mut changed = false;
    for (index, (key, mode)) in key_modes.into_iter().enumerate() {
        let is_down = is_key_down(window, key);
        if is_down && !key_latches[index] {
            changed |= light_state.toggle(mode);
        }
        key_latches[index] = is_down;
    }

    changed
}

fn update_demo_light_materials(
    materials: &mut [Material],
    point_light_material_index: usize,
    area_light_material_index: usize,
    light_state: DemoLightState,
) {
    materials[point_light_material_index] = Material::invisible();

    materials[area_light_material_index] = if light_state.area_enabled {
        Material::emissive(vec3a(1.0, 0.98, 0.95), area_light_emission().into(), 1.0)
    } else {
        Material::invisible()
    };
}

fn set_instance_mask(instance: &mut vk::AccelerationStructureInstanceKHR, mask: u8) -> bool {
    if instance.instance_custom_index_and_mask.high_8() == mask {
        return false;
    }

    instance.instance_custom_index_and_mask =
        vk::Packed24_8::new(instance.instance_custom_index_and_mask.low_24(), mask);
    true
}

fn sync_light_instance_visibility(scene: &mut SceneData, light_state: DemoLightState) -> bool {
    let mut changed = false;
    changed |= set_instance_mask(&mut scene.instances[scene.point_light_instance_index], 0x00);

    let area_mask = if light_state.area_enabled { 0xff } else { 0x00 };
    changed |= set_instance_mask(
        &mut scene.instances[scene.area_light_instance_index],
        area_mask,
    );
    changed
}

fn update_camera_from_input(
    window: &glfw::PWindow,
    camera: &mut CameraState,
    delta_time: f32,
    mouse_delta: (f32, f32),
) -> bool {
    let speed_multiplier =
        if is_key_down(window, Key::LeftShift) || is_key_down(window, Key::RightShift) {
            CAMERA_BOOST_MULTIPLIER
        } else {
            1.0
        };
    let move_step = CAMERA_MOVE_SPEED * speed_multiplier * delta_time.max(1.0 / 240.0);

    let forward = camera.forward();
    let right = camera.right();
    let mut movement = Vec3::ZERO;

    if is_key_down(window, Key::W) {
        movement += forward;
    }
    if is_key_down(window, Key::S) {
        movement -= forward;
    }
    if is_key_down(window, Key::D) {
        movement += right;
    }
    if is_key_down(window, Key::A) {
        movement -= right;
    }
    if is_key_down(window, Key::Space) {
        movement += Vec3::Y;
    }
    if is_key_down(window, Key::LeftControl) || is_key_down(window, Key::RightControl) {
        movement -= Vec3::Y;
    }

    let mut changed = false;
    if movement.length_squared() > 0.0 {
        camera.translate(movement.normalize() * move_step);
        changed = true;
    }

    let (mouse_dx, mouse_dy) = mouse_delta;
    if mouse_dx != 0.0 || mouse_dy != 0.0 {
        camera.rotate(
            mouse_dx * CAMERA_MOUSE_SENSITIVITY,
            -mouse_dy * CAMERA_MOUSE_SENSITIVITY,
        );
        changed = true;
    }

    changed
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let validation = ValidationLayerConfig::new();
    let entry = unsafe { ash::Entry::load() }?;
    assert!(
        validation.check_support(&entry)?,
        "Validation layer not supported"
    );

    let mut glfw = glfw::init(glfw::fail_on_errors).ok();
    let window = if !HEADLESS_MODE {
        let g = glfw.as_mut().unwrap();
        g.window_hint(glfw::WindowHint::ClientApi(glfw::ClientApiHint::NoApi));
        g.window_hint(glfw::WindowHint::Resizable(false));
        let (mut win, _events) = g
            .create_window(
                WIDTH,
                HEIGHT,
                "Vulkan Ray Tracing",
                glfw::WindowMode::Windowed,
            )
            .expect("Failed to create GLFW window");
        win.set_cursor_mode(CursorMode::Disabled);
        Some(win)
    } else {
        None
    };

    let instance_extensions = get_instance_extensions(HEADLESS_MODE);
    let instance = create_instance(
        &entry,
        &validation.as_ptrs(),
        &instance_extensions,
        validation.enabled,
    )?;

    let surface_loader = if !HEADLESS_MODE {
        Some(khr::surface::Instance::new(&entry, &instance))
    } else {
        None
    };

    let surface = if !HEADLESS_MODE {
        let win = window.as_ref().unwrap();
        Some(unsafe {
            ash_window::create_surface(
                &entry,
                &instance,
                win.display_handle()
                    .expect("Failed to get display handle")
                    .as_raw(),
                win.window_handle()
                    .expect("Failed to get window handle")
                    .as_raw(),
                None,
            )?
        })
    } else {
        None
    };

    let (physical_device, queue_indices) = pick_physical_device_and_queue_family_indices(
        &instance,
        surface_loader.as_ref(),
        surface,
        &[
            khr::acceleration_structure::NAME,
            khr::deferred_host_operations::NAME,
            khr::ray_tracing_pipeline::NAME,
        ],
        true,
    )?
    .ok_or("No suitable physical device found")?;

    let graphics_queue_index = queue_indices.graphics_family.unwrap();
    let device = create_device(&instance, physical_device, &queue_indices, HEADLESS_MODE)?;

    let rt_pipeline_properties = get_rt_pipeline_properties(&instance, physical_device);
    let acceleration_structure = khr::acceleration_structure::Device::new(&instance, &device);
    let rt_pipeline = khr::ray_tracing_pipeline::Device::new(&instance, &device);

    let graphics_queue = unsafe { device.get_device_queue(graphics_queue_index, 0) };
    let command_pool = create_command_pool(&device, graphics_queue_index)?;
    let device_memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };

    let render_target = RenderTargetImage::new(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;
    transition_image_to_general(&device, command_pool, graphics_queue, render_target.image)?;

    let current_noisy_image = RenderTargetImage::new(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;
    let previous_color_image = RenderTargetImage::new(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;
    let previous_position_image = RenderTargetImage::new(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;
    let previous_normal_roughness_image = RenderTargetImage::new(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;
    let previous_moments_image = RenderTargetImage::new(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;
    let current_position_image = RenderTargetImage::new(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;
    let current_normal_roughness_image = RenderTargetImage::new(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;
    let current_moments_image = RenderTargetImage::new(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;
    let denoise_ping_image = RenderTargetImage::new(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;

    transition_image_to_general(
        &device,
        command_pool,
        graphics_queue,
        current_noisy_image.image,
    )?;
    transition_image_to_general(
        &device,
        command_pool,
        graphics_queue,
        previous_color_image.image,
    )?;
    transition_image_to_general(
        &device,
        command_pool,
        graphics_queue,
        previous_position_image.image,
    )?;
    transition_image_to_general(
        &device,
        command_pool,
        graphics_queue,
        previous_normal_roughness_image.image,
    )?;
    transition_image_to_general(
        &device,
        command_pool,
        graphics_queue,
        previous_moments_image.image,
    )?;
    transition_image_to_general(
        &device,
        command_pool,
        graphics_queue,
        current_position_image.image,
    )?;
    transition_image_to_general(
        &device,
        command_pool,
        graphics_queue,
        current_normal_roughness_image.image,
    )?;
    transition_image_to_general(
        &device,
        command_pool,
        graphics_queue,
        current_moments_image.image,
    )?;
    transition_image_to_general(
        &device,
        command_pool,
        graphics_queue,
        denoise_ping_image.image,
    )?;

    let (bottom_as, bottom_as_buffer, aabb_buffer) = create_bottom_level_as(
        &device,
        &acceleration_structure,
        command_pool,
        graphics_queue,
        device_memory_properties,
    )?;

    let sphere_accel_handle =
        get_acceleration_structure_device_address(&acceleration_structure, bottom_as);

    let mut scene = sample_scene(sphere_accel_handle);
    let mut demo_light = default_demo_light();
    sync_light_instance_visibility(&mut scene, demo_light);

    let mut instance_buffer =
        create_instance_buffer(&device, &scene.instances, device_memory_properties);

    let (mut top_as, mut top_as_buffer) = create_top_level_as(
        &device,
        &acceleration_structure,
        command_pool,
        graphics_queue,
        device_memory_properties,
        &instance_buffer,
        scene.instances.len() as u32,
    )?;
    let mut materials = scene.materials.clone();
    update_demo_light_materials(
        &mut materials,
        scene.point_light_material_index,
        scene.area_light_material_index,
        demo_light,
    );
    let mut material_buffer = create_material_buffer(&device, &materials, device_memory_properties);
    let environment_map = generate_environment_map();
    let environment_texel_buffer =
        create_material_buffer(&device, &environment_map.texels, device_memory_properties);
    let environment_pmf_buffer =
        create_material_buffer(&device, &environment_map.pmf, device_memory_properties);
    let environment_conditional_cdf_buffer = create_material_buffer(
        &device,
        &environment_map.conditional_cdf,
        device_memory_properties,
    );
    let environment_marginal_cdf_buffer = create_material_buffer(
        &device,
        &environment_map.marginal_cdf,
        device_memory_properties,
    );

    let mut camera =
        CameraState::from_look_at(vec3(13.0, 2.0, 3.0), vec3(0.0, 0.0, 0.0), 20.0, 0.1);
    camera.focus_distance = 10.0;
    let temporal_enabled = !HEADLESS_MODE;
    let mut frame_uniform_buffer = BufferResource::new(
        std::mem::size_of::<FrameUniform>() as u64,
        vk::BufferUsageFlags::STORAGE_BUFFER,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        &device,
        device_memory_properties,
    );
    let mut previous_frame_uniform = camera.build_uniform(WIDTH as f32 / HEIGHT as f32);
    previous_frame_uniform.origin[3] = 0.0;
    let mut bootstrap_frame_uniform = camera.build_uniform(WIDTH as f32 / HEIGHT as f32);
    bootstrap_frame_uniform.origin[3] = if temporal_enabled { 1.0 } else { 0.0 };
    frame_uniform_buffer.store(&[bootstrap_frame_uniform], &device);

    let mut previous_frame_uniform_buffer = BufferResource::new(
        std::mem::size_of::<FrameUniform>() as u64,
        vk::BufferUsageFlags::STORAGE_BUFFER,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        &device,
        device_memory_properties,
    );
    previous_frame_uniform_buffer.store(&[previous_frame_uniform], &device);

    let mut light_uniform_buffer = BufferResource::new(
        std::mem::size_of::<LightUniform>() as u64,
        vk::BufferUsageFlags::STORAGE_BUFFER,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        &device,
        device_memory_properties,
    );
    light_uniform_buffer.store(
        &[demo_light.build_uniform(environment_map.average_luminance)],
        &device,
    );

    let raygen_output_view = if temporal_enabled {
        current_noisy_image.view
    } else {
        render_target.view
    };

    let descriptor_set_layout = create_descriptor_set_layout(&device)?;
    let (pipeline, pipeline_layout, shader_groups_len) =
        create_ray_tracing_pipeline(&device, &rt_pipeline, descriptor_set_layout)?;
    let (descriptor_pool, descriptor_set) =
        create_descriptor_pool_and_set(&device, descriptor_set_layout)?;

    update_descriptor_set(
        &device,
        descriptor_set,
        top_as,
        raygen_output_view,
        material_buffer.buffer,
        frame_uniform_buffer.buffer,
        previous_frame_uniform_buffer.buffer,
        light_uniform_buffer.buffer,
        environment_texel_buffer.buffer,
        environment_pmf_buffer.buffer,
        environment_conditional_cdf_buffer.buffer,
        environment_marginal_cdf_buffer.buffer,
        previous_color_image.view,
        previous_position_image.view,
        previous_normal_roughness_image.view,
        current_position_image.view,
        current_normal_roughness_image.view,
    );

    let (
        shader_binding_table_buffer,
        sbt_raygen_region,
        sbt_miss_region,
        sbt_hit_region,
        sbt_call_region,
    ) = create_shader_binding_table(
        &device,
        &rt_pipeline,
        pipeline,
        &rt_pipeline_properties,
        shader_groups_len,
        device_memory_properties,
    )?;

    let denoise_descriptor_set_layout = create_denoise_descriptor_set_layout(&device)?;
    let (denoise_pipeline, denoise_pipeline_layout) =
        create_denoise_pipeline(&device, denoise_descriptor_set_layout)?;
    let (denoise_descriptor_pool, denoise_descriptor_set) =
        create_denoise_descriptor_pool_and_set(&device, denoise_descriptor_set_layout)?;
    update_denoise_descriptor_set(
        &device,
        denoise_descriptor_set,
        current_noisy_image.view,
        previous_color_image.view,
        previous_position_image.view,
        previous_normal_roughness_image.view,
        current_position_image.view,
        current_normal_roughness_image.view,
        previous_moments_image.view,
        current_moments_image.view,
        denoise_ping_image.view,
        render_target.view,
        frame_uniform_buffer.buffer,
        previous_frame_uniform_buffer.buffer,
    );

    let image_subresource_range = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1);

    let image_barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(
            vk::AccessFlags::SHADER_READ
                | vk::AccessFlags::SHADER_WRITE
                | vk::AccessFlags::MEMORY_READ
                | vk::AccessFlags::MEMORY_WRITE,
        )
        .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
        .old_layout(vk::ImageLayout::GENERAL)
        .new_layout(vk::ImageLayout::GENERAL)
        .image(render_target.image)
        .subresource_range(image_subresource_range);

    let clear_barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
        .old_layout(vk::ImageLayout::GENERAL)
        .new_layout(vk::ImageLayout::GENERAL)
        .image(render_target.image)
        .subresource_range(image_subresource_range);

    let command_buffer = allocate_command_buffer(&device, command_pool)?;
    let compute_group_count_x = WIDTH.div_ceil(8);
    let compute_group_count_y = HEIGHT.div_ceil(8);

    let windowed_resources = if !HEADLESS_MODE {
        Some(WindowedResources::new(
            &instance,
            &device,
            physical_device,
            surface.unwrap(),
            surface_loader.as_ref().unwrap(),
            render_target.view,
            WIDTH,
            HEIGHT,
        ))
    } else {
        None
    };

    let mut sampled = 0u32;
    let mut should_close = false;
    let mut reset_accumulation = true;
    let mut last_frame_time = Instant::now();
    let mut last_cursor_pos: Option<(f64, f64)> = None;
    let mut light_key_latches = [false; 4];

    eprintln!(
        "Controls: WASD move, Space/Ctrl up-down, mouse look, 1/2/3/4 toggle lights, Esc quit"
    );
    eprintln!("Light state: {}", demo_light.summary());

    loop {
        if should_close || (HEADLESS_MODE && sampled >= N_SAMPLES) {
            break;
        }

        let delta_time = last_frame_time.elapsed().as_secs_f32();
        last_frame_time = Instant::now();

        if !HEADLESS_MODE {
            let g = glfw.as_mut().unwrap();
            g.poll_events();

            let win = window.as_ref().unwrap();
            if win.should_close() {
                should_close = true;
                continue;
            }

            if is_key_down(win, Key::Escape) {
                should_close = true;
                continue;
            }

            let current_cursor_pos = win.get_cursor_pos();
            let mouse_delta = if let Some((last_x, last_y)) = last_cursor_pos {
                (
                    (current_cursor_pos.0 - last_x) as f32,
                    (current_cursor_pos.1 - last_y) as f32,
                )
            } else {
                (0.0, 0.0)
            };
            last_cursor_pos = Some(current_cursor_pos);

            if update_camera_from_input(win, &mut camera, delta_time, mouse_delta) {
                sampled = 0;
                if HEADLESS_MODE {
                    reset_accumulation = true;
                }
            }

            if update_light_from_input(win, &mut demo_light, &mut light_key_latches) {
                let visibility_changed = sync_light_instance_visibility(&mut scene, demo_light);

                if visibility_changed {
                    unsafe {
                        device.device_wait_idle()?;
                    }
                    instance_buffer.store(&scene.instances, &device);

                    let (new_top_as, new_top_as_buffer) = create_top_level_as(
                        &device,
                        &acceleration_structure,
                        command_pool,
                        graphics_queue,
                        device_memory_properties,
                        &instance_buffer,
                        scene.instances.len() as u32,
                    )?;

                    let old_top_as = top_as;
                    let old_top_as_buffer =
                        std::mem::replace(&mut top_as_buffer, new_top_as_buffer);
                    top_as = new_top_as;

                    update_descriptor_set(
                        &device,
                        descriptor_set,
                        top_as,
                        raygen_output_view,
                        material_buffer.buffer,
                        frame_uniform_buffer.buffer,
                        previous_frame_uniform_buffer.buffer,
                        light_uniform_buffer.buffer,
                        environment_texel_buffer.buffer,
                        environment_pmf_buffer.buffer,
                        environment_conditional_cdf_buffer.buffer,
                        environment_marginal_cdf_buffer.buffer,
                        previous_color_image.view,
                        previous_position_image.view,
                        previous_normal_roughness_image.view,
                        current_position_image.view,
                        current_normal_roughness_image.view,
                    );

                    unsafe {
                        acceleration_structure.destroy_acceleration_structure(old_top_as, None);
                        old_top_as_buffer.destroy(&device);
                    }
                }

                update_demo_light_materials(
                    &mut materials,
                    scene.point_light_material_index,
                    scene.area_light_material_index,
                    demo_light,
                );
                material_buffer.store(&materials, &device);
                sampled = 0;
                reset_accumulation = true;
                eprintln!("\nLight state: {}", demo_light.summary());
            }
        }

        let mut current_frame_uniform = camera.build_uniform(WIDTH as f32 / HEIGHT as f32);
        current_frame_uniform.origin[3] = if temporal_enabled { 1.0 } else { 0.0 };
        if reset_accumulation {
            previous_frame_uniform.origin[3] = 0.0;
        }
        previous_frame_uniform_buffer.store(&[previous_frame_uniform], &device);
        frame_uniform_buffer.store(&[current_frame_uniform], &device);
        light_uniform_buffer.store(
            &[demo_light.build_uniform(environment_map.average_luminance)],
            &device,
        );

        let samples = if HEADLESS_MODE {
            std::cmp::min(N_SAMPLES - sampled, N_SAMPLES_ITER)
        } else {
            WINDOW_SAMPLES_PER_FRAME
        };

        unsafe {
            device.begin_command_buffer(
                command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::SIMULTANEOUS_USE),
            )?;

            if temporal_enabled {
                if reset_accumulation {
                    for image in [
                        previous_color_image.image,
                        previous_position_image.image,
                        previous_normal_roughness_image.image,
                        previous_moments_image.image,
                    ] {
                        device.cmd_clear_color_image(
                            command_buffer,
                            image,
                            vk::ImageLayout::GENERAL,
                            &vk::ClearColorValue {
                                float32: [0.0, 0.0, 0.0, 0.0],
                            },
                            &[image_subresource_range],
                        );
                    }

                    let history_clear_barriers = [
                        vk::ImageMemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                            .dst_access_mask(
                                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                            )
                            .old_layout(vk::ImageLayout::GENERAL)
                            .new_layout(vk::ImageLayout::GENERAL)
                            .image(previous_color_image.image)
                            .subresource_range(image_subresource_range),
                        vk::ImageMemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                            .dst_access_mask(
                                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                            )
                            .old_layout(vk::ImageLayout::GENERAL)
                            .new_layout(vk::ImageLayout::GENERAL)
                            .image(previous_position_image.image)
                            .subresource_range(image_subresource_range),
                        vk::ImageMemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                            .dst_access_mask(
                                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                            )
                            .old_layout(vk::ImageLayout::GENERAL)
                            .new_layout(vk::ImageLayout::GENERAL)
                            .image(previous_normal_roughness_image.image)
                            .subresource_range(image_subresource_range),
                        vk::ImageMemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                            .dst_access_mask(
                                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                            )
                            .old_layout(vk::ImageLayout::GENERAL)
                            .new_layout(vk::ImageLayout::GENERAL)
                            .image(previous_moments_image.image)
                            .subresource_range(image_subresource_range),
                    ];

                    device.cmd_pipeline_barrier(
                        command_buffer,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &history_clear_barriers,
                    );
                }
            } else if reset_accumulation {
                device.cmd_clear_color_image(
                    command_buffer,
                    render_target.image,
                    vk::ImageLayout::GENERAL,
                    &vk::ClearColorValue {
                        float32: [0.0, 0.0, 0.0, 0.0],
                    },
                    &[image_subresource_range],
                );

                device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[clear_barrier],
                );
            }

            device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::RAY_TRACING_KHR,
                pipeline,
            );
            device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::RAY_TRACING_KHR,
                pipeline_layout,
                0,
                &[descriptor_set],
                &[],
            );

            if temporal_enabled {
                device.cmd_push_constants(
                    command_buffer,
                    pipeline_layout,
                    vk::ShaderStageFlags::RAYGEN_KHR,
                    0,
                    &sampled.to_le_bytes(),
                );

                rt_pipeline.cmd_trace_rays(
                    command_buffer,
                    &sbt_raygen_region,
                    &sbt_miss_region,
                    &sbt_hit_region,
                    &sbt_call_region,
                    WIDTH,
                    HEIGHT,
                    1,
                );

                let rt_to_compute_barriers = [
                    vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ)
                        .old_layout(vk::ImageLayout::GENERAL)
                        .new_layout(vk::ImageLayout::GENERAL)
                        .image(current_noisy_image.image)
                        .subresource_range(image_subresource_range),
                    vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ)
                        .old_layout(vk::ImageLayout::GENERAL)
                        .new_layout(vk::ImageLayout::GENERAL)
                        .image(current_position_image.image)
                        .subresource_range(image_subresource_range),
                    vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ)
                        .old_layout(vk::ImageLayout::GENERAL)
                        .new_layout(vk::ImageLayout::GENERAL)
                        .image(current_normal_roughness_image.image)
                        .subresource_range(image_subresource_range),
                ];

                device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &rt_to_compute_barriers,
                );

                device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::COMPUTE,
                    denoise_pipeline,
                );
                device.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::COMPUTE,
                    denoise_pipeline_layout,
                    0,
                    &[denoise_descriptor_set],
                    &[],
                );

                let temporal_push_constants = DenoisePushConstants {
                    mode: 0,
                    step_width: 1,
                    input_is_ping: 1,
                    _padding: 0,
                };
                device.cmd_push_constants(
                    command_buffer,
                    denoise_pipeline_layout,
                    vk::ShaderStageFlags::COMPUTE,
                    0,
                    push_constants_bytes(&temporal_push_constants),
                );
                device.cmd_dispatch(
                    command_buffer,
                    compute_group_count_x,
                    compute_group_count_y,
                    1,
                );

                let temporal_output_barriers = [
                    vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                        .dst_access_mask(
                            vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                        )
                        .old_layout(vk::ImageLayout::GENERAL)
                        .new_layout(vk::ImageLayout::GENERAL)
                        .image(denoise_ping_image.image)
                        .subresource_range(image_subresource_range),
                    vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ)
                        .old_layout(vk::ImageLayout::GENERAL)
                        .new_layout(vk::ImageLayout::GENERAL)
                        .image(current_moments_image.image)
                        .subresource_range(image_subresource_range),
                ];
                device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &temporal_output_barriers,
                );

                for iteration in 0..DENOISE_ATROUS_PASSES {
                    let input_is_ping = if iteration % 2 == 0 { 1 } else { 0 };
                    let atrous_push_constants = DenoisePushConstants {
                        mode: 1,
                        step_width: 1u32 << iteration,
                        input_is_ping,
                        _padding: 0,
                    };
                    device.cmd_push_constants(
                        command_buffer,
                        denoise_pipeline_layout,
                        vk::ShaderStageFlags::COMPUTE,
                        0,
                        push_constants_bytes(&atrous_push_constants),
                    );
                    device.cmd_dispatch(
                        command_buffer,
                        compute_group_count_x,
                        compute_group_count_y,
                        1,
                    );

                    if iteration + 1 < DENOISE_ATROUS_PASSES {
                        let intermediate_image = if input_is_ping == 1 {
                            render_target.image
                        } else {
                            denoise_ping_image.image
                        };
                        let atrous_barrier = vk::ImageMemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                            .dst_access_mask(
                                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                            )
                            .old_layout(vk::ImageLayout::GENERAL)
                            .new_layout(vk::ImageLayout::GENERAL)
                            .image(intermediate_image)
                            .subresource_range(image_subresource_range);
                        device.cmd_pipeline_barrier(
                            command_buffer,
                            vk::PipelineStageFlags::COMPUTE_SHADER,
                            vk::PipelineStageFlags::COMPUTE_SHADER,
                            vk::DependencyFlags::empty(),
                            &[],
                            &[],
                            &[atrous_barrier],
                        );
                    }
                }
            } else {
                for sample_index in 0..samples {
                    if !reset_accumulation || sample_index > 0 {
                        device.cmd_pipeline_barrier(
                            command_buffer,
                            vk::PipelineStageFlags::ALL_COMMANDS,
                            vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                            vk::DependencyFlags::empty(),
                            &[],
                            &[],
                            &[image_barrier],
                        );
                    }

                    device.cmd_push_constants(
                        command_buffer,
                        pipeline_layout,
                        vk::ShaderStageFlags::RAYGEN_KHR,
                        0,
                        &(sampled + sample_index).to_le_bytes(),
                    );

                    rt_pipeline.cmd_trace_rays(
                        command_buffer,
                        &sbt_raygen_region,
                        &sbt_miss_region,
                        &sbt_hit_region,
                        &sbt_call_region,
                        WIDTH,
                        HEIGHT,
                        1,
                    );
                }
            }

            device.end_command_buffer(command_buffer)?;
        }

        submit_and_wait(&device, graphics_queue, command_buffer)?;
        if temporal_enabled {
            copy_image_to_image(
                &device,
                command_pool,
                graphics_queue,
                denoise_ping_image.image,
                previous_color_image.image,
                WIDTH,
                HEIGHT,
            )?;
            copy_image_to_image(
                &device,
                command_pool,
                graphics_queue,
                current_position_image.image,
                previous_position_image.image,
                WIDTH,
                HEIGHT,
            )?;
            copy_image_to_image(
                &device,
                command_pool,
                graphics_queue,
                current_normal_roughness_image.image,
                previous_normal_roughness_image.image,
                WIDTH,
                HEIGHT,
            )?;
            copy_image_to_image(
                &device,
                command_pool,
                graphics_queue,
                current_moments_image.image,
                previous_moments_image.image,
                WIDTH,
                HEIGHT,
            )?;
        }
        sampled += samples;
        reset_accumulation = false;
        previous_frame_uniform = current_frame_uniform;

        if let Some(ref resources) = windowed_resources {
            render_to_swapchain(
                &device,
                command_pool,
                graphics_queue,
                resources,
                if temporal_enabled { 1 } else { sampled.max(1) },
            )?;

            if FRAME_DELAY_MS > 0 {
                thread::sleep(Duration::from_millis(FRAME_DELAY_MS));
            }
        }

        if HEADLESS_MODE {
            eprint!("\rSamples: {} / {} ", sampled, N_SAMPLES);
        } else {
            eprint!("\rAccumulated Samples: {} ", sampled);
        }
    }

    if let Some(resources) = windowed_resources {
        unsafe {
            device.device_wait_idle()?;
            resources.destroy(&device);
        }
    }

    unsafe { device.free_command_buffers(command_pool, &[command_buffer]) };
    eprintln!("\nDone");

    let (dst_image, dst_device_memory) = create_host_visible_image(
        &device,
        WIDTH,
        HEIGHT,
        COLOR_FORMAT,
        device_memory_properties,
    )?;

    copy_image_to_host(
        &device,
        command_pool,
        graphics_queue,
        render_target.image,
        dst_image,
        WIDTH,
        HEIGHT,
    )?;

    save_image_to_png(
        &device,
        dst_device_memory,
        dst_image,
        WIDTH,
        HEIGHT,
        if temporal_enabled { 1 } else { sampled.max(1) },
        "out.png",
    );

    unsafe {
        device.free_memory(dst_device_memory, None);
        device.destroy_image(dst_image, None);

        device.destroy_command_pool(command_pool, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        shader_binding_table_buffer.destroy(&device);
        device.destroy_pipeline(pipeline, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.destroy_pipeline_layout(pipeline_layout, None);
        device.destroy_descriptor_pool(denoise_descriptor_pool, None);
        device.destroy_pipeline(denoise_pipeline, None);
        device.destroy_descriptor_set_layout(denoise_descriptor_set_layout, None);
        device.destroy_pipeline_layout(denoise_pipeline_layout, None);

        acceleration_structure.destroy_acceleration_structure(top_as, None);
        top_as_buffer.destroy(&device);
        acceleration_structure.destroy_acceleration_structure(bottom_as, None);
        bottom_as_buffer.destroy(&device);
        aabb_buffer.destroy(&device);

        render_target.destroy(&device);
        current_noisy_image.destroy(&device);
        previous_color_image.destroy(&device);
        previous_position_image.destroy(&device);
        previous_normal_roughness_image.destroy(&device);
        previous_moments_image.destroy(&device);
        current_position_image.destroy(&device);
        current_normal_roughness_image.destroy(&device);
        current_moments_image.destroy(&device);
        denoise_ping_image.destroy(&device);
        material_buffer.destroy(&device);
        frame_uniform_buffer.destroy(&device);
        previous_frame_uniform_buffer.destroy(&device);
        light_uniform_buffer.destroy(&device);
        environment_texel_buffer.destroy(&device);
        environment_pmf_buffer.destroy(&device);
        environment_conditional_cdf_buffer.destroy(&device);
        environment_marginal_cdf_buffer.destroy(&device);
        instance_buffer.destroy(&device);

        device.destroy_device(None);

        if let Some(s) = surface {
            if let Some(loader) = surface_loader.as_ref() {
                loader.destroy_surface(s, None);
            }
        }

        instance.destroy_instance(None);
    }

    Ok(())
}
