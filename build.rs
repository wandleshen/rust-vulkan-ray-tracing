use std::error::Error;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = std::env::var("OUT_DIR")?;
    let shader_dir = Path::new("shaders");

    // 编译所有 shader 文件
    let shaders = vec![
        ("raygen.rgen.shader", shaderc::ShaderKind::RayGeneration),
        ("miss.rmiss.shader", shaderc::ShaderKind::Miss),
        ("closesthit.rchit.shader", shaderc::ShaderKind::ClosestHit),
        (
            "intersection.rint.shader",
            shaderc::ShaderKind::Intersection,
        ),
        ("blit.vert.shader", shaderc::ShaderKind::Vertex),
        ("blit.frag.shader", shaderc::ShaderKind::Fragment),
    ];

    let compiler = shaderc::Compiler::new().unwrap();
    let mut options = shaderc::CompileOptions::new().unwrap();
    options.set_target_env(
        shaderc::TargetEnv::Vulkan,
        shaderc::EnvVersion::Vulkan1_4 as u32,
    );
    options.set_target_spirv(shaderc::SpirvVersion::V1_6);

    for (shader_file, kind) in shaders {
        let shader_path = shader_dir.join(shader_file);
        let source = fs::read_to_string(&shader_path)?;

        let artifact =
            compiler.compile_into_spirv(&source, kind, shader_file, "main", Some(&options))?;

        let output_name = shader_file.strip_suffix(".shader").unwrap_or(shader_file);
        let output_file = format!("{}/{}.spv", out_dir, output_name);
        fs::write(&output_file, artifact.as_binary_u8())?;

        println!("cargo:rerun-if-changed={}", shader_path.display());
    }

    Ok(())
}
