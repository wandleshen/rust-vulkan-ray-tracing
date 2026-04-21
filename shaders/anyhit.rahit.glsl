#version 460
#extension GL_EXT_ray_tracing : require

struct MaterialData {
    vec4 baseColor;
    vec4 emission;
    vec4 params;
    vec4 medium;
};

layout(binding = 2, set = 0) readonly buffer Materials {
    MaterialData materials[];
} matBuffer;

bool isInvisibleMaterial(MaterialData material) {
    return material.medium.w < 0.0;
}

void main() {
    MaterialData material = matBuffer.materials[gl_InstanceCustomIndexEXT];
    if (isInvisibleMaterial(material)) {
        ignoreIntersectionEXT;
    }
}


