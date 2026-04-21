#version 460
#extension GL_EXT_ray_tracing : require

hitAttributeEXT vec3 hitAttribute;

void main() {
    vec3 rayOrigin = gl_ObjectRayOriginEXT;
    vec3 rayDirection = gl_ObjectRayDirectionEXT;
    float tMin = gl_RayTminEXT;
    float tMax = gl_RayTmaxEXT;
    
    // 与单位球求交
    vec3 oc = rayOrigin;
    float a = dot(rayDirection, rayDirection);
    float halfB = dot(oc, rayDirection);
    float c = dot(oc, oc) - 1.0;
    
    float discriminant = halfB * halfB - a * c;
    if (discriminant < 0.0) return;
    
    float sqrtd = sqrt(discriminant);
    float root0 = (-halfB - sqrtd) / a;
    float root1 = (-halfB + sqrtd) / a;
    
    if (root0 > tMin && root0 < tMax) {
        reportIntersectionEXT(root0, 0);
    }
    if (root1 > tMin && root1 < tMax) {
        reportIntersectionEXT(root1, 0);
    }
}
