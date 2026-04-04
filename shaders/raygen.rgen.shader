#version 460
#extension GL_EXT_ray_tracing : require

const float PI = 3.14159265359;
const float EPSILON = 1e-4;
const float SHADOW_EPSILON = 1e-3;

struct MaterialData {
    vec4 baseColor;
    vec4 emission;
    vec4 params;
    vec4 medium;
};

layout(binding = 0, set = 0) uniform accelerationStructureEXT tlas;
layout(binding = 1, set = 0, rgba32f) uniform image2D outputImage;
layout(binding = 2, set = 0) readonly buffer Materials {
    MaterialData materials[];
} matBuffer;
layout(binding = 3, set = 0) readonly buffer FrameDataBuffer {
    vec4 origin;
    vec4 lowerLeftCorner;
    vec4 horizontal;
    vec4 vertical;
    vec4 basisULensRadius;
    vec4 basisVPadding;
} frameData;
layout(binding = 4, set = 0) readonly buffer LightDataBuffer {
    vec4 modeAndParams;
    vec4 positionRadius;
    vec4 directionRange;
    vec4 colorIntensity;
} lightData;

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

struct BsdfSample {
    vec3 direction;
    vec3 bsdf;
    vec3 weight;
    float pdf;
    bool isDelta;
    bool valid;
};

