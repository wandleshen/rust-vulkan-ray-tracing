# 光照、材质与路径追踪实现详解

本文档面向当前仓库中已经实现的 Vulkan Ray Tracing 路径追踪器，重点解释以下几部分：

- 多光源统一采样；
- HDR 环境光与环境 PDF 设计；
- 统一 BSDF / BTDF / clearcoat 框架；
- 粗糙透射与 Beer-Lambert 吸收；
- MIS、RR、路径吞吐量更新；
- 对应代码位置与数据结构。

文中提到的核心代码主要位于：

- `shaders/raygen.rgen.glsl`
- `shaders/closesthit.rchit.glsl`
- `shaders/miss.rmiss.glsl`
- `src/light.rs`
- `src/environment.rs`
- `src/material.rs`
- `src/pipeline.rs`
- `src/main.rs`
- `src/scene.rs`

---

## 1. 整体架构概览

当前渲染器的职责分布如下：

1. **CPU 侧**负责构造场景、材质、光源列表、环境贴图采样表；
2. **`src/pipeline.rs`** 负责把这些数据挂到 ray tracing descriptor set；
3. **`raygen` shader** 完成真正的路径追踪主循环；
4. **`closesthit` shader** 输出交点位置、法线、材质索引、front-face 标记与命中距离；
5. **`miss` shader** 仅负责告诉 `raygen` 本次追踪没有命中几何体；
6. **`anyhit` shader** 负责忽略“不可见材质”的交点，例如点光代理球或被隐藏的面积光代理球。

从渲染流程上看，这个实现已经是一个标准的 **unidirectional path tracer**：

- 主光线从相机出发；
- 命中后先做一次显式光源采样（next event estimation）；
- 再从 BSDF 分布中采样一个新的方向继续反弹；
- 在 miss 或命中发光体时累积辐射亮度；
- 用 MIS 混合“光源采样”和“BSDF 采样”两类路径；
- 用 Russian Roulette 截断长路径；
- 用 Beer-Lambert 处理透射介质内的吸收。

---

## 2. CPU 侧数据组织

### 2.1 材质参数布局

材质结构定义在 `src/material.rs`：

```rust
pub struct Material {
    pub base_color: Vec4,
    pub emission: Vec4,
    pub params: Vec4,
    pub medium: Vec4,
}
```

四个 `vec4` 的语义如下：

- `base_color.rgb`：基础颜色 / 金属 F0 颜色 / 透射 tint；
- `base_color.a`：specular 强度；
- `emission.rgb`：自发光辐射亮度；
- `emission.a`：clearcoat 权重；
- `params.x`：roughness；
- `params.y`：metallic；
- `params.z`：transmission；
- `params.w`：ior；
- `medium.rgb`：吸收系数 `sigma_a`；
- `medium.w`：吸收密度；若 `< 0` 则表示 invisible sentinel。

因此，当前实现已经从最初的“toy-like 漫反射 / 金属 / 折射开关”升级为一个 **principled-like 参数化表面模型**。

### 2.2 多光源列表

光源列表定义在 `src/light.rs`。

当前不再使用“单一 light mode”作为 shader 内部分支，而是统一构造成一个固定上限的 light list：

- `Sky / Environment`
- `Point`
- `Directional`
- `Area Sphere`

每个光源条目 `GpuLight` 都包含：

- 位置与半径；
- 方向与类型；
- 发光强度与选择 PMF；
- 选择 CDF、可见 emissive 标记、功率估计。

CPU 侧会为每个光源估算一个“采样功率”：

- 环境光：由环境图平均亮度估算；
- 点光：按 `4πI` 估算；
- 平行光：按一个经验尺度估算；
- 面积球光：按 `luminance * area` 估算。

然后对这些功率归一化，得到光源选择 PMF：

$$
P(L_i) = \frac{w_i}{\sum_j w_j}
$$

再累积为 CDF，供 shader 在 `sampleLight(...)` 中按概率选灯。

### 2.3 HDR 环境贴图与采样表

