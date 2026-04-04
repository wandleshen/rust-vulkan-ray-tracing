use glam::{Vec3, Vec4, vec3};

pub const ENV_MAP_WIDTH: usize = 512;
pub const ENV_MAP_HEIGHT: usize = 256;

pub struct EnvironmentMapData {
    pub texels: Vec<Vec4>,
    pub pmf: Vec<f32>,
    pub conditional_cdf: Vec<f32>,
    pub marginal_cdf: Vec<f32>,
    pub total_weight: f32,
    pub average_luminance: f32,
}

fn luminance(color: Vec3) -> f32 {
    0.2126 * color.x + 0.7152 * color.y + 0.0722 * color.z
}

fn uv_to_direction(u: f32, v: f32) -> Vec3 {
    let phi = u * std::f32::consts::TAU - std::f32::consts::PI;
    let theta = v * std::f32::consts::PI;
    let sin_theta = theta.sin();

    vec3(sin_theta * phi.cos(), theta.cos(), sin_theta * phi.sin()).normalize()
}

fn generated_environment_radiance(direction: Vec3) -> Vec3 {
    let hemisphere_t = ((direction.y + 1.0) * 0.5).clamp(0.0, 1.0);

    let nadir = vec3(0.18, 0.22, 0.32);
    let horizon = vec3(0.78, 0.84, 0.96);
    let zenith = vec3(0.10, 0.22, 0.62);
    let base = if hemisphere_t < 0.5 {
        nadir.lerp(horizon, hemisphere_t * 2.0)
    } else {
        horizon.lerp(zenith, ((hemisphere_t - 0.5) * 2.0).powf(0.35))
    };

    let sun_direction = vec3(0.35, 0.88, 0.25).normalize();
    let sun_cosine = direction.dot(sun_direction).max(0.0);
    let sun_disk = vec3(32.0, 28.0, 22.0) * sun_cosine.powf(2200.0);
    let sun_glow = vec3(6.0, 4.8, 3.8) * sun_cosine.powf(140.0);

    let window_axis_a = vec3(-0.65, 0.55, -0.52).normalize();
    let window_axis_b = vec3(0.72, 0.48, -0.32).normalize();
    let window_a = vec3(10.0, 10.4, 11.5) * direction.dot(window_axis_a).max(0.0).powf(320.0);
    let window_b = vec3(8.5, 8.8, 9.6) * direction.dot(window_axis_b).max(0.0).powf(280.0);

    let rim = vec3(0.9, 0.95, 1.0) * (1.0 - direction.y.abs()).powf(6.0) * 0.35;

    base * 1.1 + sun_disk + sun_glow + window_a + window_b + rim
}

pub fn generate_environment_map() -> EnvironmentMapData {
    let mut texels = Vec::with_capacity(ENV_MAP_WIDTH * ENV_MAP_HEIGHT);
    let mut weights = vec![0.0f32; ENV_MAP_WIDTH * ENV_MAP_HEIGHT];
    let mut conditional_cdf = vec![0.0f32; ENV_MAP_WIDTH * ENV_MAP_HEIGHT];
    let mut marginal_cdf = vec![0.0f32; ENV_MAP_HEIGHT];
    let mut row_sums = vec![0.0f32; ENV_MAP_HEIGHT];

    let mut total_luminance = 0.0f32;

    for y in 0..ENV_MAP_HEIGHT {
        let v = (y as f32 + 0.5) / ENV_MAP_HEIGHT as f32;
        let theta = v * std::f32::consts::PI;
        let sin_theta = theta.sin().max(1.0e-4);

        for x in 0..ENV_MAP_WIDTH {
            let u = (x as f32 + 0.5) / ENV_MAP_WIDTH as f32;
            let direction = uv_to_direction(u, v);
            let color = generated_environment_radiance(direction);
            let idx = y * ENV_MAP_WIDTH + x;

            texels.push(color.extend(1.0));

            let texel_luminance = luminance(color);
            total_luminance += texel_luminance;

            let weight = texel_luminance * sin_theta;
            weights[idx] = weight;
            row_sums[y] += weight;
        }
    }

    let total_weight = row_sums.iter().copied().sum::<f32>().max(1.0e-8);
    let average_luminance = total_luminance / (ENV_MAP_WIDTH * ENV_MAP_HEIGHT) as f32;

    let mut row_accum = 0.0f32;
    for y in 0..ENV_MAP_HEIGHT {
        let row_sum = row_sums[y];
        let mut row_running = 0.0f32;

        for x in 0..ENV_MAP_WIDTH {
            let idx = y * ENV_MAP_WIDTH + x;
            row_running += weights[idx];
            conditional_cdf[idx] = if row_sum > 0.0 {
                (row_running / row_sum).min(1.0)
            } else {
                (x + 1) as f32 / ENV_MAP_WIDTH as f32
            };
        }

        row_accum += row_sum;
        marginal_cdf[y] = (row_accum / total_weight).min(1.0);
    }

    let pmf = weights
        .into_iter()
        .map(|weight| weight / total_weight)
        .collect::<Vec<_>>();

    EnvironmentMapData {
        texels,
        pmf,
        conditional_cdf,
        marginal_cdf,
        total_weight,
        average_luminance,
    }
}