uint pcgHash(uint seed) {
    uint state = seed * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

float randomFloat(inout uint seed) {
    seed = pcgHash(seed);
    return float(seed) / float(0xffffffffu);
}

float randomFloatRange(inout uint seed, float minValue, float maxValue) {
    return minValue + (maxValue - minValue) * randomFloat(seed);
}

vec3 randomInUnitDisk(inout uint seed) {
    while (true) {
        vec3 point = vec3(
            randomFloatRange(seed, -1.0, 1.0),
            randomFloatRange(seed, -1.0, 1.0),
            0.0
        );
        if (dot(point, point) < 1.0) {
            return point;
        }
    }
}

vec3 sampleUniformSphere(inout uint seed) {
    float u1 = randomFloat(seed);
    float u2 = randomFloat(seed);
    float z = 1.0 - 2.0 * u1;
    float radius = sqrt(max(0.0, 1.0 - z * z));
    float phi = 2.0 * PI * u2;
    return vec3(radius * cos(phi), radius * sin(phi), z);
}

Ray getRay(float s, float t, inout uint seed) {
    float lensRadius = frameData.basisULensRadius.w;
    vec3 basisU = frameData.basisULensRadius.xyz;
    vec3 basisV = frameData.basisVPadding.xyz;
    vec3 lensSample = lensRadius * randomInUnitDisk(seed);
    vec3 offset = basisU * lensSample.x + basisV * lensSample.y;

    Ray ray;
    ray.origin = frameData.origin.xyz + offset;
    ray.direction = frameData.lowerLeftCorner.xyz
        + s * frameData.horizontal.xyz
        + t * frameData.vertical.xyz
        - frameData.origin.xyz
        - offset;
    return ray;
}

vec3 customReflect(vec3 incident, vec3 normal) {
    return incident - 2.0 * dot(normal, incident) * normal;
}

vec3 customRefract(vec3 incident, vec3 normal, float eta) {
    float cosTheta = min(dot(-incident, normal), 1.0);
    vec3 refractedPerpendicular = eta * (incident + cosTheta * normal);
    vec3 refractedParallel = -sqrt(abs(1.0 - dot(refractedPerpendicular, refractedPerpendicular))) * normal;
    return refractedPerpendicular + refractedParallel;
}

float schlickWeight(float cosine) {
    float m = clamp(1.0 - cosine, 0.0, 1.0);
    return m * m * m * m * m;
}

vec3 fresnelSchlick(vec3 f0, float cosine) {
    return f0 + (vec3(1.0) - f0) * schlickWeight(cosine);
}

float fresnelDielectric(float cosine, float ior) {
    float r0 = (1.0 - ior) / (1.0 + ior);
    r0 *= r0;
    return r0 + (1.0 - r0) * schlickWeight(cosine);
}

void buildOrthonormalBasis(vec3 normal, out vec3 tangent, out vec3 bitangent) {
    vec3 helper = abs(normal.z) < 0.999 ? vec3(0.0, 0.0, 1.0) : vec3(1.0, 0.0, 0.0);
    tangent = normalize(cross(helper, normal));
    bitangent = cross(normal, tangent);
}

vec3 toWorld(vec3 localDirection, vec3 normal) {
    vec3 tangent;
    vec3 bitangent;
    buildOrthonormalBasis(normal, tangent, bitangent);
    return tangent * localDirection.x + bitangent * localDirection.y + normal * localDirection.z;
}

vec3 sampleCosineHemisphere(inout uint seed) {
    float u1 = randomFloat(seed);
    float u2 = randomFloat(seed);
    float radius = sqrt(u1);
    float phi = 2.0 * PI * u2;
    return vec3(
        radius * cos(phi),
        radius * sin(phi),
        sqrt(max(0.0, 1.0 - u1))
    );
}

float ggxDistribution(float alpha, float nDotH) {
    float alpha2 = alpha * alpha;
    float denominator = nDotH * nDotH * (alpha2 - 1.0) + 1.0;
    return alpha2 / max(PI * denominator * denominator, EPSILON);
}

float smithMaskingTerm(float alpha, float nDotDirection) {
    float alpha2 = alpha * alpha;
    float nDot2 = nDotDirection * nDotDirection;
    return (2.0 * nDotDirection) / max(nDotDirection + sqrt(alpha2 + (1.0 - alpha2) * nDot2), EPSILON);
}

float smithGeometry(float alpha, float nDotV, float nDotL) {
    return smithMaskingTerm(alpha, nDotV) * smithMaskingTerm(alpha, nDotL);
}

vec3 sampleGGXHalfVector(vec3 normal, float alpha, inout uint seed) {
    float u1 = randomFloat(seed);
    float u2 = randomFloat(seed);
    float phi = 2.0 * PI * u1;
    float cosTheta = sqrt((1.0 - u2) / max(1.0 + (alpha * alpha - 1.0) * u2, EPSILON));
    float sinTheta = sqrt(max(0.0, 1.0 - cosTheta * cosTheta));
    vec3 localHalfVector = vec3(sinTheta * cos(phi), sinTheta * sin(phi), cosTheta);
    return normalize(toWorld(localHalfVector, normal));
}

vec3 offsetRayOrigin(vec3 position, vec3 normal, vec3 direction) {
    float side = dot(direction, normal) >= 0.0 ? 1.0 : -1.0;
    return position + normal * side * SHADOW_EPSILON;
}

bool isTransmissionMaterial(MaterialData material) {
    return material.params.z > 0.5;
}

bool isMetalMaterial(MaterialData material) {
    return material.params.y > 0.5 && !isTransmissionMaterial(material);
}

vec3 sampleSky(vec3 direction) {
    vec3 unitDirection = normalize(direction);
    float t = 0.5 * (unitDirection.y + 1.0);
    return mix(vec3(1.0), vec3(0.5, 0.7, 1.0), t) * lightData.colorIntensity.rgb * lightData.modeAndParams.y;
}

vec3 evaluateBackground(vec3 direction) {
    int lightMode = int(lightData.modeAndParams.x + 0.5);
    if (lightMode == 0) {
        return sampleSky(direction);
    }
    return vec3(0.0);
}

float pdfDiffuse(vec3 normal, vec3 outgoingDirection) {
    return max(dot(normal, outgoingDirection), 0.0) / PI;
}

vec3 evaluateDiffuse(MaterialData material) {
    return material.baseColor.rgb / PI;
}

float pdfMetal(MaterialData material, vec3 normal, vec3 incomingDirection, vec3 outgoingDirection) {
    float roughness = clamp(material.params.x, 0.02, 1.0);
    float alpha = max(roughness * roughness, 0.001);
    vec3 halfVector = normalize(outgoingDirection - incomingDirection);
    float nDotH = max(dot(normal, halfVector), 0.0);
    float vDotH = max(dot(-incomingDirection, halfVector), 0.0);

    if (nDotH <= 0.0 || vDotH <= 0.0) {
        return 0.0;
    }

    float distribution = ggxDistribution(alpha, nDotH);
    return max(distribution * nDotH / max(4.0 * vDotH, EPSILON), EPSILON);
}

vec3 evaluateMetal(MaterialData material, vec3 normal, vec3 incomingDirection, vec3 outgoingDirection) {
    float nDotL = max(dot(normal, outgoingDirection), 0.0);
    float nDotV = max(dot(normal, -incomingDirection), 0.0);

    if (nDotL <= 0.0 || nDotV <= 0.0) {
        return vec3(0.0);
    }

    float roughness = clamp(material.params.x, 0.02, 1.0);
    float alpha = max(roughness * roughness, 0.001);
    vec3 halfVector = normalize(outgoingDirection - incomingDirection);
    float nDotH = max(dot(normal, halfVector), 0.0);
    float vDotH = max(dot(-incomingDirection, halfVector), 0.0);

    if (nDotH <= 0.0 || vDotH <= 0.0) {
        return vec3(0.0);
    }

    vec3 fresnel = fresnelSchlick(material.baseColor.rgb, vDotH);
    float distribution = ggxDistribution(alpha, nDotH);
    float geometry = smithGeometry(alpha, nDotV, nDotL);
    return fresnel * (distribution * geometry / max(4.0 * nDotV * nDotL, EPSILON));
}

vec3 evaluateBSDF(MaterialData material, vec3 normal, vec3 incomingDirection, vec3 outgoingDirection, uint frontFace) {
    if (isTransmissionMaterial(material)) {
        return vec3(0.0);
    }

    if (isMetalMaterial(material)) {
        return evaluateMetal(material, normal, incomingDirection, outgoingDirection);
    }

    return evaluateDiffuse(material);
}

float pdfBSDF(MaterialData material, vec3 normal, vec3 incomingDirection, vec3 outgoingDirection, uint frontFace) {
    if (isTransmissionMaterial(material)) {
        return 0.0;
    }

    if (isMetalMaterial(material)) {
        return pdfMetal(material, normal, incomingDirection, outgoingDirection);
    }

    return pdfDiffuse(normal, outgoingDirection);
}

BsdfSample sampleDiffuse(MaterialData material, vec3 normal, inout uint seed) {
    BsdfSample bsdfSample;
    bsdfSample.direction = normalize(toWorld(sampleCosineHemisphere(seed), normal));
    bsdfSample.bsdf = evaluateDiffuse(material);
    bsdfSample.pdf = pdfDiffuse(normal, bsdfSample.direction);
    float nDotL = max(dot(normal, bsdfSample.direction), 0.0);
    bsdfSample.weight = bsdfSample.bsdf * nDotL / max(bsdfSample.pdf, EPSILON);
    bsdfSample.isDelta = false;
    bsdfSample.valid = nDotL > 0.0 && bsdfSample.pdf > 0.0;
    return bsdfSample;
}

BsdfSample sampleMetal(MaterialData material, vec3 normal, vec3 incomingDirection, inout uint seed) {
    BsdfSample bsdfSample;
    bsdfSample.isDelta = false;
    bsdfSample.valid = false;

    float roughness = clamp(material.params.x, 0.02, 1.0);
    float alpha = max(roughness * roughness, 0.001);
    vec3 halfVector = sampleGGXHalfVector(normal, alpha, seed);
    bsdfSample.direction = normalize(customReflect(incomingDirection, halfVector));

    float nDotL = max(dot(normal, bsdfSample.direction), 0.0);
    if (nDotL <= 0.0) {
        return bsdfSample;
    }

    bsdfSample.bsdf = evaluateMetal(material, normal, incomingDirection, bsdfSample.direction);
    bsdfSample.pdf = pdfMetal(material, normal, incomingDirection, bsdfSample.direction);
    bsdfSample.weight = bsdfSample.bsdf * nDotL / max(bsdfSample.pdf, EPSILON);
    bsdfSample.valid = bsdfSample.pdf > 0.0 && max(bsdfSample.weight.r, max(bsdfSample.weight.g, bsdfSample.weight.b)) > 0.0;
    return bsdfSample;
}

BsdfSample sampleDielectric(MaterialData material, vec3 normal, vec3 incomingDirection, uint frontFace, inout uint seed) {
    BsdfSample bsdfSample;
    bsdfSample.bsdf = vec3(0.0);
    bsdfSample.isDelta = true;
    bsdfSample.valid = true;

    float ior = max(material.params.w, 1.0);
    float eta = frontFace == 1u ? (1.0 / ior) : ior;
    vec3 unitIncoming = normalize(incomingDirection);
    float cosine = min(dot(-unitIncoming, normal), 1.0);
    float sine = sqrt(max(0.0, 1.0 - cosine * cosine));
    bool totalInternalReflection = eta * sine > 1.0;
    float reflectance = fresnelDielectric(cosine, ior);

    if (totalInternalReflection || randomFloat(seed) < reflectance) {
        bsdfSample.direction = normalize(customReflect(unitIncoming, normal));
        bsdfSample.pdf = max(reflectance, EPSILON);
        bsdfSample.weight = vec3(1.0);
    } else {
        bsdfSample.direction = normalize(customRefract(unitIncoming, normal, eta));
        bsdfSample.pdf = max(1.0 - reflectance, EPSILON);
        bsdfSample.weight = material.baseColor.rgb;
    }

    return bsdfSample;
}

BsdfSample sampleBSDF(MaterialData material, vec3 normal, vec3 incomingDirection, uint frontFace, inout uint seed) {
    if (isTransmissionMaterial(material)) {
        return sampleDielectric(material, normal, incomingDirection, frontFace, seed);
    }

    if (isMetalMaterial(material)) {
        return sampleMetal(material, normal, incomingDirection, seed);
    }

    return sampleDiffuse(material, normal, seed);
}

bool traceVisibility(vec3 position, vec3 normal, vec3 direction, float maxDistance) {
    payload.isMiss = 0u;
    traceRayEXT(
        tlas,
        gl_RayFlagsTerminateOnFirstHitEXT | gl_RayFlagsSkipClosestHitShaderEXT | gl_RayFlagsOpaqueEXT,
        0xFF,
        0, 0, 0,
        offsetRayOrigin(position, normal, direction),
        SHADOW_EPSILON,
        direction,
        maxDistance,
        0
    );
    return payload.isMiss == 1u;
}

vec3 evaluateDirectLighting(
    MaterialData material,
    vec3 position,
    vec3 normal,
    vec3 incomingDirection,
    uint frontFace,
    inout uint seed
) {
    if (isTransmissionMaterial(material)) {
        return vec3(0.0);
    }

    int lightMode = int(lightData.modeAndParams.x + 0.5);
    vec3 lightContribution = vec3(0.0);

    if (lightMode == 1) {
        vec3 toLight = lightData.positionRadius.xyz - position;
        float distanceToLight = length(toLight);
        vec3 lightDirection = toLight / max(distanceToLight, EPSILON);
        float nDotL = max(dot(normal, lightDirection), 0.0);
        if (nDotL > 0.0 && traceVisibility(position, normal, lightDirection, max(distanceToLight - SHADOW_EPSILON, SHADOW_EPSILON))) {
            vec3 incidentRadiance = lightData.colorIntensity.rgb / max(distanceToLight * distanceToLight, 1.0);
            lightContribution = evaluateBSDF(material, normal, incomingDirection, lightDirection, frontFace) * incidentRadiance * nDotL;
        }
    } else if (lightMode == 2) {
        vec3 lightDirection = normalize(-lightData.directionRange.xyz);
        float nDotL = max(dot(normal, lightDirection), 0.0);
        if (nDotL > 0.0 && traceVisibility(position, normal, lightDirection, 1e4)) {
            lightContribution = evaluateBSDF(material, normal, incomingDirection, lightDirection, frontFace)
                * lightData.colorIntensity.rgb
                * nDotL;
        }
    } else if (lightMode == 3) {
        vec3 lightCenter = lightData.positionRadius.xyz;
        float radius = max(lightData.positionRadius.w, 0.01);
        vec3 lightNormal = sampleUniformSphere(seed);
        vec3 lightPosition = lightCenter + radius * lightNormal;
        vec3 toLight = lightPosition - position;
        float distanceSquared = dot(toLight, toLight);
        float distanceToLight = sqrt(distanceSquared);
        vec3 lightDirection = toLight / max(distanceToLight, EPSILON);
        float nDotL = max(dot(normal, lightDirection), 0.0);
        float lightCosine = max(dot(lightNormal, -lightDirection), 0.0);
        float pdfArea = 1.0 / max(4.0 * PI * radius * radius, EPSILON);

        if (nDotL > 0.0
            && lightCosine > 0.0
            && traceVisibility(position, normal, lightDirection, max(distanceToLight - SHADOW_EPSILON, SHADOW_EPSILON))) {
            vec3 bsdfValue = evaluateBSDF(material, normal, incomingDirection, lightDirection, frontFace);
            lightContribution = bsdfValue
                * lightData.colorIntensity.rgb
                * nDotL
                * lightCosine
                / max(distanceSquared * pdfArea, EPSILON);
        }
    }

    return lightContribution;
}

void main() {
    uint randSeed = (gl_LaunchIDEXT.y * gl_LaunchSizeEXT.x + gl_LaunchIDEXT.x) ^ constants.seed;

    float u = (float(gl_LaunchIDEXT.x) + randomFloat(randSeed)) / float(gl_LaunchSizeEXT.x - 1);
    float v = (float(gl_LaunchIDEXT.y) + randomFloat(randSeed)) / float(gl_LaunchSizeEXT.y - 1);

    Ray ray = getRay(u, v, randSeed);
    vec3 throughput = vec3(1.0);
    vec3 radiance = vec3(0.0);

    const int maxDepth = 50;
    for (int depth = 0; depth < maxDepth; depth++) {
        traceRayEXT(
            tlas,
            gl_RayFlagsOpaqueEXT,
            0xFF,
            0, 0, 0,
            ray.origin,
            EPSILON,
            ray.direction,
            1e4,
            0
        );

        if (payload.isMiss == 1u) {
            radiance += throughput * evaluateBackground(ray.direction);
            break;
        }

        vec3 hitPosition = payload.position;
        vec3 surfaceNormal = payload.normal;
        uint materialIndex = payload.material;
        uint frontFace = payload.frontFace;
        MaterialData material = matBuffer.materials[materialIndex];

        if (length(material.emission.rgb) > 0.0) {
            radiance += throughput * material.emission.rgb;
            break;
        }

        vec3 incomingDirection = normalize(ray.direction);
        radiance += throughput * evaluateDirectLighting(
            material,
            hitPosition,
            surfaceNormal,
            incomingDirection,
            frontFace,
            randSeed
        );

        BsdfSample bsdfSample = sampleBSDF(
            material,
            surfaceNormal,
            incomingDirection,
            frontFace,
            randSeed
        );

        if (!bsdfSample.valid) {
            break;
        }

        throughput *= bsdfSample.weight;
        ray.origin = offsetRayOrigin(hitPosition, surfaceNormal, bsdfSample.direction);
        ray.direction = bsdfSample.direction;

        if (depth >= 4) {
            float continuationProbability = clamp(max(throughput.r, max(throughput.g, throughput.b)), 0.05, 0.95);
            if (randomFloat(randSeed) > continuationProbability) {
                break;
            }
            throughput /= continuationProbability;
        }
    }

    vec4 oldColor = imageLoad(outputImage, ivec2(gl_LaunchIDEXT.xy));
    imageStore(outputImage, ivec2(gl_LaunchIDEXT.xy), oldColor + vec4(radiance, 1.0));
}