环境数据在 `src/environment.rs` 中生成。

当前实现使用的是 **内置生成的 HDR 环境贴图缓冲**，而不是读取外部 `.hdr` 文件；但从 shader 视角看，它已经是一个真正的经纬度环境贴图，包含：

- `texels`：环境贴图颜色；
- `pmf`：每个 texel 的离散概率质量；
- `conditional_cdf`：每一行内的条件 CDF；
- `marginal_cdf`：按行的边缘 CDF。

环境图的权重定义为：

$$
 w(x,y) = L(x,y) \cdot \sin\theta
$$

其中：

- `L(x,y)` 是 texel 亮度；
- `\theta` 是该 texel 的纬度角；
- `sin(theta)` 用来补偿经纬度映射在球面上的面积畸变。

这正是经典环境贴图 importance sampling 的标准做法。

---

## 3. Descriptor 与 GPU 数据通路

`src/pipeline.rs` 中的 descriptor set 现在包含以下 binding：

- `0`：TLAS
- `1`：输出图像
- `2`：材质缓冲
- `3`：相机 / frame 数据
- `4`：光源列表缓冲
- `5`：环境贴图 texel 缓冲
- `6`：环境贴图 PMF 缓冲
- `7`：环境贴图行内条件 CDF 缓冲
- `8`：环境贴图边缘 CDF 缓冲

这使 shader 可以在一次路径追踪过程中同时访问：

- 任意表面材质；
- 任意光源；
- 任意环境方向对应的 HDR 辐射亮度；
- 环境采样所需的 PDF / CDF 表。

---

## 4. 光线追踪主循环

路径追踪主循环位于 `shaders/raygen.rgen.glsl` 的 `main()` 中。

### 4.1 初始阶段

每个像素会：

1. 基于 `gl_LaunchIDEXT` 和 push constant seed 生成随机种子；
2. 做子像素抖动；
3. 用景深透镜模型生成相机光线；
4. 初始化：
   - `throughput = 1`
   - `radiance = 0`
   - `previousBsdfPdf = 1`
   - `previousBounceWasDelta = true`

### 4.2 每次 bounce 的执行顺序

对每次反弹，顺序是：

1. `traceRayEXT(...)` 追踪主光线；
2. 若 miss：
   - 读取环境背景；
   - 若上一跳不是 delta，则与光源 PDF 做 MIS；
   - 终止路径；
3. 若命中：
   - 若当前处于介质内部，按 `exp(-sigma_a * distance)` 衰减 throughput；
   - 读取材质；
   - 如果命中的是 emissive 物体，则累积发光并用 MIS 修正；
   - 否则先进行显式光源采样；
   - 再从 BSDF 采样新的反弹方向；
   - 更新 throughput；
   - 若发生透射，则更新 medium stack；
   - 进行 RR。

这个顺序意味着当前实现属于：

- **surface-only path tracing**
- **next event estimation**
- **surface absorption via medium state**

并没有做体散射，仅做均匀介质吸收。

---

## 5. 几何命中与 payload

### 5.1 `closesthit`

`shaders/closesthit.rchit.glsl` 会输出：

- `position`
- `normal`
- `material`
- `frontFace`
- `distance`

其中 `frontFace` 的定义是：

- 若射线与几何法线相对，则为 true；
- 否则将法线翻转，使 shader 内使用的 shading normal 始终朝向入射视线半球。

这非常重要，因为后续 BSDF 计算都假设：

$$
\mathbf{n} \cdot \mathbf{w_i} > 0
$$

其中 `wi = -incomingDirection`。

### 5.2 `miss`

`shaders/miss.rmiss.glsl` 只负责：

- `payload.isMiss = 1`
- `payload.distance = 0`

真正的环境光读取发生在 `raygen` 中，而不是在 `miss` shader 中直接写颜色。

这样做的优点是：

- miss 与 hit 路径逻辑都留在一个地方；
- 更容易做 MIS；
- 更容易处理路径吞吐量与 medium 状态。

---

## 6. 统一光源采样框架

