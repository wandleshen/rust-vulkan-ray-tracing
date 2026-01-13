# Vulkan Ray Tracing

基于 Vulkan Ray Tracing 扩展实现的路径追踪渲染器，使用 Rust 和 ash 库开发。

## 功能特性

- 基于物理的路径追踪渲染
- 支持三种材质类型：Lambertian（漫反射）、Metal（金属）、Dielectric（电介质/玻璃）
- 程序化生成的球体场景（类似 Ray Tracing in One Weekend 最终场景）
- 支持景深效果
- 双模式运行：窗口实时预览 / 无头离线渲染

## 系统要求

- Rust 工具链
- Vulkan SDK

## 构建

```bash
cargo build --release
```

## 运行

```bash
cargo run --release
```

## 配置

在 `src/main.rs` 开头可以修改以下配置：

```rust
// 渲染模式
const HEADLESS_MODE: bool = false;  // true = 无头模式, false = 窗口模式
const PREVIEW_INTERVAL: u32 = 100;  // 窗口模式下每多少个 sample 更新显示

// 图像设置
const WIDTH: u32 = 1200;
const HEIGHT: u32 = 800;
const N_SAMPLES: u32 = 5000;        // 总采样数
const N_SAMPLES_ITER: u32 = 100;    // 每批次采样数（无头模式）
```

### 窗口模式

设置 `HEADLESS_MODE = false` 启用窗口模式：
- 显示实时渲染预览窗口
- 每 `PREVIEW_INTERVAL` 个 sample 更新一次显示
- 可随时关闭窗口停止渲染
- 渲染完成后输出 `out.png`

### 无头模式

设置 `HEADLESS_MODE = true` 启用无头模式：
- 无窗口，仅命令行显示进度
- 适合服务器或后台渲染
- 渲染完成后输出 `out.png`

## 示例输出

渲染完成后会在项目根目录生成 `out.png`，包含：
- 地面：大型漫反射球体
- 中心三个大球：玻璃、漫反射、金属材质
- 周围随机分布的小球：随机材质和颜色
