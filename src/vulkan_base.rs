//! Vulkan 基础设施模块
//!
//! 包含 Instance、Device、Command Pool 创建以及调试工具

use ash::prelude::VkResult;
use ash::{ext, khr, vk, Device, Entry, Instance};
use std::collections::HashSet;
use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;

/// Vulkan 调试回调
pub unsafe extern "system" fn default_vulkan_debug_utils_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _p_user_data: *mut c_void,
) -> vk::Bool32 { unsafe {
    let severity = match message_severity {
        vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE => "[Verbose]",
        vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => "[Warning]",
        vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => "[Error]",
        vk::DebugUtilsMessageSeverityFlagsEXT::INFO => "[Info]",
        _ => "[Unknown]",
    };
    let types = match message_type {
        vk::DebugUtilsMessageTypeFlagsEXT::GENERAL => "[General]",
        vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE => "[Performance]",
        vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION => "[Validation]",
        _ => "[Unknown]",
    };
    let message = CStr::from_ptr((*p_callback_data).p_message);
    println!("[Debug]{}{}{:?}", severity, types, message);

    vk::FALSE
}}

pub unsafe fn check_validation_layer_support<'a>(
    entry: &Entry,
    required_validation_layers: impl IntoIterator<Item = &'a CStr>,
) -> VkResult<bool> { unsafe {
    let supported_layers: HashSet<CString> = entry
        .enumerate_instance_layer_properties()?
        .into_iter()
        .map(|layer_property| CStr::from_ptr(layer_property.layer_name.as_ptr()).to_owned())
        .collect();

    Ok(required_validation_layers
        .into_iter()
        .all(|l| supported_layers.contains(l)))
}}

pub fn pick_physical_device_and_queue_family_indices(
    instance: &Instance,
    extensions: &[&CStr],
) -> VkResult<Option<(vk::PhysicalDevice, u32)>> {
    Ok(unsafe { instance.enumerate_physical_devices() }?
        .into_iter()
        .find_map(|physical_device| {
            if unsafe { instance.enumerate_device_extension_properties(physical_device) }.map(
                |exts| {
                    let set: HashSet<&CStr> = exts
                        .iter()
                        .map(|ext| unsafe { CStr::from_ptr(&ext.extension_name as *const c_char) })
                        .collect();

                    extensions.iter().all(|ext| set.contains(ext))
                },
            ) != Ok(true)
            {
                return None;
            }

            let graphics_family =
                unsafe { instance.get_physical_device_queue_family_properties(physical_device) }
                    .into_iter()
                    .enumerate()
                    .find(|(_, device_properties)| {
                        device_properties.queue_count > 0
                            && device_properties
                                .queue_flags
                                .contains(vk::QueueFlags::GRAPHICS)
                    });

            graphics_family.map(|(i, _)| (physical_device, i as u32))
        }))
}

pub unsafe fn create_shader_module(device: &Device, code: &[u8]) -> VkResult<vk::ShaderModule> { unsafe {
    let shader_module_create_info = vk::ShaderModuleCreateInfo {
        s_type: vk::StructureType::SHADER_MODULE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: vk::ShaderModuleCreateFlags::empty(),
        code_size: code.len(),
        p_code: code.as_ptr() as *const u32,
        ..Default::default()
    };

    device.create_shader_module(&shader_module_create_info, None)
}}

/// 创建 Vulkan Instance
pub fn create_instance(
    entry: &Entry,
    validation_layers: &[*const i8],
    instance_extensions: &[*const i8],
    enable_validation: bool,
) -> VkResult<Instance> {
    let application_name = CString::new("Vulkan Ray Tracing").expect("Failed to create application name");
    let engine_name = CString::new("No Engine").expect("Failed to create engine name");

    let mut debug_utils_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
        .message_severity(
            vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
        )
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION,
        )
        .pfn_user_callback(Some(default_vulkan_debug_utils_callback));

    let application_info = vk::ApplicationInfo::default()
        .application_name(application_name.as_c_str())
        .application_version(vk::make_api_version(0, 1, 0, 0))
        .engine_name(engine_name.as_c_str())
        .engine_version(vk::make_api_version(0, 1, 0, 0))
        .api_version(vk::API_VERSION_1_3);

    let instance_create_info = vk::InstanceCreateInfo::default()
        .application_info(&application_info)
        .enabled_layer_names(validation_layers)
        .enabled_extension_names(instance_extensions);

    let instance_create_info = if enable_validation {
        instance_create_info.push_next(&mut debug_utils_create_info)
    } else {
        instance_create_info
    };

    unsafe { entry.create_instance(&instance_create_info, None) }
}