### 6.1 光源选择 PMF

`sampleLight(...)` 的第一步不是直接采样某个具体光源，而是：

1. 用随机数在 light list 的 CDF 上选一个光源；
2. 在该光源内部执行条件采样；
3. 返回 **总 PDF**：

$$
 p(\omega) = P(L_i) \cdot p(\omega \mid L_i)
$$

这正是“多光源统一采样”的核心。

### 6.2 点光源

点光是 **delta light**。

- 方向是确定的：`lightPos - surfacePos`
- 条件 PDF 视为 1
- 总 PDF 为：

$$
 p = P(L_{point})
$$

其辐射强度按反平方衰减：

$$
 L = \frac{I}{r^2}
$$

### 6.3 平行光

平行光同样是 **delta light**。

- 方向固定；
- 条件 PDF = 1；
- 总 PDF = 选择 PMF。

### 6.4 球面积光

球面积光先在球面上均匀采样一点：

- 面面积 PDF：

$$
 p_A = \frac{1}{4\pi r^2}
$$

- 再转成方向域 PDF：

$$
 p_\omega = p_A \cdot \frac{d^2}{|n_l \cdot (-\omega)|}
$$

其中：

- `d` 是着色点到光源采样点的距离；
- `n_l` 是光源表面法线；
- `omega` 是从着色点指向光源点的方向。

总 PDF 为：

$$
 p = P(L_{area}) \cdot p_\omega
$$

### 6.5 环境光

环境光的采样分两层：

1. 先按 light list 选中 environment light；
2. 再按环境贴图的二维分布采样方向。

环境图方向采样使用：

- 行边缘 CDF：选择 `y`
- 行内条件 CDF：选择 `x`
- 在 texel 内再做 jitter

方向域 PDF 为：

$$
 p_\omega(\omega) = \frac{p_{texel}(x,y)}{\Delta \omega_{x,y}}
$$

其中 texel 对应的球面立体角：

$$
 \Delta \omega_{x,y} = \Delta \phi \cdot (\cos\theta_0 - \cos\theta_1)
$$

总 PDF 则为：

$$
 p(\omega) = P(L_{env}) \cdot p_\omega(\omega)
$$

这正是当前实现中 `lightPdfForMiss(...)` 的基础。

---

## 7. BSDF / BTDF 设计

统一 BSDF 入口在 `shaders/raygen.rgen.glsl`：

- `evaluateBSDF(...)`
- `pdfBSDF(...)`
- `sampleBSDF(...)`

### 7.1 lobe 划分

当前材质由四类 lobe 组成：

1. **Diffuse**：Lambert 漫反射
2. **Specular Reflection**：GGX 微表面镜面反射
3. **Transmission**：GGX 微表面粗糙透射
4. **Clearcoat**：额外的高光清漆层

对于极低 roughness 的金属与透射材质，代码会自动退化为 **delta 分支**，避免在极尖峰分布上用连续 PDF 造成黑边或高噪声。

### 7.2 Diffuse

漫反射实现是经典 Lambert：

$$
 f_d = \frac{c_{base}}{\pi}
$$

其采样采用 cosine-weighted hemisphere：

$$
 p_d(\omega_o) = \frac{\max(n \cdot \omega_o, 0)}{\pi}
$$

Diffuse 的整体权重由：

$$
 w_d = (1 - metallic)(1 - transmission)
$$

控制。

### 7.3 GGX 镜面反射

镜面反射实现采用 Cook-Torrance + GGX：

$$
 f_r = \frac{D(h)G(\omega_i, \omega_o)F(\omega_i, h)}{4|n \cdot \omega_i||n \cdot \omega_o|}
$$

其中：

- `D`：GGX NDF
- `G`：Smith 几何项
- `F`：Schlick Fresnel

`F0` 的构造方式为：

$$
 F_0 = mix(F_{0,dielectric}, baseColor, metallic)
$$

其中 dielectric 的 `F0` 来自 IOR：

$$
 F_{0,dielectric} = \left(\frac{ior - 1}{ior + 1}\right)^2
