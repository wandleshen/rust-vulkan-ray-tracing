# 时空降噪实现详解

本文档只描述当前仓库中已经实现的时空降噪链路：它在每一帧里做什么、每一步依赖哪些数据、采用什么公式、对应哪些代码位置。本文档不再包含任何规划、目标、路线或实施步骤。

核心代码位置：

- `src/main.rs`
- `src/denoise.rs`
- `shaders/raygen.rgen.glsl`
- `shaders/denoise.comp.glsl`
- `src/windowed.rs`

---

## 1. 每一帧的整体执行顺序

当前实现中，一帧的图形处理器执行顺序如下：

```text
光线追踪阶段
    -> 输出当前帧 1 样本/像素的噪声辐亮度
    -> 输出当前帧首次命中世界坐标
    -> 输出当前帧首次命中法线与粗糙度

光线追踪到计算阶段的同步屏障

时域计算阶段
    -> 当前世界点重投影到上一帧
    -> 历史有效性判断
    -> 历史颜色裁剪
    -> 时域累积
    -> 亮度矩与方差更新

空间滤波阶段
    -> 一轮边缘感知 `a-trous` 滤波

历史回灌阶段
    -> 时域输出复制到颜色历史
    -> 当前帧 position / normal / moments 复制到历史辅助缓存

显示阶段
    -> 将 `render_target` 画到 `swapchain`
```

对应的命令缓冲位置：

- `src/main.rs:822`
  - `cmd_trace_rays(...)`，执行光线追踪阶段
- `src/main.rs:857`
  - 光线追踪写入结果到计算读取结果之间的图像同步屏障
- `src/main.rs:894`
  - 时域计算阶段的 `cmd_dispatch(...)`
- `src/main.rs:929`
  - `a-trous` 迭代循环开始
- `src/main.rs:944`
  - 空间滤波阶段的 `cmd_dispatch(...)`
- `src/main.rs:1017`
  - 历史回灌开始
- `src/main.rs:1059`
  - `render_to_swapchain(...)`
- `src/windowed.rs:519`
  - 真正的 Vulkan 图形渲染通道从这里开始

这里有一个非常关键的点：

- 光线追踪阶段和两个计算阶段都不在 Vulkan graphics render pass 里面；
- graphics render pass 只在最后把 `render_target` 显示到 `swapchain` 时才开始；
- 因此，当前渲染链路本质上是“光线追踪 + 计算后处理 + 最后一层极薄的显示通道”。

---

## 2. 输入、输出与描述符绑定

降噪计算着色器的描述符布局定义在 `src/denoise.rs:33`。当前绑定如下：

| 绑定 | 资源 | 作用 |
|---|---|---|
| 0 | `currentNoisyImage` | 当前帧 1 样本/像素的噪声辐亮度 |
| 1 | `previousColorImage` | 上一帧时域输出的颜色历史 |
| 2 | `previousPositionImage` | 上一帧首次命中的世界坐标 |
| 3 | `previousNormalRoughnessImage` | 上一帧首次命中的法线与粗糙度 |
| 4 | `currentPositionImage` | 当前帧首次命中的世界坐标 |
| 5 | `currentNormalRoughnessImage` | 当前帧首次命中的法线与粗糙度 |
| 6 | `previousMomentsImage` | 上一帧亮度一阶矩、二阶矩与方差 |
| 7 | `currentMomentsImage` | 当前帧更新后的亮度一阶矩、二阶矩与方差 |
| 8 | `filterPingImage` | 时域输出，以及空间滤波阶段的输入之一 |
| 9 | `filterPongImage` | 空间滤波阶段的另一侧 ping-pong 缓冲 |
| 10 | 当前帧相机缓冲 | 当前帧相机参数 |
| 11 | 上一帧相机缓冲 | 上一帧相机参数 |

这些图像的创建位置在 `src/main.rs`：

