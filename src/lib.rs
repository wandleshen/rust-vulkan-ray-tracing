//! Vulkan Ray Tracing 库
//!
//! 提供基于 Vulkan 的光线追踪渲染功能

pub mod acceleration_structure;
pub mod buffer;
pub mod camera;
pub mod denoise;
pub mod environment;
pub mod image_utils;
pub mod light;
pub mod material;
pub mod pipeline;
pub mod scene;
pub mod vulkan_base;
pub mod windowed;

// Buffer 模块导出
pub use buffer::{BufferResource, aligned_size, get_buffer_device_address, get_memory_type_index};

// 相机模块导出
pub use camera::{CameraState, FrameUniform};
pub use denoise::{
    DenoisePipelineResources, DenoisePushConstants, create_denoise_descriptor_pool_and_set,
    create_denoise_descriptor_set_layout, create_denoise_pipeline, push_constants_bytes,
    update_denoise_descriptor_set,
};

pub use environment::{
    ENV_MAP_HEIGHT, ENV_MAP_WIDTH, EnvironmentMapData, generate_environment_map,
};

// 灯光模块导出
pub use light::{
    DemoLightState, GpuLight, LightMode, LightUniform, MAX_LIGHTS, area_light_emission,
    area_light_position, area_light_radius, default_demo_light, key_to_light_mode,
    point_light_emission, point_light_position, point_light_radius,
};

// 图像工具导出
pub use image_utils::{
    RenderTargetImage, copy_image_to_host, copy_image_to_image, create_host_visible_image,
    save_image_to_png, transition_image_to_general,
};

// 材质导出
pub use material::Material;

// 场景导出
pub use scene::{SceneData, create_sphere_instance, sample_scene};

// Vulkan 基础设施导出
pub use vulkan_base::{
    QueueFamilyIndices, ValidationLayerConfig, allocate_command_buffer,
    check_validation_layer_support, create_command_pool, create_device, create_instance,
    create_shader_module, default_vulkan_debug_utils_callback, get_instance_extensions,
    get_rt_pipeline_properties, pick_physical_device_and_queue_family_indices, submit_and_wait,
};

// 窗口模块导出
pub use windowed::{
    Swapchain, WindowedResources, check_surface_support, create_blit_pipeline, create_framebuffers,
    create_render_pass, render_to_swapchain,
};

// 加速结构导出
pub use acceleration_structure::{
    AccelerationStructureResources, create_bottom_level_as, create_instance_buffer,
    create_top_level_as, get_acceleration_structure_device_address,
};

// 管线导出
pub use pipeline::{
    RayTracingPipelineResources, create_descriptor_pool_and_set, create_descriptor_set_layout,
    create_material_buffer, create_ray_tracing_pipeline, create_shader_binding_table,
    update_descriptor_set,
};