$$

因此当前实现已经不是“只有金属才有镜面”，而是 **非金属也有物理意义上的 Fresnel 反射**。

### 7.4 GGX 粗糙透射

粗糙透射部分使用 GGX 微表面折射近似。

代码会先根据：

$$
 h = normalize(w_i + \eta w_o)
$$

构造折射半向量，再计算近似 BTDF：

$$
 f_t \propto (1 - F)DG \cdot \eta^2
\cdot \frac{|(w_i \cdot h)(w_o \cdot h)|}{|n \cdot w_i||n \cdot w_o|(w_i \cdot h + \eta w_o \cdot h)^2}
$$

当前实现中还乘了：

- `baseColor.rgb` 作为透射 tint；
- `transmission` 作为透射强度；
- `metallic` 会抑制 transmission。

这部分已经足以让玻璃类物体从“能折射”升级到“像一个粗糙介质界面”。

### 7.5 Delta 透射与 Delta 金属

当 roughness 很小时：

- 透射材质走 `sampleDeltaTransmissionBSDF(...)`
- 金属材质走 `sampleDeltaMetalBSDF(...)`

原因是：

- 对极窄高光 / 折射峰做连续采样会非常难收敛；
- delta 处理在路径追踪中更稳定；
- 同时能保留精确镜像 / 玻璃效果。

### 7.6 Clearcoat

当前 clearcoat 作为额外的镜面 lobe 引入：

- 权重来自 `emission.a`
- 使用更窄的 roughness
- F0 固定接近 `0.04`

这更接近 Disney / UE 材质模型中的“薄清漆层”概念。

---

## 8. BSDF 采样 PDF 设计

### 8.1 混合分布

BSDF 并不是对某一个 lobe 采样，而是先构造一个离散混合分布：

$$
P(lobe) = \frac{w_{lobe}}{\sum_k w_k}
$$

当前实现考虑的权重包括：

- diffuse weight
- specular reflection weight
- transmission weight
- clearcoat weight

然后：

1. 先抽一个 lobe；
2. 在该 lobe 内采样方向；
3. 总 PDF 为该混合 PDF。

### 8.2 总 PDF

因此，对于同半球反射：

$$
 p(\omega_o) = P_d p_d + P_s p_s + P_c p_c
$$

对于异半球透射：

$$
 p(\omega_o) = P_t p_t
$$

这个设计直接对应代码中的 `pdfBSDF(...)`。

### 8.3 throughput 更新

对于非 delta 样本，吞吐量更新为：

$$
 throughput \leftarrow throughput \cdot \frac{f(\omega_i, \omega_o) |n \cdot \omega_o|}{p(\omega_o)}
$$

这正是 `BsdfSample.weight` 的来源。

---

## 9. MIS：光源采样与 BSDF 采样的融合

当前实现使用 **power heuristic**：

$$
 w_a = \frac{p_a^2}{p_a^2 + p_b^2}
$$

使用点主要有两处：

1. **显式光源采样**：
   - 光源给出一个方向；
   - 用 `pdfBSDF(...)` 算 BSDF 在该方向上的 PDF；
   - 对非 delta 光源做 MIS。

2. **BSDF 采样命中 emissive / miss 环境时**：
   - 若上一跳不是 delta，则需要计算“这个方向若由光源采样得到”的 PDF；
   - 环境命中时调用 `lightPdfForMiss(...)`；
   - 面积光命中时调用 `lightPdfForEmissiveHit(...)`。

为什么 delta 光不用 MIS？

因为 delta 分布在普通方向域下不可积，不适合和连续 PDF 直接比较，所以这里对 delta light / delta BSDF 直接给权重 1。

---

## 10. Beer-Lambert 吸收与 medium stack

### 10.1 基本公式

均匀吸收介质使用 Beer-Lambert：

$$
 T(d) = e^{-\sigma_a d}
$$

其中：

- `sigma_a` 为 RGB 吸收系数；
- `d` 为当前这段路径在介质内走过的距离。