- `src/main.rs:244`：`current_noisy_image`
- `src/main.rs:251`：`previous_color_image`
- `src/main.rs:258`：`previous_position_image`
- `src/main.rs:265`：`previous_normal_roughness_image`
- `src/main.rs:272`：`previous_moments_image`
- `src/main.rs:279`：`current_position_image`
- `src/main.rs:286`：`current_normal_roughness_image`
- `src/main.rs:293`：`current_moments_image`
- `src/main.rs:300`：`denoise_ping_image`

当前实现中若干 `.w` / `.a` 分量还承担了元数据作用：

- `currentPositionImage.w > 0.5`
  - 当前像素有有效首次命中
- `previousPositionImage.w > 0.5`
  - 历史像素有有效首次命中
- `filterPingImage.a`
  - 历史长度
- `currentMomentsImage.z`
  - 当前像素的亮度方差估计
- `previousFrameData.origin.w > 0.5`
  - 上一帧相机数据可用

因此当前降噪不只是复用历史颜色，而是同时复用：

- 历史颜色；
- 历史几何锚点；
- 历史统计量；
- 当前帧局部噪声估计。

---

## 3. 为什么需要首次命中的几何缓冲

当前时域降噪的核心锚点不是运动向量，而是 **首次命中的世界坐标**。

`raygen` 在路径追踪时除了写当前像素的噪声辐亮度，还会写出：

- 首次命中的世界坐标；
- 首次命中的法线；
- 首次命中的粗糙度。

这三类数据分别服务于：

1. **世界坐标**
   - 把当前像素重投影到上一帧；
2. **法线**
   - 验证历史像素与当前像素是否仍然落在同一块表面上；
3. **粗糙度**
   - 调节历史长度上限、历史验证阈值与空间滤波强度。

这也是为什么当前方案在漫反射表面上通常比在镜面反射表面上稳定：

- 漫反射的出射辐亮度变化更依赖局部表面；
- 镜面反射的出射辐亮度对视角非常敏感；
- 即使首次命中表面没变，镜面方向看到的环境和物体也可能已经变化很多。

---

## 4. 世界坐标如何投影到上一帧

### 4.1 相机参数化方式

当前计算着色器没有使用标准的 4x4 投影矩阵，而是直接沿用 `raygen` 相机使用的平面参数：

- `origin`
- `lowerLeftCorner`
- `horizontal`
- `vertical`

它们分别表示：

- 相机原点；
- 成像平面左下角；
- 成像平面横向基向量；
- 成像平面纵向基向量。

相关代码位置：

- `shaders/denoise.comp.glsl:21`
- `shaders/denoise.comp.glsl:30`
- `shaders/denoise.comp.glsl:57`
- `shaders/denoise.comp.glsl:66`
- `shaders/denoise.comp.glsl:75`

### 4.2 成像平面中心与前向方向

成像平面中心为：

\[
\mathbf{c} = \mathbf{ll} + 0.5\,\mathbf{h} + 0.5\,\mathbf{v}
\]

其中：

- `ll = lowerLeftCorner.xyz`
- `h = horizontal.xyz`
- `v = vertical.xyz`

相机前向方向为：

\[
\mathbf{f} = \frac{\mathbf{c} - \mathbf{o}}{\|\mathbf{c} - \mathbf{o}\|}
\]

焦平面距离为：

\[
D_f = \|\mathbf{c} - \mathbf{o}\|
\]

对应实现：

- `frameCenter(...)`：`shaders/denoise.comp.glsl:57`
- `frameForward(...)`：`shaders/denoise.comp.glsl:66`

### 4.3 世界点投影到某一帧 UV 的公式

给定当前像素的首次命中世界坐标 `p`，首先计算相对相机原点的向量：

\[
\mathbf{r} = \mathbf{p} - \mathbf{o}
\]

再将其沿相机前向方向投影，得到深度：

\[
z = \mathbf{r} \cdot \mathbf{f}
\]

如果 `z <= \varepsilon`，则认为该点位于相机背后，投影失败。

若投影有效，则将该点缩放到焦平面上：

\[
\mathbf{p}_{proj} = \mathbf{o} + \mathbf{r} \cdot \frac{D_f}{z}
\]

