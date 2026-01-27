use vulkan_raytracing::*;
use ash::khr;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ========== 渲染配置 ==========
    const HEADLESS_MODE: bool = false;
    const WIDTH: u32 = 1200;
    const HEIGHT: u32 = 800;

    // ========== GLFW 初始化 ==========
    let mut glfw = glfw::init(glfw::fail_on_errors)?;
    let window = if !HEADLESS_MODE {
        glfw.window_hint(glfw::WindowHint::ClientApi(glfw::ClientApiHint::NoApi));
        glfw.window_hint(glfw::WindowHint::Resizable(false));
        let (mut win, _events) = glfw
            .create_window(
                WIDTH,
                HEIGHT,
                "Vulkan Raytracing",
                glfw::WindowMode::Windowed,
            )
            .expect("Failed to create GLFW window.");

        win.set_key_callback(|window, key, _scancode, action, _modifiers| {
            if key == glfw::Key::Escape && action == glfw::Action::Press {
                window.set_should_close(true);
            }
        });

        Some(win)
    } else {
        None
    };

    // ========== 验证层设置 ==========
    let validation = ValidationLayerConfig::new();
    let entry = unsafe { ash::Entry::load() }?;
    assert!(validation.check_support(&entry)?, "Validation layer not supported");

    // ========== Vulkan Instance 创建 ==========
    let instance_extensions = get_instance_extensions(HEADLESS_MODE);
    let instance = create_instance(
        &entry,
        &validation.as_ptrs(),
        &instance_extensions,
        validation.enabled,
    )?;

    println!("Vulkan Instance created successfully");

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

    if surface.is_some() {
        println!("Surface created successfully");
    }

    // ========== 物理设备和队列族选择 ==========
    let (physical_device, queue_indices) = pick_physical_device_and_queue_family_indices(
        &instance,
        surface_loader.as_ref(),
        surface,
        &[
            khr::acceleration_structure::NAME,
            khr::deferred_host_operations::NAME,
            khr::ray_tracing_pipeline::NAME,
        ],
        true, // need_compute: 光线追踪需要 compute 队列
    )?
    .ok_or("No suitable physical device found")?;

    let graphics_queue_index = queue_indices.graphics_family.unwrap();

    // 打印物理设备信息
    let device_properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let device_name = unsafe {
        std::ffi::CStr::from_ptr(device_properties.device_name.as_ptr())
            .to_string_lossy()
    };
    println!("Selected physical device: {}", device_name);
    println!("Graphics queue family index: {}", graphics_queue_index);
    if let Some(compute_index) = queue_indices.compute_family {
        println!("Compute queue family index: {}", compute_index);
    }
    if let Some(present_index) = queue_indices.present_family {
        println!("Present queue family index: {}", present_index);
    }

    // ========== 逻辑设备创建 ==========
    let device = create_device(&instance, physical_device, &queue_indices, HEADLESS_MODE)?;

    println!("Logical device created successfully");

    // 获取队列
    let graphics_queue = unsafe { device.get_device_queue(graphics_queue_index, 0) };
    println!("Graphics queue obtained: {:?}", graphics_queue);

    // ========== Swapchain 创建 ==========
    let swapchain = if !HEADLESS_MODE && surface.is_some() {
        let sc = Swapchain::new(
            &instance,
            &device,
            physical_device,
            surface.unwrap(),
            surface_loader.as_ref().unwrap(),
            WIDTH,
            HEIGHT,
        )?;
        println!(
            "Swapchain created: format={:?}, extent={}x{}, images={}",
            sc.format,
            sc.extent.width,
            sc.extent.height,
            sc.images.len()
        );
        Some(sc)
    } else {
        None
    };

    // ========== 主循环 ==========
    while !HEADLESS_MODE {
        glfw.poll_events();
        if let Some(win) = window.as_ref() {
            if win.should_close() {
                break;
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    // ========== 资源清理 ==========
    println!("Cleaning up resources...");

    unsafe {
        device.device_wait_idle()?;

        // 销毁 Swapchain
        if let Some(sc) = swapchain {
            sc.destroy(&device);
        }

        // 销毁逻辑设备
        device.destroy_device(None);

        // 销毁 Surface
        if let Some(s) = surface {
            if let Some(loader) = surface_loader.as_ref() {
                loader.destroy_surface(s, None);
            }
        }

        // 销毁 Instance
        instance.destroy_instance(None);
    } 
    Ok(())
}