### 10.2 当前实现方式

当前 `raygen` 中维护了一个简单的 `mediumStack`：

- 透射进入物体时 push 当前材质的吸收参数；
- 透射离开物体时 pop；
- 在每次命中后，根据 `payload.distance` 对 throughput 乘上：

$$
 throughput *= e^{-\sigma_a d}
$$

这意味着：

- 吸收是按路径段计算的；
- 只对真正“穿过界面”的 transmission 事件生效；
- 纯反射不会改变 medium state。

### 10.3 局限性

当前 medium stack 是轻量版本，适合本 demo：

- 支持简单进入 / 离开；
- 对复杂嵌套介质没有做介质 ID 严格匹配；
- 没有做体散射，只做吸收。

但对玻璃 / 彩色介质外观已经足够有帮助。

---

## 11. Russian Roulette

在 bounce 深度较大后，继续追踪所有路径会越来越浪费。

因此当前实现使用：

$$
 p_{rr} = clamp(max(throughput.r, throughput.g, throughput.b), 0.05, 0.95)
$$

- 若随机数大于 `p_rr`，路径终止；
- 否则：

$$
 throughput /= p_{rr}
$$

这样可以保证估计仍然无偏，同时减少长路径开销。

---

## 12. 当前场景中的光源与材质效果

### 12.1 光源

当前默认支持并可同时启用：

- HDR environment
- point light（delta，默认不可见几何代理）
- directional light（delta）
- area sphere light（可见 emissive 几何）

在窗口模式下，`1/2/3/4` 会切换各自的启用状态，而不再是“互斥模式切换”。

### 12.2 材质

当前场景中已经用到了：

- diffuse + clearcoat
- metal + GGX reflection
- dielectric + rough transmission
- absorption tinted glass
- emissive area light material
- invisible sentinel material

因此 demo 已经能覆盖：

- 漫反射
- 金属高光
- 粗糙玻璃
- 彩色吸收
- 面光 + 环境光 + delta 光混合

---

## 13. 仍然存在的局限与后续可扩展方向

虽然现在已经比最初版本完整很多，但仍有一些可以继续改进的点：

1. **外部 HDRI 载入**
   - 当前环境图是内置生成的 HDR texel buffer；
   - 若后续加入 `.hdr/.exr` 载入，就能直接替换为真实摄影环境贴图。

2. **visible normal sampling / VNDF**
   - 当前 GGX 采样使用普通 NDF half-vector sampling；
   - 若改为 VNDF，可进一步降低高 roughness 下的方差。

3. **更完整的 principled 参数**
   - 可继续加入 sheen、anisotropy、specular tint 等。

4. **透射阴影 / colored shadow**
   - 当前阴影可见性仍以遮挡为主；
   - 若要让光穿过玻璃并携带吸收色，需要做更复杂的 shadow transmittance。

5. **更严格的 medium management**
   - 对嵌套介质可加入 medium ID 与更稳定的 push/pop 匹配机制。

---

## 14. 代码索引

为了便于继续阅读源码，建议按下面顺序看：

1. `src/material.rs`
   - 理解材质参数编码。
2. `src/light.rs`
   - 理解 light list、PMF/CDF 的 CPU 构造。
3. `src/environment.rs`
   - 理解 HDR 环境图生成与二维 importance sampling 表。
4. `src/pipeline.rs`
   - 理解 descriptor binding 如何把这些数据喂给 shader。
5. `shaders/closesthit.rchit.glsl`
   - 理解 payload 的几何语义。
6. `shaders/raygen.rgen.glsl`
   - 先看 `main()`；
   - 再看 `sampleLight(...)` 与 `lightPdfForMiss(...)`；
   - 再看 `evaluateBSDF(...) / pdfBSDF(...) / sampleBSDF(...)`；
   - 最后看粗糙透射与 medium stack。

如果你是按算法学习，推荐阅读顺序是：

- 先看第 4 节和第 9 节，理解路径是怎么走的；
- 再看第 6～8 节，理解材质、采样和 PDF；
- 最后看第 10 节，理解吸收是怎么挂进路径里的。