然后相对成像平面左下角求偏移：

\[
\mathbf{q} = \mathbf{p}_{proj} - \mathbf{ll}
\]

最后沿 `horizontal` 和 `vertical` 投影得到 UV：

\[
u = \frac{\mathbf{q} \cdot \mathbf{h}}{\mathbf{h} \cdot \mathbf{h}}
\]

\[
v = \frac{\mathbf{q} \cdot \mathbf{v}}{\mathbf{v} \cdot \mathbf{v}}
\]

只有当：

```text
0 <= u <= 1
0 <= v <= 1
```

时，这个 UV 才被认为落在该帧屏幕范围内。

对应函数：

- `projectWorldPositionToFrameUv(...)`：`shaders/denoise.comp.glsl:75`

### 4.4 为什么要投影到上一帧

时域累积要回答的问题是：

- 当前像素代表的是哪个世界点？
- 这个世界点在上一帧屏幕上落在哪个像素？
- 那个像素保存了什么颜色历史、几何历史和统计历史？

因此当前重投影的数据流就是：

```text
当前像素的首次命中世界坐标
    -> 用上一帧相机做投影
    -> 得到上一帧 UV
    -> 得到上一帧像素坐标
    -> 读取 previousColor / previousPosition / previousNormal / previousMoments
```

对应代码：

- `reprojectWorldPositionToPreviousUv(...)`：`shaders/denoise.comp.glsl:104`
- `uvToImageCoordinate(...)`：`shaders/denoise.comp.glsl:143`
- 调用位置：`shaders/denoise.comp.glsl:225`

---

## 5. 历史有效性判断

时域阶段的主体函数是 `runTemporalPass(...)`，位于 `shaders/denoise.comp.glsl:205`。

### 5.1 当前像素是否有有效几何锚点

首先读取当前首次命中：

```glsl
vec4 currentPositionData = imageLoad(currentPositionImage, pixelCoord);
bool currentValid = currentPositionData.w > 0.5;
```

如果当前像素没有有效首次命中，那么它就没有稳定的几何锚点，时域历史也就无法可靠复用。

### 5.2 上一帧历史是否可用

当前实现还会检查：

```glsl
previousFrameData.origin.w > 0.5
```

这相当于一个“上一帧相机数据和历史缓存已经初始化”的标志，防止刚重置时去读无效历史。

### 5.3 位置一致性

重投影到 `previousCoord` 之后，会比较历史位置 `p_prev` 与当前位置 `p_curr`：

\[
E_p = \|\mathbf{p}_{prev} - \mathbf{p}_{curr}\|
\]

当前实现的位置容差不是固定常数，而是随视距和粗糙度变化：

\[
T_p = \max(0.02, \operatorname{mix}(0.01, 0.03, r) \cdot \max(d_{view}, 1.0))
\]

其中：

- `r` 是当前像素粗糙度；
- `d_{view} = \|p_{curr} - o_{camera}\|`。

对应代码：

```glsl
float viewDistance = length(currentPositionData.xyz - frameData.origin.xyz);
float positionTolerance = max(0.02, mix(0.01, 0.03, roughness) * max(viewDistance, 1.0));
float positionError = length(previousPositionData.xyz - currentPositionData.xyz);
```

这样设计的含义是：

- 越近的表面要求越严格；
- 越远的表面允许略大的投影误差；
- 越粗糙的表面允许略大的几何误差。

### 5.4 法线一致性

还会比较当前法线与历史法线的夹角一致性：

\[
A_n = \mathbf{n}_{prev} \cdot \mathbf{n}_{curr}
\]

阈值同样由粗糙度控制：

\[
T_n = \operatorname{mix}(0.98, 0.85, r)
\]

对应代码：

```glsl
float normalThreshold = mix(0.98, 0.85, roughness);
float normalAlignment = dot(previousNormal, currentNormal);
```

历史通过验证的条件为：

```text
positionError <= positionTolerance
并且
normalAlignment >= normalThreshold
```

