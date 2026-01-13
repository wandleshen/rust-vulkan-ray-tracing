#version 460
#extension GL_EXT_ray_tracing : require

struct MaterialData {
    uint t;
    vec4 data;
};

layout(binding = 0, set = 0) uniform accelerationStructureEXT tlas;
layout(binding = 1, set = 0, rgba32f) uniform image2D outputImage;
layout(binding = 2, set = 0) readonly buffer Materials {
    MaterialData materials[];
} matBuffer;

layout(location = 0) rayPayloadEXT RayPayload {
    uint isMiss;
    vec3 position;
    vec3 normal;
    uint material;
    uint frontFace;
} payload;

layout(push_constant) uniform PushConstants {
    uint seed;
} constants;

struct Ray {
    vec3 origin;
    vec3 direction;
};

struct Camera {
    vec3 origin;
    vec3 lowerLeftCorner;
    vec3 horizontal;
    vec3 vertical;
    vec3 u;
    vec3 v;
    float lensRadius;
};

// 随机数生成器
uint pcgHash(uint seed) {
    uint state = seed * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

float randomFloat(inout uint seed) {
    seed = pcgHash(seed);
    return float(seed) / float(0xffffffffu);
}

float randomFloatRange(inout uint seed, float min, float max) {
    return min + (max - min) * randomFloat(seed);
}

vec3 randomVec3(inout uint seed) {
    return vec3(randomFloat(seed), randomFloat(seed), randomFloat(seed));
}

vec3 randomVec3Range(inout uint seed, float min, float max) {
    return vec3(
        randomFloatRange(seed, min, max),
        randomFloatRange(seed, min, max),
        randomFloatRange(seed, min, max)
    );
}

vec3 randomInUnitSphere(inout uint seed) {
    while (true) {
        vec3 p = randomVec3Range(seed, -1.0, 1.0);
        if (dot(p, p) < 1.0) return p;
    }
}

vec3 randomInUnitDisk(inout uint seed) {
    while (true) {
        vec3 p = vec3(
            randomFloatRange(seed, -1.0, 1.0),
            randomFloatRange(seed, -1.0, 1.0),
            0.0
        );
        if (dot(p, p) < 1.0) return p;
    }
}

bool isNearZero(vec3 v) {
    const float s = 1e-8;
    return abs(v.x) < s && abs(v.y) < s && abs(v.z) < s;
}

// Camera 函数
Camera createCamera(vec3 lookFrom, vec3 lookAt, vec3 vup, float vfov, float aspectRatio, float aperture, float focusDist) {
    Camera cam;
    float theta = vfov;
    float h = tan(theta * 0.5);
    float viewportHeight = 2.0 * h;
    float viewportWidth = aspectRatio * viewportHeight;
    
    vec3 w = normalize(lookFrom - lookAt);
    vec3 u = normalize(cross(vup, w));
    vec3 v = cross(w, u);
    
    cam.origin = lookFrom;
    cam.horizontal = focusDist * viewportWidth * u;
    cam.vertical = focusDist * viewportHeight * v;
    cam.lowerLeftCorner = cam.origin - cam.horizontal / 2.0 - cam.vertical / 2.0 - focusDist * w;
    cam.u = u;
    cam.v = v;
    cam.lensRadius = aperture / 2.0;
    
    return cam;
}

Ray getRay(Camera cam, float s, float t, inout uint seed) {
    vec3 rd = cam.lensRadius * randomInUnitDisk(seed);
    vec3 offset = cam.u * rd.x + cam.v * rd.y;
    
    Ray ray;
    ray.origin = cam.origin + offset;
    ray.direction = cam.lowerLeftCorner + s * cam.horizontal + t * cam.vertical - cam.origin - offset;
    return ray;
}

// 材质散射
vec3 customReflect(vec3 v, vec3 n) {
    return v - 2.0 * dot(v, n) * n;
}

vec3 customRefract(vec3 uv, vec3 n, float etaiOverEtat) {
    float cosTheta = min(dot(-uv, n), 1.0);
    vec3 rOutPerp = etaiOverEtat * (uv + cosTheta * n);
    vec3 rOutParallel = -sqrt(abs(1.0 - dot(rOutPerp, rOutPerp))) * n;
    return rOutPerp + rOutParallel;
}

float reflectance(float cosine, float refIdx) {
    float r0 = (1.0 - refIdx) / (1.0 + refIdx);
    r0 = r0 * r0;
    return r0 + (1.0 - r0) * pow(1.0 - cosine, 5.0);
}

struct Scatter {
    vec3 color;
    Ray ray;
    bool valid;
};

Scatter scatter(uint matType, vec4 matData, Ray ray, inout uint seed) {
    Scatter s;
    s.valid = false;
    
    vec3 hitPos = payload.position;
    vec3 normal = payload.normal;
    
    if (matType == 0) { // Lambertian
        vec3 albedo = matData.xyz;
        vec3 scatterDir = normal + normalize(randomInUnitSphere(seed));
        if (isNearZero(scatterDir)) {
            scatterDir = normal;
        }
        s.ray.origin = hitPos;
        s.ray.direction = scatterDir;
        s.color = albedo;
        s.valid = true;
    } else if (matType == 1) { // Metal
        vec3 albedo = matData.xyz;
        float fuzz = matData.w;
        vec3 reflected = customReflect(normalize(ray.direction), normal);
        s.ray.origin = hitPos;
        s.ray.direction = reflected + fuzz * randomInUnitSphere(seed);
        s.color = albedo;
        s.valid = dot(s.ray.direction, normal) > 0.0;
    } else if (matType == 2) { // Dielectric
        float ir = matData.x;
        s.color = vec3(1.0);
        float refractionRatio = payload.frontFace == 1 ? (1.0 / ir) : ir;
        vec3 unitDir = normalize(ray.direction);
        float cosTheta = min(dot(-unitDir, normal), 1.0);
        float sinTheta = sqrt(1.0 - cosTheta * cosTheta);
        bool cannotRefract = refractionRatio * sinTheta > 1.0;
        
        vec3 direction;
        if (cannotRefract || reflectance(cosTheta, refractionRatio) > randomFloat(seed)) {
            direction = customReflect(unitDir, normal);
        } else {
            direction = customRefract(unitDir, normal, refractionRatio);
        }
        
        s.ray.origin = hitPos;
        s.ray.direction = direction;
        s.valid = true;
    }
    
    return s;
}

void main() {
    uint randSeed = (gl_LaunchIDEXT.y * gl_LaunchSizeEXT.x + gl_LaunchIDEXT.x) ^ constants.seed;
    
    const float PI = 3.14159265359;
    Camera camera = createCamera(
        vec3(13.0, 2.0, 3.0),
        vec3(0.0, 0.0, 0.0),
        vec3(0.0, 1.0, 0.0),
        20.0 / 180.0 * PI,
        float(gl_LaunchSizeEXT.x) / float(gl_LaunchSizeEXT.y),
        0.1,
        10.0
    );
    
    float u = (float(gl_LaunchIDEXT.x) + randomFloat(randSeed)) / float(gl_LaunchSizeEXT.x - 1);
    float v = (float(gl_LaunchIDEXT.y) + randomFloat(randSeed)) / float(gl_LaunchSizeEXT.y - 1);
    
    Ray ray = getRay(camera, u, v, randSeed);
    vec3 color = vec3(1.0);
    
    const int maxDepth = 50;
    for (int depth = 0; depth < maxDepth; depth++) {
        traceRayEXT(
            tlas,
            gl_RayFlagsOpaqueEXT,
            0xFF,
            0, 0, 0,
            ray.origin,
            1e-4,
            ray.direction,
            1e4,
            0
        );
        
        if (payload.isMiss == 1) {
            // 天空颜色
            vec3 unitDir = normalize(ray.direction);
            float t = 0.5 * (unitDir.y + 1.0);
            vec3 skyColor = mix(vec3(1.0), vec3(0.5, 0.7, 1.0), t);
            color *= skyColor;
            break;
        }
        
        uint matIdx = payload.material;
        MaterialData mat = matBuffer.materials[matIdx];
        uint matType = mat.t;
        vec4 matData = mat.data;
        
        Scatter scattered = scatter(matType, matData, ray, randSeed);
        if (!scattered.valid) {
            color = vec3(0.0);
            break;
        }
        
        color *= scattered.color;
        ray = scattered.ray;
    }
    
    vec4 oldColor = imageLoad(outputImage, ivec2(gl_LaunchIDEXT.xy));
    vec4 newColor = vec4(color, 1.0) + oldColor;
    imageStore(outputImage, ivec2(gl_LaunchIDEXT.xy), newColor);
}