---

## 15. 环境 miss 分支与 MIS 代码解读

下面这一段代码位于 `shaders/raygen.rgen.glsl:1304`，它处理的是“当前路径没有再命中任何几何体，而是直接看到环境贴图”的情况：

```glsl
if (payload.isMiss == 1u) {
    vec3 backgroundRadiance = evaluateBackground(ray.direction);
    if (depth == 0 || previousBounceWasDelta) {
        radiance += throughput * backgroundRadiance;
    } else {
        float lightPdf = lightPdfForMiss(ray.direction);
        float weight = lightPdf > 0.0
            ? powerHeuristic(previousBsdfPdf, lightPdf)
            : 1.0;
        radiance += throughput * backgroundRadiance * weight;
    }
    break;
}
```

### 15.1 这段代码在做什么

它的语义可以概括为：

1. 这次 `traceRayEXT(...)` 没有命中场景几何；
2. 当前光线离开场景，看到的是环境光；
3. 取出当前方向上的环境辐射亮度；
4. 如果这是主光线，或者上一跳是 delta 事件，则直接累积；
5. 否则，要把“BSDF 采样命中环境”与“环境光显式采样”做 MIS；
6. 累积后终止路径。

这段逻辑是 path tracing 中非常典型的一步：

- 命中表面时，既会做 next event estimation；
- 也会从 BSDF 继续采样一条新的反弹方向；
- 如果这条 BSDF 采样的方向最后直接看到环境，就需要和“环境光显式采样”那一路做去重式加权。

### 15.2 `payload.isMiss == 1u` 是什么

`payload` 是 ray tracing shader 之间传递的命中结果结构。

- 在 `shaders/closesthit.rchit.glsl:28` 中，命中几何时会写入 `payload.isMiss = 0`；
- 在 `shaders/miss.rmiss.glsl:13` 中，miss shader 会写入 `payload.isMiss = 1`。

所以这里的判断含义就是：

- 这条路径没有再打到任何球体；
- 它最终看到的是环境贴图，而不是某个物体表面。

### 15.3 `evaluateBackground(ray.direction)` 是什么

`evaluateBackground(...)` 的作用是：

- 给定一个方向 `ray.direction`；
- 返回这个方向在环境光中的辐射亮度 `L_env(omega)`。

在当前实现中，这意味着：

- 先把方向转成环境图 UV；
- 再在环境贴图中做双线性读取；
- 返回该方向的 HDR 颜色。

所以：

```glsl
vec3 backgroundRadiance = evaluateBackground(ray.direction);
```

可以理解为：

> “当前这条光线看向天空的这个方向时，天空发过来的真实环境光亮度是多少？”

### 15.4 `throughput` 是什么

`throughput` 是路径吞吐量，表示“路径到当前为止还剩下多少能量”。

它会在每一跳被不断乘上：

- BRDF / BTDF 值；
- 余弦项；
- 除以采样 PDF；
- 以及介质吸收项。

因此：

```glsl
radiance += throughput * backgroundRadiance;
```

的含义不是“把背景颜色直接加进去”，而是：

> “把这条路径走到当前方向时所携带的能量，乘上它最终看到的环境辐射亮度，作为本条路径的终点贡献累积进像素。”

### 15.5 `depth == 0` 为什么直接累积

`depth == 0` 表示当前是主光线第一次追踪就 miss 了，也就是：

- 相机直接看到了天空；
- 中间没有经过任何表面反弹。

这种情况下，没有必要做 MIS，因为：

- 这不是“BSDF 采样路径”和“显式光源采样路径”的重合问题；
- 它只是相机直接看到背景。

所以这里直接：

```glsl
radiance += throughput * backgroundRadiance;
```

### 15.6 `previousBounceWasDelta` 是什么

它表示“上一跳 BSDF 事件是不是 delta 事件”。

delta 事件包括：

- 理想镜面反射；
- 理想折射；
- 以及非常低 roughness 时退化成的 delta 金属 / delta 玻璃。