### 5.5 为什么镜面反射仍然更难稳定

这种判断本质上只是验证“是否还是同一块表面”，而不是验证“是否还是同一条光路”。

对于漫反射，这通常已经足够有用；但对于镜面反射或高光反射：

- 首次命中仍然可能是同一个金属球表面；
- 但该表面反射到的环境热点已经变化很多；
- 所以几何上有效的历史，辐射上却未必仍然相似。

这就是为什么当前实现中，镜面反射部分天然比漫反射部分更容易闪烁。

---

## 6. 当前帧邻域统计与历史颜色裁剪

### 6.1 为什么需要邻域统计

即使位置和法线都通过验证，直接混合历史颜色仍然很危险，因为可能出现：

- 鬼影；
- 拖尾；
- 单个亮点被时间维度不断放大；
- 一会儿变亮、一会儿变暗的时域抽动。

因此当前实现会先在当前帧噪声图像上做一个 `3x3` 邻域统计，再用这个统计量去限制历史颜色的取值范围。

对应函数：

- `computeNeighborhoodStats(...)`：`shaders/denoise.comp.glsl:148`

### 6.2 RGB 均值与标准差

对于邻域中的每个颜色样本 `c_i`，计算：

\[
\mu = \frac{1}{N} \sum_i c_i
\]

\[
M_2 = \frac{1}{N} \sum_i c_i^2
\]

\[
\sigma = \sqrt{\max(M_2 - \mu^2, 0)}
\]

这里：

- `\mu` 是 RGB 均值；
- `\sigma` 是逐通道标准差；
- 这里的 `M_2` 是当前邻域颜色平方均值，不是时间上的亮度二阶矩。

对应代码：

```glsl
mean += sampleColor;
secondMoment += sampleColor * sampleColor;
...
stats.mean = mean;
stats.sigma = sqrt(max(secondMoment - mean * mean, vec3(0.0)));
```

### 6.3 亮度均值与亮度方差

亮度定义为：

\[
L = 0.2126 R + 0.7152 G + 0.0722 B
\]

邻域亮度均值：

\[
\mu_L = \frac{1}{N} \sum_i L_i
\]

邻域亮度方差：

\[
\sigma_L^2 = \max\left(\frac{1}{N}\sum_i L_i^2 - \mu_L^2, 0\right)
\]

对应代码：

```glsl
float sampleLuma = luminance(sampleColor);
lumaMean += sampleLuma;
lumaSecondMoment += sampleLuma * sampleLuma;
...
stats.lumaVariance = max(lumaSecondMoment - lumaMean * lumaMean, 0.0);
```

这个亮度方差后面会直接用于：

- 作为时域方差的下界来源；
- 控制空间滤波阶段的颜色权重与滤波强度。

### 6.4 历史颜色裁剪公式

历史颜色不会直接参与时域混合，而是先被裁剪到当前邻域的合理范围内。

当前实现中：

\[
C_{min} = \mu - 2.5\sigma - b
\]

\[
C_{max} = \mu + 2.5\sigma + b
\]

其中偏置项为：

```glsl
vec3 clampBias = vec3(0.05);
```

然后执行：

\[
C_{hist}^{clamped} = \operatorname{clamp}(C_{hist}, C_{min}, C_{max})
\]

对应代码：

```glsl
vec3 clampMin = stats.mean - 2.5 * stats.sigma - clampBias;
vec3 clampMax = stats.mean + 2.5 * stats.sigma + clampBias;
vec3 clampedHistory = clamp(historyData.rgb, clampMin, clampMax);
```

它的意义是：

- 如果历史值远远偏离当前邻域分布，就把它拉回一个合理范围；
- 避免火花噪点或错误历史不断继承到后续帧；
- 让时域累积更保守。

---

## 7. 时域累积的计算

### 7.1 相机静止度

当前实现不是固定历史长度，而是先估计“相机当前有多静止”。

平移变化：

\[
\Delta_t = \|o_{curr} - o_{prev}\|
\]

