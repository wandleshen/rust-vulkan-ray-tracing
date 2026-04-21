#version 460
#extension GL_EXT_ray_tracing : require

layout(location = 0) rayPayloadInEXT RayPayload {
    uint isMiss;
    float distance;
    vec3 position;
    vec3 normal;
    uint material;
    uint frontFace;
} payload;

hitAttributeEXT vec3 hitAttribute;

void main() {
    float t = gl_HitTEXT;
    vec3 worldRayOrigin = gl_WorldRayOriginEXT;
    vec3 worldRayDirection = gl_WorldRayDirectionEXT;
    mat4x3 objectToWorld = gl_ObjectToWorldEXT;
    uint material = gl_InstanceCustomIndexEXT;
    
    vec3 hitPos = worldRayOrigin + t * worldRayDirection;
    vec3 center = objectToWorld[3];
    vec3 normal = normalize(hitPos - center);
    
    bool frontFace = dot(worldRayDirection, normal) < 0.0;
    normal = frontFace ? normal : -normal;
    
    payload.isMiss = 0;
    payload.distance = t;
    payload.position = hitPos;
    payload.normal = normal;
    payload.material = material;
    payload.frontFace = frontFace ? 1 : 0;
}