为什么 delta 很特殊？

因为 delta 分布不是普通连续方向分布，它在方向空间里相当于一个尖脉冲：

- 没法像普通 diffuse / glossy 一样用连续 PDF 去和光源 PDF 做稳定比较；
- 所以通常不对 delta 路径做常规的 MIS 混合。

因此当：

```glsl
depth == 0 || previousBounceWasDelta
```

成立时，代码会直接累积环境项，不乘 MIS 权重。

### 15.7 `lightPdfForMiss(ray.direction)` 是什么

它表示：

> “如果不是靠 BSDF 采样走到这个环境方向，而是靠光源采样去采环境光，那么采到当前这个方向的 PDF 是多少？”

这正是 MIS 所需要的“另一条采样路径的概率”。

在当前实现里，`lightPdfForMiss(...)` 包含两层含义：

$$
p_{light}(\omega) = P(L_{env}) \cdot p(\omega \mid L_{env})
$$

其中：

- `P(L_env)` 是在多光源列表里选中 environment light 的 PMF；
- `p(omega | L_env)` 是环境贴图 importance sampling 在该方向上的条件 PDF。

所以它不是简单的“环境图这个像素有多亮”，而是一个真正用于积分与 MIS 的方向域 PDF。

### 15.8 `previousBsdfPdf` 是什么

`previousBsdfPdf` 表示：

- 上一跳从 BSDF 中采样出当前这条反弹方向时，BSDF 分布给出的 PDF。

也就是说，如果当前路径是：

- 上一跳在表面上用 `sampleBSDF(...)` 选出了一个新方向；
- 然后这个方向没有再命中物体，而是看到了环境；

那么 `previousBsdfPdf` 就是：

> “BSDF 生成这个方向的概率密度”

MIS 需要同时知道两条采样路径的概率：

- BSDF 是怎么走到这里的；
- light sampling 又有多大概率会走到这里。

### 15.9 `powerHeuristic(previousBsdfPdf, lightPdf)` 在做什么

这一步是在算 MIS 权重。

当前实现使用的是 power heuristic：

$$
w = \frac{p_{bsdf}^2}{p_{bsdf}^2 + p_{light}^2}
$$

也就是说：

- 如果这个方向更像是 BSDF 容易采到的方向，那么权重偏向 BSDF 路径；
- 如果这个方向更像是环境采样更容易采到的方向，那么 BSDF 这一路的权重会被压低。

这样做的目的不是“修正数值大小”，而是：

- 防止两条采样策略对同一贡献重复高估；
- 同时显著降低方差。

### 15.10 为什么 miss 环境时需要 MIS

假设上一跳是 rough 金属或 rough 玻璃：

- 一方面，你可以在命中点显式采样环境光；
- 另一方面，你也可以从 BSDF 采样一个方向，然后这条路径刚好看到环境。

这两种方式都能贡献同一份“环境光照到当前表面”的能量。

如果不做 MIS：

- 会造成同一类贡献被重复、且以高方差形式估计；
- 画面会更噪。

如果做了 MIS：

- 两种采样策略会自动按各自擅长的方向分工；
- 环境热点、粗糙高光、天空反射等都会更稳定。

### 15.11 `break` 为什么在这里

当路径 miss 后：

- 当前光线已经离开场景；
- 不会再命中任何新的表面；
- 这条路径的终点贡献已经确定。

因此这里必须终止本条路径：

```glsl
break;
```

这与“命中 emissive 几何后也会 `break`”是同一种终止逻辑：

- 路径已经到达一个可直接输出辐射亮度的终点。

### 15.12 用一句话总结这段代码

这段 miss 分支的本质是：

> “如果路径最终看到了环境光，就把当前路径吞吐量乘上环境辐射亮度累积进结果；若这条路径来自非 delta 的 BSDF 采样，则再与环境光显式采样那一路做 MIS 加权。”

这一步正是环境光在路径追踪器中既作为“背景图”，又作为“真正光源”统一进入积分框架的关键连接点。