朝向变化用前向向量点乘近似：

\[
\Delta_r = 1 - \operatorname{clamp}(f_{curr} \cdot f_{prev}, 0, 1)
\]

再组合成相机静止度：

\[
S = 1 - \operatorname{clamp}(4\Delta_t + 400\Delta_r, 0, 1)
\]

对应实现：

- `temporalCameraStillness()`：`shaders/denoise.comp.glsl:115`

这个值越接近 `1`，说明相机越静止，当前帧就越允许积累更长历史。

### 7.2 最大历史长度

最大历史长度同时依赖粗糙度与相机静止度：

\[
H_{max} = \operatorname{clamp}(\operatorname{mix}(12, 48, r) \cdot \operatorname{mix}(0.5, 1.5, S), 2, 64)
\]

对应实现：

- `temporalMaxHistory(...)`：`shaders/denoise.comp.glsl:133`

它的含义是：

- 表面越粗糙，越允许积累更长历史；
- 相机越静止，越允许积累更长历史；
- 历史长度最终被夹在 `[2, 64]` 之间。

### 7.3 历史长度与时域混合权重

如果历史通过验证，则：

\[
H_t = \min(H_{prev} + 1, H_{max})
\]

再由历史长度构造混合权重：

\[
w_h = \frac{H_{prev}}{H_t}
\]

最后执行时域混合：

\[
C_t = (1 - w_h) C_{current} + w_h C_{hist}^{clamped}
\]

对应代码：

```glsl
float previousHistoryLength = clamp(historyData.a, 1.0, maxHistory - 1.0);
historyLength = min(previousHistoryLength + 1.0, maxHistory);
float historyWeight = previousHistoryLength / historyLength;
temporalColor = mix(currentColor, clampedHistory, historyWeight);
```

这里的 `historyData.a` 存的就是历史长度，它和颜色一起保存在 `previousColorImage` 中。

### 7.4 为什么不是无限平均

当前时域累积并不是“从第 1 帧一直平均到当前帧”的无限运行平均，而是一个：

- 有历史上限；
- 有历史验证；
- 有历史裁剪；
- 受相机运动和粗糙度共同调节的有限平均。

这样做是因为：

- 无限平均虽然更稳，但会严重拖尾；
- 限制 `H_max` 可以在稳定与响应之间折中；
- 粗糙表面和静止相机的确更适合长期积累；
- 尖锐镜面和运动相机必须更保守。

---

## 8. 亮度矩与方差的计算

### 8.1 为什么只存亮度统计量

当前实现没有为 RGB 分别存完整协方差，而是只存亮度的一阶矩、二阶矩与由此恢复的方差。

这样做的好处是：

- 成本低；
- 实现简单；
- 对空间滤波强度控制已经足够有用。

### 8.2 当前帧亮度

当前帧颜色亮度为：

\[
L_t = \operatorname{luminance}(C_{current})
\]

若没有历史可用，则初始化为：

\[
M_{1,t} = L_t
\]

\[
M_{2,t} = L_t^2
\]

对应初始化代码：

```glsl
float currentLuma = luminance(currentColor);
float filteredM1 = currentLuma;
float filteredM2 = currentLuma * currentLuma;
float filteredVariance = max(stats.lumaVariance, 1e-6);
```

### 8.3 有效历史下的一阶矩与二阶矩更新

如果历史有效，则亮度矩按和颜色相同的时域权重进行更新：

\[
M_{1,t} = (1 - w_h)L_t + w_h M_{1,prev}
\]

\[
M_{2,t} = (1 - w_h)L_t^2 + w_h M_{2,prev}
\]

对应代码：

```glsl
filteredM1 = mix(currentLuma, previousMoments.x, historyWeight);
filteredM2 = mix(currentLuma * currentLuma, previousMoments.y, historyWeight);
```

其中：

- `previousMoments.x` 是上一帧的一阶矩；
- `previousMoments.y` 是上一帧的二阶矩。

### 8.4 方差恢复与方差下界

亮度方差通过标准关系恢复：

