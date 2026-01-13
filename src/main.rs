//! Vulkan Ray Tracing 主程序
//!
//! 实现基于 Vulkan 的实时光线追踪渲染

use vulkan_raytracing::*;

use std::{ffi::CString, thread, time::Duration};

use ash::{khr, vk};
use rand::prelude::*;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ========== 渲染配置 ==========
    const HEADLESS_MODE: bool = false; // true = 无头模式(输出PNG), false = 窗口模式(实时预览)
    const PREVIEW_INTERVAL: u32 = 1; // 窗口模式下每多少个sample更新一次显示
    const FRAME_DELAY_MS: u64 = 0; // 每帧之间的延迟（毫秒），0 表示无延迟

    const ENABLE_VALIDATION_LAYER: bool = false;
    const WIDTH: u32 = 1200;
    const HEIGHT: u32 = 800;
    const COLOR_FORMAT: vk::Format = vk::Format::R32G32B32A32_SFLOAT;

    const N_SAMPLES: u32 = 5000;
    const N_SAMPLES_ITER: u32 = 100;

    // ========== 验证层设置 ==========
    let validation_layers: Vec<CString> = if ENABLE_VALIDATION_LAYER {
        vec![CString::new("VK_LAYER_KHRONOS_validation")?]
    } else {
        Vec::new()
    };
    let validation_layers_ptr: Vec<*const i8> =
        validation_layers.iter().map(|c_str| c_str.as_ptr()).collect();

    let entry = unsafe { ash::Entry::load() }?;

    assert_eq!(
        unsafe {
            check_validation_layer_support(
                &entry,
                validation_layers.iter().map(|cstring| cstring.as_c_str()),
            )
        },
        Ok(true)
    );

    // ========== GLFW 初始化 ==========
    let mut glfw = glfw::init(glfw::fail_on_errors).ok();
    let window = if !HEADLESS_MODE {
        let g = glfw.as_mut().unwrap();
        g.window_hint(glfw::WindowHint::ClientApi(glfw::ClientApiHint::NoApi));
        g.window_hint(glfw::WindowHint::Resizable(false));
        let (win, _events) = g
            .create_window(WIDTH, HEIGHT, "Vulkan Ray Tracing", glfw::WindowMode::Windowed)
            .expect("Failed to create GLFW window");
        Some(win)
    } else {
        None
    };

    // ========== Vulkan Instance 创建 ==========
    let instance_extensions = get_instance_extensions(HEADLESS_MODE);
    let instance = create_instance(
        &entry,
        &validation_layers_ptr,
        &instance_extensions,
        ENABLE_VALIDATION_LAYER,
    )?;

    // ========== Surface 创建 ==========
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
                win.display_handle().expect("Failed to get display handle").as_raw(),
                win.window_handle().expect("Failed to get window handle").as_raw(),
                None,
            )?
        })
    } else {
        None
    };

    // ========== 物理设备和队列族选择 ==========
    let (physical_device, queue_family_index) = pick_physical_device_and_queue_family_indices(
        &instance,
        &[
            khr::acceleration_structure::NAME,
            khr::deferred_host_operations::NAME,
            khr::ray_tracing_pipeline::NAME,
        ],
    )?
    .ok_or("No suitable physical device found")?;

    // ========== 逻辑设备创建 ==========
    let device = create_device(&instance, physical_device, queue_family_index, HEADLESS_MODE)?;

    let rt_pipeline_properties = get_rt_pipeline_properties(&instance, physical_device);
    let acceleration_structure = khr::acceleration_structure::Device::new(&instance, &device);
    let rt_pipeline = khr::ray_tracing_pipeline::Device::new(&instance, &device);

    let graphics_queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    let command_pool = create_command_pool(&device, queue_family_index)?;

    let device_memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };

    // ========== 渲染目标图像创建 ==========
    let render_target =
        RenderTargetImage::new(&device, WIDTH, HEIGHT, COLOR_FORMAT, device_memory_properties)?;

    transition_image_to_general(&device, command_pool, graphics_queue, render_target.image)?;

    // ========== 加速结构创建 ==========
    let (bottom_as, bottom_as_buffer, aabb_buffer) = create_bottom_level_as(
        &device,
        &acceleration_structure,
        command_pool,
        graphics_queue,
        device_memory_properties,
    )?;

    let sphere_accel_handle =
        get_acceleration_structure_device_address(&acceleration_structure, bottom_as);

    let (sphere_instances, materials) = sample_scene(sphere_accel_handle);

    let instance_buffer =
        create_instance_buffer(&device, &sphere_instances, device_memory_properties);

    let (top_as, top_as_buffer) = create_top_level_as(
        &device,
        &acceleration_structure,
        command_pool,
        graphics_queue,
        device_memory_properties,
        &instance_buffer,
        sphere_instances.len() as u32,
    )?;

    // ========== 材质缓冲区 ==========
    let material_buffer = create_material_buffer(&device, &materials, device_memory_properties);

    // ========== Ray Tracing 管线创建 ==========
    let descriptor_set_layout = create_descriptor_set_layout(&device)?;

    let (pipeline, pipeline_layout, shader_groups_len) =
        create_ray_tracing_pipeline(&device, &rt_pipeline, descriptor_set_layout)?;

    let (descriptor_pool, descriptor_set) =
        create_descriptor_pool_and_set(&device, descriptor_set_layout)?;

    update_descriptor_set(
        &device,
        descriptor_set,
        top_as,
        render_target.view,
        material_buffer.buffer,
    );

    // ========== Shader Binding Table ==========
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

    // ========== 渲染循环准备 ==========
    let image_barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::SHADER_READ)
        .dst_access_mask(vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::GENERAL)
        .new_layout(vk::ImageLayout::GENERAL)
        .image(render_target.image)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1),
        );

    // 清除图像
    {
        let command_buffer = allocate_command_buffer(&device, command_pool)?;

        unsafe {
            device.begin_command_buffer(
                command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::SIMULTANEOUS_USE),
            )?;

            let range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1);

            device.cmd_clear_color_image(
                command_buffer,
                render_target.image,
                vk::ImageLayout::GENERAL,
                &vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 0.0],
                },
                &[range],
            );

            let clear_barrier = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::GENERAL)
                .new_layout(vk::ImageLayout::GENERAL)
                .image(render_target.image)
                .subresource_range(range);

            device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[clear_barrier],
            );

            device.end_command_buffer(command_buffer)?;
        }

        submit_and_wait(&device, graphics_queue, command_buffer)?;
        unsafe { device.free_command_buffers(command_pool, &[command_buffer]) };
    }

    let mut rng = StdRng::from_os_rng();
    let mut sampled = 0u32;

    let command_buffer = allocate_command_buffer(&device, command_pool)?;

    // ========== 窗口模式资源 ==========
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

    // ========== 主渲染循环 ==========
    let mut should_close = false;
    while sampled < N_SAMPLES && !should_close {
        // 窗口模式：处理事件
        if !HEADLESS_MODE {
            let g = glfw.as_mut().unwrap();
            g.poll_events();
            if window.as_ref().unwrap().should_close() {
                should_close = true;
                continue;
            }
        }

        let samples = std::cmp::min(
            N_SAMPLES - sampled,
            if HEADLESS_MODE {
                N_SAMPLES_ITER
            } else {
                PREVIEW_INTERVAL
            },
        );
        sampled += samples;

        // 记录光线追踪命令
        unsafe {
            device.begin_command_buffer(
                command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::SIMULTANEOUS_USE),
            )?;

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

            for _ in 0..samples {
                device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[image_barrier],
                );

                device.cmd_push_constants(
                    command_buffer,
                    pipeline_layout,
                    vk::ShaderStageFlags::RAYGEN_KHR,
                    0,
                    &rng.next_u32().to_le_bytes(),
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

            device.end_command_buffer(command_buffer)?;
        }

        submit_and_wait(&device, graphics_queue, command_buffer)?;

        // 窗口模式：渲染到 swapchain
        if let Some(ref resources) = windowed_resources {
            render_to_swapchain(
                &device,
                command_pool,
                graphics_queue,
                resources,
                sampled,
            )?;

            if FRAME_DELAY_MS > 0 {
                thread::sleep(Duration::from_millis(FRAME_DELAY_MS));
            }
        }

        eprint!("\rSamples: {} / {} ", sampled, N_SAMPLES);
    }

    // ========== 清理窗口模式资源 ==========
    if let Some(resources) = windowed_resources {
        unsafe {
            device.device_wait_idle()?;
            resources.destroy(&device);
        }
    }

    unsafe { device.free_command_buffers(command_pool, &[command_buffer]) };
    eprintln!("\nDone");

    // ========== 导出 PNG ==========
    let (dst_image, dst_device_memory) =
        create_host_visible_image(&device, WIDTH, HEIGHT, COLOR_FORMAT, device_memory_properties)?;

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
        N_SAMPLES,
        "out.png",
    );

    unsafe {
        device.free_memory(dst_device_memory, None);
        device.destroy_image(dst_image, None);
    }

    // ========== 资源清理 ==========
    unsafe {
        device.destroy_command_pool(command_pool, None);
        device.destroy_descriptor_pool(descriptor_pool, None);
        shader_binding_table_buffer.destroy(&device);
        device.destroy_pipeline(pipeline, None);
        device.destroy_descriptor_set_layout(descriptor_set_layout, None);
        device.destroy_pipeline_layout(pipeline_layout, None);

        acceleration_structure.destroy_acceleration_structure(top_as, None);
        top_as_buffer.destroy(&device);
        acceleration_structure.destroy_acceleration_structure(bottom_as, None);
        bottom_as_buffer.destroy(&device);
        aabb_buffer.destroy(&device);

        render_target.destroy(&device);
        material_buffer.destroy(&device);
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
