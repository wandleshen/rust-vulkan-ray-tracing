use ash::prelude::VkResult;
use ash::{Device, Entry, Instance, ext, khr, vk};
use std::collections::HashSet;
use std::ffi::{CStr, CString, c_void};
use std::os::raw::c_char;

pub struct ValidationLayerConfig {
    pub layers: Vec<CString>,
    pub enabled: bool,
}

impl ValidationLayerConfig {
    /// 创建验证层配置（debug 模式启用，release 模式禁用）
    pub fn new() -> Self {
        #[cfg(debug_assertions)]
        let layers = vec![CString::new("VK_LAYER_KHRONOS_validation").unwrap()];
        #[cfg(not(debug_assertions))]
        let layers = Vec::new();

        let enabled = !layers.is_empty();
        Self { layers, enabled }
    }

    /// 获取层名称指针列表
    pub fn as_ptrs(&self) -> Vec<*const i8> {
        self.layers.iter().map(|c_str| c_str.as_ptr()).collect()
    }

    /// 检查验证层是否支持
    pub fn check_support(&self, entry: &Entry) -> VkResult<bool> {
        if !self.enabled {
            return Ok(true);
        }
        unsafe { check_validation_layer_support(entry, self.layers.iter().map(|c| c.as_c_str())) }
    }
}

impl Default for ValidationLayerConfig {
    fn default() -> Self {
        Self::new()
    }
}

pub unsafe extern "system" fn default_vulkan_debug_utils_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _p_user_data: *mut c_void,
) -> vk::Bool32 {
    unsafe {
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
    }
}

pub unsafe fn check_validation_layer_support<'a>(
    entry: &Entry,
    required_validation_layers: impl IntoIterator<Item = &'a CStr>,
) -> VkResult<bool> {
    unsafe {
        let supported_layers: HashSet<CString> = entry
            .enumerate_instance_layer_properties()?
            .into_iter()
            .map(|layer_property| CStr::from_ptr(layer_property.layer_name.as_ptr()).to_owned())
            .collect();

        Ok(required_validation_layers
            .into_iter()
            .all(|l| supported_layers.contains(l)))
    }
}

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

pub fn create_instance(
    entry: &Entry,
    validation_layers: &[*const i8],
    instance_extensions: &[*const i8],
    enable_validation: bool,
) -> VkResult<Instance> {
    let application_name =
        CString::new("Vulkan Ray Tracing").expect("Failed to create application name");
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

    let instance_create_info: vk::InstanceCreateInfo<'_> = if enable_validation {
        instance_create_info.push_next(&mut debug_utils_create_info)
    } else {
        instance_create_info
    };

    unsafe { entry.create_instance(&instance_create_info, None) }
}

/// 队列族索引
#[derive(Default, Clone, Copy, Debug)]
pub struct QueueFamilyIndices {
    pub graphics_family: Option<u32>,
    pub compute_family: Option<u32>,
    pub present_family: Option<u32>,
}

impl QueueFamilyIndices {
    /// 检查是否满足要求
    /// - need_compute: 是否需要 compute 队列
    /// - need_present: 是否需要 present 队列
    pub fn is_complete(&self, need_compute: bool, need_present: bool) -> bool {
        let has_graphics = self.graphics_family.is_some();
        let has_compute = !need_compute || self.compute_family.is_some();
        let has_present = !need_present || self.present_family.is_some();
        has_graphics && has_compute && has_present
    }

    /// 获取唯一的队列族索引列表（用于创建设备时避免重复）
    pub fn unique_families(&self) -> Vec<u32> {
        let mut families = Vec::new();
        if let Some(g) = self.graphics_family {
            families.push(g);
        }
        if let Some(c) = self.compute_family {
            if !families.contains(&c) {
                families.push(c);
            }
        }
        if let Some(p) = self.present_family {
            if !families.contains(&p) {
                families.push(p);
            }
        }
        families
    }
}

pub fn pick_physical_device_and_queue_family_indices(
    instance: &Instance,
    surface_loader: Option<&khr::surface::Instance>,
    surface: Option<vk::SurfaceKHR>,
    extensions: &[&CStr],
    need_compute: bool,
) -> VkResult<Option<(vk::PhysicalDevice, QueueFamilyIndices)>> {
    let need_present = surface.is_some();

    Ok(unsafe { instance.enumerate_physical_devices() }?
        .into_iter()
        .find_map(|physical_device| {
            // 检查设备扩展支持
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

            let queue_families =
                unsafe { instance.get_physical_device_queue_family_properties(physical_device) };

            let mut indices = QueueFamilyIndices::default();

            // 查找图形队列族
            if let Some(graphics_index) = queue_families
                .iter()
                .enumerate()
                .find(|(_, properties)| {
                    properties.queue_count > 0
                        && properties.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                })
                .map(|(i, _)| i as u32)
            {
                indices.graphics_family = Some(graphics_index);
            }

            // 查找计算队列族
            if need_compute {
                if let Some(compute_index) = queue_families
                    .iter()
                    .enumerate()
                    .find(|(_, properties)| {
                        properties.queue_count > 0
                            && properties.queue_flags.contains(vk::QueueFlags::COMPUTE)
                    })
                    .map(|(i, _)| i as u32)
                {
                    indices.compute_family = Some(compute_index);
                }
            }

            // 查找呈现队列族
            if let (Some(loader), Some(surf)) = (surface_loader, surface) {
                if let Some(present_index) = queue_families
                    .iter()
                    .enumerate()
                    .find(|(i, _)| {
                        unsafe {
                            loader
                                .get_physical_device_surface_support(physical_device, *i as u32, surf)
                                .unwrap_or(false)
                        }
                    })
                    .map(|(i, _)| i as u32)
                {
                    indices.present_family = Some(present_index);
                }
            }

            // 检查是否满足要求
            if indices.is_complete(need_compute, need_present) {
                Some((physical_device, indices))
            } else {
                None
            }
        }))
}

pub fn create_device(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    queue_indices: &QueueFamilyIndices,
    headless_mode: bool,
) -> VkResult<Device> {
    let priorities = [1.0];

    // 为每个唯一的队列族创建 QueueCreateInfo
    let queue_create_infos: Vec<vk::DeviceQueueCreateInfo> = queue_indices
        .unique_families()
        .iter()
        .map(|&index| {
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(index)
                .queue_priorities(&priorities)
        })
        .collect();

    let mut features2 = vk::PhysicalDeviceFeatures2::default();

    let mut features12 = vk::PhysicalDeviceVulkan12Features::default()
        .buffer_device_address(true)
        .scalar_block_layout(true);

    let mut as_feature = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
        .acceleration_structure(true);

    let mut raytracing_pipeline =
        vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default().ray_tracing_pipeline(true);

    let mut enabled_extension_names = vec![
        vk::KHR_RAY_TRACING_PIPELINE_NAME.as_ptr(),
        vk::KHR_ACCELERATION_STRUCTURE_NAME.as_ptr(),
        vk::KHR_DEFERRED_HOST_OPERATIONS_NAME.as_ptr(),
        vk::KHR_SPIRV_1_4_NAME.as_ptr(),
        vk::EXT_SCALAR_BLOCK_LAYOUT_NAME.as_ptr(),
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