\[
\sigma_t^2 = M_{2,t} - M_{1,t}^2
\]

但当前实现没有直接把它作为最终方差，而是又与当前邻域亮度方差做了一个下界比较：

\[
variance_t = \max(M_{2,t} - M_{1,t}^2, 0.25 \cdot variance_{neighborhood})
\]

对应代码：

```glsl
filteredVariance = max(filteredM2 - filteredM1 * filteredM1, stats.lumaVariance * 0.25);
```

这样做的原因是：

- 时域累积有时会过于自信；
- 如果 `M_2 - M_1^2` 被压得太低，空间滤波阶段会误以为该像素已经收敛；
- 用当前邻域方差乘一个系数作为下界，可以保留一定的“噪声警觉性”。

### 8.5 时域阶段的输出

最终时域阶段写出两类结果：

```glsl
imageStore(filterPingImage, pixelCoord, vec4(temporalColor, historyLength));
imageStore(currentMomentsImage, pixelCoord, vec4(filteredM1, filteredM2, filteredVariance, 1.0));
```

因此显式输出包括：

- 时域颜色；
- 历史长度；
- 亮度一阶矩；
- 亮度二阶矩；
- 亮度方差。

---

## 9. `a-trous` 空间滤波

当前空间滤波的入口函数为：

- `runAtrousPass(...)`：`shaders/denoise.comp.glsl:264`

虽然它叫 `a-trous`，但当前配置只启用了一轮，因此更准确地说，它是“一轮带边缘感知权重的小波式空间滤波”。

### 9.1 ping-pong 读写

空间滤波阶段通过 `inputIsPing` 决定当前从 `filterPingImage` 读还是从 `filterPongImage` 读：

- `loadAtrousInput(...)`：`shaders/denoise.comp.glsl:191`
- `storeAtrousOutput(...)`：`shaders/denoise.comp.glsl:197`

这样做的原因是：

- 同一轮不能边读边覆盖自己的输入；
- 如果后续开启多轮滤波，必须使用 ping-pong。

### 9.2 步长

第 `k` 轮 `a-trous` 的步长定义为：

\[
stepWidth = 2^k
\]

CPU 侧通过 push constant 写入：

- `src/main.rs:931`
- `src/main.rs:933`

也就是：

```rust
step_width: 1u32 << iteration
```

当前项目中：

- `src/main.rs:24`
  - `const DENOISE_ATROUS_PASSES: u32 = 1;`

因此现在只启用了一轮空间滤波。

### 9.3 基础卷积核

当前使用的是归一化后的 `1 4 6 4 1` 核：

\[
\left[\frac{1}{16}, \frac{1}{4}, \frac{3}{8}, \frac{1}{4}, \frac{1}{16}\right]
\]

实现函数：

- `kernelWeight(...)`：`shaders/denoise.comp.glsl:181`

二维基础权重为：

\[
w_{base}(x, y) = k(x) \cdot k(y)
\]

对应代码：

```glsl
float weight = kernelWeight(x + 2) * kernelWeight(y + 2);
```

### 9.4 深度权重

如果中心像素和邻域像素都有效，则会计算沿中心法线方向的深度差：

\[
\Delta_d = |(p_s - p_c) \cdot n_c|
\]

对应代码：

```glsl
abs(dot(samplePositionData.xyz - centerPositionData.xyz, centerNormal))
```

深度 sigma 为：

\[
\sigma_d = \max(0.01, \operatorname{mix}(0.01, 0.10, r) \cdot \max(d_{view}, 1.0))
\]

深度权重为：

\[
w_d = \exp\left(-\frac{\Delta_d}{\sigma_d}\right)
\]

对应代码：

```glsl
float depthSigma = max(0.01, mix(0.01, 0.10, roughness) * max(viewDistance, 1.0));
float depthWeight = exp(
    -abs(dot(samplePositionData.xyz - centerPositionData.xyz, centerNormal))
    / depthSigma
);
```

这个权重保证：

- 深度不连续处不会被强行糊在一起；
- 前景与背景边界更容易被保留下来。