/// 创建逻辑设备
pub fn create_device(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    headless_mode: bool,
) -> VkResult<Device> {
    let priorities = [1.0];

    let queue_create_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);

    let mut features2 = vk::PhysicalDeviceFeatures2::default();
    unsafe { instance.get_physical_device_features2(physical_device, &mut features2) };

    let mut features12 = vk::PhysicalDeviceVulkan12Features::default()
        .shader_int8(true)
        .buffer_device_address(true)
        .vulkan_memory_model(true)
        .vulkan_memory_model_device_scope(true)
        .timeline_semaphore(true)
        .scalar_block_layout(true)
        .storage_buffer8_bit_access(true);

    let mut as_feature = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
        .acceleration_structure(true);

    let mut raytracing_pipeline =
        vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default().ray_tracing_pipeline(true);

    let queue_create_infos = [queue_create_info];

    let mut enabled_extension_names = vec![
        vk::KHR_RAY_TRACING_PIPELINE_NAME.as_ptr(),
        vk::KHR_ACCELERATION_STRUCTURE_NAME.as_ptr(),
        vk::KHR_DEFERRED_HOST_OPERATIONS_NAME.as_ptr(),
        vk::KHR_SPIRV_1_4_NAME.as_ptr(),
        vk::EXT_SCALAR_BLOCK_LAYOUT_NAME.as_ptr(),
        vk::KHR_GET_MEMORY_REQUIREMENTS2_NAME.as_ptr(),
    ];

    // 窗口模式需要 swapchain 扩展
    if !headless_mode {
        enabled_extension_names.push(vk::KHR_SWAPCHAIN_NAME.as_ptr());
    }

    let device_create_info = vk::DeviceCreateInfo::default()
        .push_next(&mut features2)
        .push_next(&mut features12)
        .push_next(&mut as_feature)
        .push_next(&mut raytracing_pipeline)
        .queue_create_infos(&queue_create_infos)
        .enabled_extension_names(&enabled_extension_names);

    unsafe { instance.create_device(physical_device, &device_create_info, None) }
}

/// 创建 Command Pool
pub fn create_command_pool(device: &Device, queue_family_index: u32) -> VkResult<vk::CommandPool> {
    let command_pool_create_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family_index)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);

    unsafe { device.create_command_pool(&command_pool_create_info, None) }
}

/// 获取 Ray Tracing 管线属性
pub fn get_rt_pipeline_properties(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
) -> vk::PhysicalDeviceRayTracingPipelinePropertiesKHR<'static> {
    let mut rt_pipeline_properties = vk::PhysicalDeviceRayTracingPipelinePropertiesKHR::default();
    let mut physical_device_properties2 =
        vk::PhysicalDeviceProperties2::default().push_next(&mut rt_pipeline_properties);

    unsafe {
        instance.get_physical_device_properties2(physical_device, &mut physical_device_properties2);
    }

    rt_pipeline_properties
}

/// 获取 Instance 扩展列表
pub fn get_instance_extensions(headless_mode: bool) -> Vec<*const i8> {
    let mut instance_extensions: Vec<*const i8> = vec![ext::debug_utils::NAME.as_ptr()];
    if !headless_mode {
        instance_extensions.push(khr::surface::NAME.as_ptr());
        #[cfg(target_os = "windows")]
        instance_extensions.push(khr::win32_surface::NAME.as_ptr());
        #[cfg(target_os = "linux")]
        {
            instance_extensions.push(khr::xlib_surface::NAME.as_ptr());
            instance_extensions.push(khr::wayland_surface::NAME.as_ptr());
        }
        #[cfg(target_os = "macos")]
        instance_extensions.push(ash::mvk::macos_surface::NAME.as_ptr());
    }
    instance_extensions
}

/// 分配单个命令缓冲区
pub fn allocate_command_buffer(
    device: &Device,
    command_pool: vk::CommandPool,
) -> VkResult<vk::CommandBuffer> {
    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_buffer_count(1)
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY);

    Ok(unsafe { device.allocate_command_buffers(&allocate_info) }?[0])
}

/// 提交命令缓冲区并等待完成
pub fn submit_and_wait(
    device: &Device,
    queue: vk::Queue,
    command_buffer: vk::CommandBuffer,
) -> VkResult<()> {
    let command_buffers = [command_buffer];
    let submit_infos = [vk::SubmitInfo::default().command_buffers(&command_buffers)];

    unsafe {
        device.queue_submit(queue, &submit_infos, vk::Fence::null())?;
        device.queue_wait_idle(queue)?;
    }

    Ok(())
}
