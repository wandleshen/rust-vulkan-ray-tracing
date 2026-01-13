#version 460
#extension GL_EXT_ray_tracing : require

layout(location = 0) rayPayloadInEXT RayPayload {
    uint isMiss;
    vec3 position;
    vec3 normal;
    uint material;
    uint frontFace;
} payload;

void main() {
    payload.isMiss = 1;
}