### 9.5 法线权重

法线权重使用中心法线与邻域法线的点乘，并由粗糙度调节指数：

\[
E_n = \max(n_c \cdot n_s, 0)
\]

\[
\gamma_n = \operatorname{mix}(256, 24, r)
\]

\[
w_n = E_n^{\gamma_n}
\]

对应代码：

```glsl
float normalExponent = mix(256.0, 24.0, roughness);
float normalWeight = pow(max(dot(centerNormal, sampleNormal), 0.0), normalExponent);
```

它的意义是：

- 粗糙度小时，法线权重更严格；
- 粗糙度大时，法线权重会适度放宽。

### 9.6 颜色权重

颜色项使用亮度差，而不是 RGB 欧氏距离：

\[
\Delta_L = |L_s - L_c|
\]

对应代码：

```glsl
float colorDelta = abs(luminance(sampleData.rgb) - luminance(centerColor));
```

颜色 sigma 由当前像素方差和粗糙度共同控制：

\[
\sigma_c = \max(0.01, \operatorname{mix}(0.015, 1.5\sqrt{variance_c} + 0.01, roughnessFilterStrength))
\]

颜色权重为：

\[
w_c = \exp\left(-\frac{\Delta_L}{\sigma_c}\right)
\]

对应代码：

```glsl
float colorSigma = max(
    0.01,
    mix(0.015, 1.5 * sqrt(centerVariance) + 0.01, roughnessFilterStrength)
);
float colorWeight = exp(-colorDelta / colorSigma);
```

方差越大，`colorSigma` 越大，意味着当前像素噪声越大，空间滤波就会更愿意跨亮度差去做平滑。

### 9.7 单个样本的总权重

因此一个邻域样本最终的权重为：

\[
w_i = w_{base} \cdot w_d \cdot w_n \cdot w_c
\]

对应代码中就是对 `weight` 连续累乘：

```glsl
weight *= depthWeight * normalWeight;
...
weight *= colorWeight;
```

### 9.8 滤波强度

当前实现不会无条件用 `filteredColor` 覆盖中心像素，而是根据粗糙度、历史长度和方差再计算一个更保守的滤波强度：

```glsl
float roughnessFilterStrength = smoothstep(0.18, 0.50, roughness);
float historyFilterStrength = smoothstep(3.0, 10.0, centerHistoryLength);
float varianceFilterStrength = smoothstep(0.002, 0.02, centerVariance);
float filterStrength = 0.45
    * roughnessFilterStrength
    * mix(0.35, 1.0, historyFilterStrength)
    * mix(0.5, 1.0, varianceFilterStrength);
```

这意味着：

- 粗糙度小，滤波更弱；
- 历史长度短，滤波更弱；
- 方差小，滤波更弱；
- 粗糙、历史较长且方差仍较大时，滤波才更积极。

### 9.9 空间滤波阶段的输出

首先做归一化加权平均：

\[
C_{filtered} = \frac{\sum_i w_i C_i}{\sum_i w_i}
\]

然后和中心颜色做一次线性混合：

\[
C_{final} = (1 - s) C_{center} + s C_{filtered}
\]

其中 `s = filterStrength`。

对应代码：

```glsl
vec3 filteredColor = accumulatedWeight > EPSILON
    ? accumulatedColor / accumulatedWeight
    : centerColor;
vec3 finalColor = mix(centerColor, filteredColor, filterStrength);
```

这一层混合很重要，因为它避免了空间滤波过强时把所有细节一起抹掉。

---

## 10. 为什么历史回灌的是时域输出

当前每帧结束后，会执行：

- `denoise_ping_image -> previous_color_image`
- `current_position_image -> previous_position_image`
- `current_normal_roughness_image -> previous_normal_roughness_image`
- `current_moments_image -> previous_moments_image`

代码位置：

- `src/main.rs:1017`
- `src/main.rs:1026`
- `src/main.rs:1035`
- `src/main.rs:1044`

这里有一个关键设计：

