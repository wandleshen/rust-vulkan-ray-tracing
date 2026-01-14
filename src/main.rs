fn main() -> Result<(), Box<dyn std::error::Error>> {
    const HEADLESS_MODE: bool = false;
    const WIDTH: u32 = 1200;
    const HEIGHT: u32 = 800;

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

    while !HEADLESS_MODE {
        glfw.poll_events();

        if let Some(win) = window.as_ref() {
            if win.should_close() {
                break;
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    Ok(())
}