- 写回历史的是 **时域输出**；
- 不是最终 `render_target` 的空间滤波输出。

这样做的原因是：

- 空间滤波输出已经被邻域平滑过；
- 如果再把它继续当成下一帧历史，就会把空间模糊继续反馈到时间维度里；
- 长时间运行后会变成“历史越来越糊、细节越来越难回来”。

因此当前实现选择：

- 时域输出进入历史；
- 空间滤波输出只负责当前帧显示。

---

## 11. 一帧内完整数据流

把所有阶段串起来，当前一帧内的完整数据流可以写成：

```text
raygen
    -> currentNoisyImage
    -> currentPositionImage
    -> currentNormalRoughnessImage

runTemporalPass
    -> 读取 currentNoisy / previousColor / previousPosition / previousNormal / previousMoments
    -> 输出 filterPingImage
    -> 输出 currentMomentsImage

runAtrousPass
    -> 读取 filterPingImage
    -> 读取 currentPositionImage / currentNormalRoughnessImage / currentMomentsImage
    -> 输出 render_target

历史回灌
    -> filterPingImage 复制到 previousColorImage
    -> 当前 position / normal / moments 复制到各自 previous 图像

显示
    -> render_target 送入 swapchain
```

因此当前仓库已经具备一个完整的：

```text
光线追踪 -> 时域计算 -> 空间滤波 -> 显示
```

实时降噪链路。

---

## 12. 代码索引

适合按阅读顺序查看的入口如下：

- `src/main.rs:24`
  - `DENOISE_ATROUS_PASSES`
- `src/main.rs:244`
  - 降噪相关图像分配
- `src/main.rs:499`
  - 降噪描述符与管线初始化
- `src/main.rs:709`
  - 历史清空与重置
- `src/main.rs:822`
  - 光线追踪调度
- `src/main.rs:857`
  - 光线追踪到计算阶段的同步屏障
- `src/main.rs:881`
  - 时域阶段 push constants
- `src/main.rs:894`
  - 时域阶段计算调度
- `src/main.rs:901`
  - 时域阶段输出后的同步屏障
- `src/main.rs:929`
  - `a-trous` 循环
- `src/main.rs:944`
  - 空间滤波阶段计算调度
- `src/main.rs:1017`
  - 历史回灌
- `src/main.rs:1059`
  - `render_to_swapchain(...)`
- `src/windowed.rs:463`
  - `render_to_swapchain(...)` 实现
- `src/windowed.rs:519`
  - 图形渲染通道开始
- `src/denoise.rs:33`
  - 描述符布局
- `src/denoise.rs:108`
  - 描述符更新
- `src/denoise.rs:230`
  - 计算管线创建
- `shaders/denoise.comp.glsl:75`
  - 世界点投影到某一帧 UV
- `shaders/denoise.comp.glsl:104`
  - 重投影到上一帧
- `shaders/denoise.comp.glsl:115`
  - 相机静止度
- `shaders/denoise.comp.glsl:133`
  - 最大历史长度
- `shaders/denoise.comp.glsl:148`
  - 邻域统计
- `shaders/denoise.comp.glsl:205`
  - 时域阶段
- `shaders/denoise.comp.glsl:264`
  - `a-trous` 空间滤波阶段

---

## 13. 当前实现最核心的七个设计点

1. **重投影锚点是首次命中的世界坐标，而不是运动向量。**
2. **历史是否可用由屏幕范围、位置一致性和法线一致性共同决定。**
3. **历史颜色在参与时域混合之前，必须先经过当前帧邻域裁剪。**
4. **时域累积使用历史长度驱动的动态权重，而不是无限平均。**
5. **亮度方差来自亮度一阶矩、二阶矩，并保留当前邻域方差作为下界。**
6. **空间滤波是一轮边缘感知 `a-trous`，其强度由粗糙度、历史长度和方差共同决定。**
7. **写回历史的是时域输出，而不是最终空间滤波输出。**

这七点基本就概括了当前仓库里时空降噪链路的核心思想。
