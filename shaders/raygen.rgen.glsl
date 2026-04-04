#version 460
#extension GL_EXT_ray_tracing : require

const float PI = 3.14159265359;
const float TWO_PI = 2.0 * PI;
const float EPSILON = 1e-4;
const float SHADOW_EPSILON = 1e-3;
const float DELTA_ROUGHNESS_THRESHOLD = 0.03;
const float METAL_DELTA_ROUGHNESS_THRESHOLD = 0.05;

const int MAX_LIGHTS = 8;
const int MAX_MEDIUM_DEPTH = 8;
const int ENV_MAP_WIDTH = 512;
const int ENV_MAP_HEIGHT = 256;

const uint BSDF_EVENT_DELTA = 1u << 0;
const uint BSDF_EVENT_DIFFUSE = 1u << 1;
const uint BSDF_EVENT_GLOSSY = 1u << 2;
const uint BSDF_EVENT_REFLECTION = 1u << 3;
const uint BSDF_EVENT_TRANSMISSION = 1u << 4;

const uint LIGHT_FLAG_DELTA = 1u << 0;
const uint LIGHT_FLAG_FINITE = 1u << 1;
const uint LIGHT_FLAG_ENVIRONMENT = 1u << 2;

const int LIGHT_TYPE_ENVIRONMENT = 0;
const int LIGHT_TYPE_POINT = 1;
const int LIGHT_TYPE_DIRECTIONAL = 2;
const int LIGHT_TYPE_AREA_SPHERE = 3;

struct MaterialData {
    vec4 baseColor;
    vec4 emission;
    vec4 params;
    vec4 medium;
};

struct GpuLight {
    vec4 positionRadius;
    vec4 directionType;
    vec4 emissionPmf;
    vec4 params;
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
    vec4 meta;
    GpuLight lights[];
} lightData;
layout(binding = 5, set = 0) readonly buffer EnvironmentTexelBuffer {
    vec4 texels[];
} environmentTexels;
layout(binding = 6, set = 0) readonly buffer EnvironmentPmfBuffer {
    float values[];
} environmentPmf;
layout(binding = 7, set = 0) readonly buffer EnvironmentConditionalCdfBuffer {
    float values[];
} environmentConditionalCdf;
layout(binding = 8, set = 0) readonly buffer EnvironmentMarginalCdfBuffer {
    float values[];
} environmentMarginalCdf;

layout(location = 0) rayPayloadEXT RayPayload {
    uint isMiss;
    float distance;
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
    float eta;
    uint flags;
    bool valid;
};

struct LightSample {
    vec3 direction;
    vec3 radiance;
    float pdf;
    float distance;
    uint flags;
    bool valid;
};

bool isDeltaBsdfSample(BsdfSample bsdfSampleValue) {
    return (bsdfSampleValue.flags & BSDF_EVENT_DELTA) != 0u;
}

bool isDeltaLightSample(LightSample lightSampleValue) {
    return (lightSampleValue.flags & LIGHT_FLAG_DELTA) != 0u;
}

BsdfSample invalidBsdfSample() {
    BsdfSample bsdfSample;
    bsdfSample.direction = vec3(0.0);
    bsdfSample.bsdf = vec3(0.0);
    bsdfSample.weight = vec3(0.0);
    bsdfSample.pdf = 0.0;
    bsdfSample.eta = 1.0;
    bsdfSample.flags = 0u;
    bsdfSample.valid = false;
    return bsdfSample;
}

LightSample invalidLightSample() {
    LightSample lightSample;
    lightSample.direction = vec3(0.0);
    lightSample.radiance = vec3(0.0);
    lightSample.pdf = 0.0;
    lightSample.distance = 0.0;
    lightSample.flags = 0u;
    lightSample.valid = false;
    return lightSample;
}

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
    float phi = TWO_PI * u2;
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

float saturate(float value) {
    return clamp(value, 0.0, 1.0);
}

float maxComponent(vec3 value) {
    return max(value.x, max(value.y, value.z));
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

float powerHeuristic(float pdfA, float pdfB) {
    float a2 = pdfA * pdfA;
    float b2 = pdfB * pdfB;
    return a2 / max(a2 + b2, EPSILON);
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
    float phi = TWO_PI * u2;
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
    float phi = TWO_PI * u1;
    float cosTheta = sqrt((1.0 - u2) / max(1.0 + (alpha * alpha - 1.0) * u2, EPSILON));
    float sinTheta = sqrt(max(0.0, 1.0 - cosTheta * cosTheta));
    vec3 localHalfVector = vec3(sinTheta * cos(phi), sinTheta * sin(phi), cosTheta);
    return normalize(toWorld(localHalfVector, normal));
}

vec3 offsetRayOrigin(vec3 position, vec3 normal, vec3 direction) {
    float side = dot(direction, normal) >= 0.0 ? 1.0 : -1.0;
    return position + normal * side * SHADOW_EPSILON;
}

float materialRoughness(MaterialData material) {
    return clamp(material.params.x, 0.001, 1.0);
}

float materialMetallic(MaterialData material) {
    return clamp(material.params.y, 0.0, 1.0);
}

float materialTransmission(MaterialData material) {
    return clamp(material.params.z, 0.0, 1.0);
}

float materialIor(MaterialData material) {
    return max(material.params.w, 1.0);
}

float materialSpecular(MaterialData material) {
    return clamp(material.baseColor.a, 0.0, 1.0);
}

float materialClearcoat(MaterialData material) {
    return clamp(material.emission.a, 0.0, 1.0);
}

bool isInvisibleMaterial(MaterialData material) {
    return material.medium.w < 0.0;
}

vec3 materialAbsorption(MaterialData material) {
    return isInvisibleMaterial(material)
        ? vec3(0.0)
        : max(material.medium.rgb * max(material.medium.w, 0.0), vec3(0.0));
}

bool isTransmissionMaterial(MaterialData material) {
    return materialTransmission(material) > 0.001 && materialMetallic(material) < 0.999;
}

bool isDeltaTransmissionMaterial(MaterialData material) {
    return isTransmissionMaterial(material) && materialRoughness(material) <= DELTA_ROUGHNESS_THRESHOLD;
}

bool isDeltaMetalMaterial(MaterialData material) {
    return materialTransmission(material) <= 0.001
        && materialMetallic(material) > 0.95
        && materialRoughness(material) <= METAL_DELTA_ROUGHNESS_THRESHOLD;
}

float dielectricF0FromIor(float ior) {
    float r0 = (ior - 1.0) / (ior + 1.0);
    return r0 * r0;
}

vec3 specularF0(MaterialData material) {
    vec3 dielectricF0 = vec3(dielectricF0FromIor(materialIor(material)) * materialSpecular(material));
    return mix(dielectricF0, material.baseColor.rgb, materialMetallic(material));
}

float clearcoatAlpha(MaterialData material) {
    return max(0.001, materialRoughness(material) * materialRoughness(material) * 0.25);
}

float diffuseLobeWeight(MaterialData material) {
    return (1.0 - materialMetallic(material))
        * (1.0 - materialTransmission(material))
        * maxComponent(material.baseColor.rgb);
}

float specularLobeWeight(MaterialData material) {
    return max(0.05, maxComponent(specularF0(material)));
}

float transmissionLobeWeight(MaterialData material) {
    return (1.0 - materialMetallic(material)) * materialTransmission(material);
}

float clearcoatLobeWeight(MaterialData material) {
    return materialClearcoat(material) * 0.25;
}

bool canRefractThroughNormal(vec3 incident, vec3 normal, float eta) {
    float cosine = min(dot(-incident, normal), 1.0);
    float sin2Theta = max(0.0, 1.0 - cosine * cosine);
    return eta * eta * sin2Theta <= 1.0;
}

vec4 bsdfSampleProbabilities(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    uint frontFace
) {
    float diffuseWeight = diffuseLobeWeight(material);
    float specularWeight = specularLobeWeight(material);
    float transmissionWeight = transmissionLobeWeight(material);
    float clearcoatWeight = clearcoatLobeWeight(material);

    if (transmissionWeight > 0.0) {
        float ior = materialIor(material);
        float eta = frontFace == 1u ? (1.0 / ior) : ior;
        float cosine = saturate(dot(normal, -incomingDirection));
        float fresnel = fresnelDielectric(cosine, ior);

        if (canRefractThroughNormal(incomingDirection, normal, eta)) {
            specularWeight += transmissionWeight * fresnel;
            transmissionWeight *= (1.0 - fresnel);
        } else {
            specularWeight += transmissionWeight;
            transmissionWeight = 0.0;
        }
    }

    float sum = diffuseWeight + specularWeight + transmissionWeight + clearcoatWeight;
    if (sum <= EPSILON) {
        return vec4(0.0);
    }

    return vec4(diffuseWeight, specularWeight, transmissionWeight, clearcoatWeight) / sum;
}

int lightCount() {
    return clamp(int(lightData.meta.x + 0.5), 0, MAX_LIGHTS);
}

GpuLight getLight(int index) {
    return lightData.lights[index];
}

int lightType(GpuLight light) {
    return int(light.directionType.w + 0.5);
}

float lightSelectionPmf(int index) {
    return max(lightData.lights[index].emissionPmf.w, 0.0);
}

float lightSelectionCdf(int index) {
    return clamp(lightData.lights[index].params.x, 0.0, 1.0);
}

int findLightByType(int typeId) {
    int count = lightCount();
    for (int index = 0; index < count; ++index) {
        if (lightType(getLight(index)) == typeId) {
            return index;
        }
    }
    return -1;
}

bool hasVisibleEmissiveLight() {
    int areaLightIndex = findLightByType(LIGHT_TYPE_AREA_SPHERE);
    return areaLightIndex >= 0 && getLight(areaLightIndex).params.y > 0.5;
}
int wrapEnvironmentX(int x) {
    int width = ENV_MAP_WIDTH;
    int wrapped = x % width;
    return wrapped < 0 ? wrapped + width : wrapped;
}

int clampEnvironmentY(int y) {
    return clamp(y, 0, ENV_MAP_HEIGHT - 1);
}

int environmentIndex(int x, int y) {
    return clampEnvironmentY(y) * ENV_MAP_WIDTH + wrapEnvironmentX(x);
}

vec2 directionToEnvironmentUv(vec3 direction) {
    vec3 unitDirection = normalize(direction);
    float phi = atan(unitDirection.z, unitDirection.x);
    float theta = acos(clamp(unitDirection.y, -1.0, 1.0));
    return vec2((phi + PI) / TWO_PI, theta / PI);
}

vec3 environmentUvToDirection(vec2 uv) {
    float phi = uv.x * TWO_PI - PI;
    float theta = uv.y * PI;
    float sinTheta = sin(theta);
    return normalize(vec3(sinTheta * cos(phi), cos(theta), sinTheta * sin(phi)));
}

vec3 environmentTexel(int x, int y) {
    return environmentTexels.texels[environmentIndex(x, y)].rgb;
}

vec3 sampleEnvironment(vec3 direction) {
    vec2 uv = directionToEnvironmentUv(direction);
    float x = uv.x * float(ENV_MAP_WIDTH) - 0.5;
    float y = uv.y * float(ENV_MAP_HEIGHT) - 0.5;
    int x0 = int(floor(x));
    int y0 = int(floor(y));
    int x1 = x0 + 1;
    int y1 = y0 + 1;
    float tx = fract(x);
    float ty = fract(y);

    vec3 c00 = environmentTexel(x0, y0);
    vec3 c10 = environmentTexel(x1, y0);
    vec3 c01 = environmentTexel(x0, y1);
    vec3 c11 = environmentTexel(x1, y1);

    return mix(mix(c00, c10, tx), mix(c01, c11, tx), ty);
}

float environmentTexelSolidAngle(int y) {
    float theta0 = PI * float(y) / float(ENV_MAP_HEIGHT);
    float theta1 = PI * float(y + 1) / float(ENV_MAP_HEIGHT);
    float dPhi = TWO_PI / float(ENV_MAP_WIDTH);
    return max(dPhi * (cos(theta0) - cos(theta1)), EPSILON);
}

float environmentPdfFromIndex(int x, int y) {
    float pmf = max(environmentPmf.values[environmentIndex(x, y)], 0.0);
    return pmf / environmentTexelSolidAngle(y);
}

float environmentPdf(vec3 direction) {
    int environmentLightIndex = findLightByType(LIGHT_TYPE_ENVIRONMENT);
    if (environmentLightIndex < 0) {
        return 0.0;
    }

    vec2 uv = directionToEnvironmentUv(direction);
    int x = clamp(int(floor(uv.x * float(ENV_MAP_WIDTH))), 0, ENV_MAP_WIDTH - 1);
    int y = clamp(int(floor(uv.y * float(ENV_MAP_HEIGHT))), 0, ENV_MAP_HEIGHT - 1);
    return environmentPdfFromIndex(x, y);
}

int sampleEnvironmentMarginalRow(float xi) {
    int low = 0;
    int high = ENV_MAP_HEIGHT - 1;
    for (int iteration = 0; iteration < 12; ++iteration) {
        int mid = (low + high) / 2;
        float cdfValue = environmentMarginalCdf.values[mid];
        if (xi > cdfValue) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    return clamp(low, 0, ENV_MAP_HEIGHT - 1);
}

int sampleEnvironmentConditionalColumn(int row, float xi) {
    int low = 0;
    int high = ENV_MAP_WIDTH - 1;
    int rowOffset = row * ENV_MAP_WIDTH;
    for (int iteration = 0; iteration < 16; ++iteration) {
        int mid = (low + high) / 2;
        float cdfValue = environmentConditionalCdf.values[rowOffset + mid];
        if (xi > cdfValue) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    return clamp(low, 0, ENV_MAP_WIDTH - 1);
}

vec3 evaluateBackground(vec3 direction) {
    return findLightByType(LIGHT_TYPE_ENVIRONMENT) >= 0 ? sampleEnvironment(direction) : vec3(0.0);
}

float pdfDiffuse(vec3 normal, vec3 outgoingDirection) {
    return max(dot(normal, outgoingDirection), 0.0) / PI;
}

vec3 evaluateDiffuse(MaterialData material) {
    return material.baseColor.rgb / PI;
}

float reflectionAlpha(MaterialData material) {
    return max(materialRoughness(material) * materialRoughness(material), 0.001);
}

float pdfMicrofacetReflection(MaterialData material, vec3 normal, vec3 incomingDirection, vec3 outgoingDirection) {
    vec3 wi = normalize(-incomingDirection);
    vec3 wo = normalize(outgoingDirection);
    if (dot(normal, wo) <= 0.0 || dot(normal, wi) <= 0.0) {
        return 0.0;
    }

    vec3 halfVector = normalize(wi + wo);
    if (length(halfVector) <= EPSILON) {
        return 0.0;
    }
    if (dot(halfVector, normal) < 0.0) {
        halfVector = -halfVector;
    }

    float nDotH = max(dot(normal, halfVector), 0.0);
    float woDotH = abs(dot(wo, halfVector));
    if (nDotH <= 0.0 || woDotH <= 0.0) {
        return 0.0;
    }

    float distribution = ggxDistribution(reflectionAlpha(material), nDotH);
    return max(distribution * nDotH / max(4.0 * woDotH, EPSILON), 0.0);
}

vec3 evaluateMicrofacetReflection(MaterialData material, vec3 normal, vec3 incomingDirection, vec3 outgoingDirection) {
    vec3 wi = normalize(-incomingDirection);
    vec3 wo = normalize(outgoingDirection);
    float nDotWi = max(dot(normal, wi), 0.0);
    float nDotWo = max(dot(normal, wo), 0.0);
    if (nDotWi <= 0.0 || nDotWo <= 0.0) {
        return vec3(0.0);
    }

    vec3 halfVector = normalize(wi + wo);
    if (length(halfVector) <= EPSILON) {
        return vec3(0.0);
    }
    if (dot(halfVector, normal) < 0.0) {
        halfVector = -halfVector;
    }

    float nDotH = max(dot(normal, halfVector), 0.0);
    float wiDotH = abs(dot(wi, halfVector));
    if (nDotH <= 0.0 || wiDotH <= 0.0) {
        return vec3(0.0);
    }

    vec3 fresnel = fresnelSchlick(specularF0(material), wiDotH);
    float distribution = ggxDistribution(reflectionAlpha(material), nDotH);
    float geometry = smithGeometry(reflectionAlpha(material), nDotWi, nDotWo);
    return fresnel * (distribution * geometry / max(4.0 * nDotWi * nDotWo, EPSILON));
}

float pdfClearcoat(MaterialData material, vec3 normal, vec3 incomingDirection, vec3 outgoingDirection) {
    vec3 wi = normalize(-incomingDirection);
    vec3 wo = normalize(outgoingDirection);
    if (dot(normal, wo) <= 0.0 || dot(normal, wi) <= 0.0) {
        return 0.0;
    }

    vec3 halfVector = normalize(wi + wo);
    if (length(halfVector) <= EPSILON) {
        return 0.0;
    }
    if (dot(halfVector, normal) < 0.0) {
        halfVector = -halfVector;
    }

    float nDotH = max(dot(normal, halfVector), 0.0);
    float woDotH = abs(dot(wo, halfVector));
    if (nDotH <= 0.0 || woDotH <= 0.0) {
        return 0.0;
    }

    float distribution = ggxDistribution(clearcoatAlpha(material), nDotH);
    return max(distribution * nDotH / max(4.0 * woDotH, EPSILON), 0.0);
}

vec3 evaluateClearcoat(MaterialData material, vec3 normal, vec3 incomingDirection, vec3 outgoingDirection) {
    float clearcoat = materialClearcoat(material);
    if (clearcoat <= 0.0) {
        return vec3(0.0);
    }

    vec3 wi = normalize(-incomingDirection);
    vec3 wo = normalize(outgoingDirection);
    float nDotWi = max(dot(normal, wi), 0.0);
    float nDotWo = max(dot(normal, wo), 0.0);
    if (nDotWi <= 0.0 || nDotWo <= 0.0) {
        return vec3(0.0);
    }

    vec3 halfVector = normalize(wi + wo);
    if (length(halfVector) <= EPSILON) {
        return vec3(0.0);
    }
    if (dot(halfVector, normal) < 0.0) {
        halfVector = -halfVector;
    }

    float nDotH = max(dot(normal, halfVector), 0.0);
    float wiDotH = abs(dot(wi, halfVector));
    if (nDotH <= 0.0 || wiDotH <= 0.0) {
        return vec3(0.0);
    }

    vec3 fresnel = fresnelSchlick(vec3(0.04), wiDotH);
    float distribution = ggxDistribution(clearcoatAlpha(material), nDotH);
    float geometry = smithGeometry(clearcoatAlpha(material), nDotWi, nDotWo);
    return clearcoat * fresnel * (distribution * geometry / max(4.0 * nDotWi * nDotWo, EPSILON));
}

vec3 transmissionHalfVector(
    vec3 normal,
    vec3 incomingDirection,
    vec3 outgoingDirection,
    float eta
) {
    vec3 wi = normalize(-incomingDirection);
    vec3 wo = normalize(outgoingDirection);
    vec3 halfVector = normalize(wi + eta * wo);
    if (length(halfVector) <= EPSILON) {
        return vec3(0.0);
    }
    if (dot(halfVector, normal) < 0.0) {
        halfVector = -halfVector;
    }
    return halfVector;
}

float pdfMicrofacetTransmission(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    vec3 outgoingDirection,
    uint frontFace
) {
    vec3 wi = normalize(-incomingDirection);
    vec3 wo = normalize(outgoingDirection);
    float nDotWi = dot(normal, wi);
    float nDotWo = dot(normal, wo);
    if (nDotWi <= 0.0 || nDotWo >= 0.0) {
        return 0.0;
    }

    float eta = frontFace == 1u ? (1.0 / materialIor(material)) : materialIor(material);
    vec3 halfVector = transmissionHalfVector(normal, incomingDirection, outgoingDirection, eta);
    if (length(halfVector) <= EPSILON) {
        return 0.0;
    }

    float nDotH = max(dot(normal, halfVector), 0.0);
    float wiDotH = dot(wi, halfVector);
    float woDotH = dot(wo, halfVector);
    float denominator = wiDotH + eta * woDotH;
    if (nDotH <= 0.0 || wiDotH <= 0.0 || woDotH >= 0.0 || abs(denominator) <= EPSILON) {
        return 0.0;
    }

    float distribution = ggxDistribution(reflectionAlpha(material), nDotH);
    float dwhDwo = abs((eta * eta * woDotH) / max(denominator * denominator, EPSILON));
    return max(distribution * nDotH * dwhDwo, 0.0);
}

vec3 evaluateMicrofacetTransmission(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    vec3 outgoingDirection,
    uint frontFace
) {
    float transmission = transmissionLobeWeight(material);
    if (transmission <= 0.0) {
        return vec3(0.0);
    }

    vec3 wi = normalize(-incomingDirection);
    vec3 wo = normalize(outgoingDirection);
    float nDotWi = dot(normal, wi);
    float nDotWo = dot(normal, wo);
    if (nDotWi <= 0.0 || nDotWo >= 0.0) {
        return vec3(0.0);
    }

    float eta = frontFace == 1u ? (1.0 / materialIor(material)) : materialIor(material);
    vec3 halfVector = transmissionHalfVector(normal, incomingDirection, outgoingDirection, eta);
    if (length(halfVector) <= EPSILON) {
        return vec3(0.0);
    }

    float nDotH = max(dot(normal, halfVector), 0.0);
    float wiDotH = dot(wi, halfVector);
    float woDotH = dot(wo, halfVector);
    float denominator = wiDotH + eta * woDotH;
    if (nDotH <= 0.0 || wiDotH <= 0.0 || woDotH >= 0.0 || abs(denominator) <= EPSILON) {
        return vec3(0.0);
    }

    float distribution = ggxDistribution(reflectionAlpha(material), nDotH);
    float geometry = smithGeometry(reflectionAlpha(material), abs(nDotWi), abs(nDotWo));
    float fresnel = fresnelDielectric(abs(wiDotH), materialIor(material));

    float numerator = (1.0 - fresnel)
        * distribution
        * geometry
        * eta
        * eta
        * abs(wiDotH * woDotH);
    float denominatorSquared = denominator * denominator;
    float scale = abs(numerator / max(abs(nDotWi * nDotWo) * denominatorSquared, EPSILON));

    return material.baseColor.rgb * transmission * scale;
}
bool sampleDiffuseDirection(vec3 normal, inout uint seed, out vec3 direction) {
    direction = normalize(toWorld(sampleCosineHemisphere(seed), normal));
    return dot(normal, direction) > 0.0;
}

bool sampleMicrofacetReflectionDirection(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    inout uint seed,
    out vec3 direction
) {
    for (int attempt = 0; attempt < 4; ++attempt) {
        vec3 halfVector = sampleGGXHalfVector(normal, reflectionAlpha(material), seed);
        direction = normalize(customReflect(incomingDirection, halfVector));
        if (dot(normal, direction) > 0.0) {
            return true;
        }
    }
    return false;
}

bool sampleClearcoatDirection(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    inout uint seed,
    out vec3 direction
) {
    for (int attempt = 0; attempt < 4; ++attempt) {
        vec3 halfVector = sampleGGXHalfVector(normal, clearcoatAlpha(material), seed);
        direction = normalize(customReflect(incomingDirection, halfVector));
        if (dot(normal, direction) > 0.0) {
            return true;
        }
    }
    return false;
}

bool sampleMicrofacetTransmissionDirection(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    uint frontFace,
    inout uint seed,
    out vec3 direction
) {
    float eta = frontFace == 1u ? (1.0 / materialIor(material)) : materialIor(material);
    for (int attempt = 0; attempt < 8; ++attempt) {
        vec3 halfVector = sampleGGXHalfVector(normal, reflectionAlpha(material), seed);
        if (!canRefractThroughNormal(incomingDirection, halfVector, eta)) {
            continue;
        }

        direction = normalize(customRefract(incomingDirection, halfVector, eta));
        if (dot(normal, direction) < 0.0) {
            return true;
        }
    }
    return false;
}

vec3 evaluateBSDF(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    vec3 outgoingDirection,
    uint frontFace
) {
    if (isInvisibleMaterial(material) || isDeltaTransmissionMaterial(material) || isDeltaMetalMaterial(material)) {
        return vec3(0.0);
    }

    vec3 value = vec3(0.0);
    bool sameHemisphere = dot(normal, outgoingDirection) > 0.0;

    if (sameHemisphere) {
        value += diffuseLobeWeight(material) * evaluateDiffuse(material);
        value += evaluateMicrofacetReflection(material, normal, incomingDirection, outgoingDirection);
        value += evaluateClearcoat(material, normal, incomingDirection, outgoingDirection);
    } else {
        value += evaluateMicrofacetTransmission(material, normal, incomingDirection, outgoingDirection, frontFace);
    }

    return value;
}

float pdfBSDF(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    vec3 outgoingDirection,
    uint frontFace
) {
    if (isInvisibleMaterial(material) || isDeltaTransmissionMaterial(material) || isDeltaMetalMaterial(material)) {
        return 0.0;
    }

    vec4 probabilities = bsdfSampleProbabilities(material, normal, incomingDirection, frontFace);
    bool sameHemisphere = dot(normal, outgoingDirection) > 0.0;

    if (sameHemisphere) {
        return probabilities.x * pdfDiffuse(normal, outgoingDirection)
            + probabilities.y * pdfMicrofacetReflection(material, normal, incomingDirection, outgoingDirection)
            + probabilities.w * pdfClearcoat(material, normal, incomingDirection, outgoingDirection);
    }

    return probabilities.z
        * pdfMicrofacetTransmission(material, normal, incomingDirection, outgoingDirection, frontFace);
}

BsdfSample sampleDeltaTransmissionBSDF(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    uint frontFace,
    inout uint seed
) {
    BsdfSample bsdfSample = invalidBsdfSample();
    float ior = materialIor(material);
    float eta = frontFace == 1u ? (1.0 / ior) : ior;
    float cosine = saturate(dot(normal, -incomingDirection));
    float reflectance = fresnelDielectric(cosine, ior);
    bool totalInternalReflection = !canRefractThroughNormal(incomingDirection, normal, eta);

    bsdfSample.bsdf = vec3(0.0);
    bsdfSample.valid = true;
    bsdfSample.eta = 1.0;

    if (totalInternalReflection || randomFloat(seed) < reflectance) {
        bsdfSample.direction = normalize(customReflect(incomingDirection, normal));
        bsdfSample.pdf = max(totalInternalReflection ? 1.0 : reflectance, EPSILON);
        bsdfSample.weight = vec3(1.0);
        bsdfSample.flags = BSDF_EVENT_DELTA | BSDF_EVENT_GLOSSY | BSDF_EVENT_REFLECTION;
    } else {
        bsdfSample.direction = normalize(customRefract(incomingDirection, normal, eta));
        bsdfSample.pdf = max(1.0 - reflectance, EPSILON);
        bsdfSample.weight = material.baseColor.rgb;
        bsdfSample.eta = eta;
        bsdfSample.flags = BSDF_EVENT_DELTA | BSDF_EVENT_TRANSMISSION;
    }

    return bsdfSample;
}

BsdfSample sampleDeltaMetalBSDF(MaterialData material, vec3 normal, vec3 incomingDirection) {
    BsdfSample bsdfSample = invalidBsdfSample();
    vec3 viewDirection = normalize(-incomingDirection);
    float nDotV = max(dot(normal, viewDirection), 0.0);

    bsdfSample.direction = normalize(customReflect(incomingDirection, normal));
    bsdfSample.bsdf = vec3(0.0);
    bsdfSample.weight = fresnelSchlick(specularF0(material), nDotV);
    bsdfSample.pdf = 1.0;
    bsdfSample.eta = 1.0;
    bsdfSample.flags = BSDF_EVENT_DELTA | BSDF_EVENT_GLOSSY | BSDF_EVENT_REFLECTION;
    bsdfSample.valid = dot(normal, bsdfSample.direction) > 0.0;
    return bsdfSample;
}

BsdfSample sampleSurfaceBSDF(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    uint frontFace,
    inout uint seed
) {
    BsdfSample bsdfSample = invalidBsdfSample();
    vec4 probabilities = bsdfSampleProbabilities(material, normal, incomingDirection, frontFace);
    float probabilitySum = probabilities.x + probabilities.y + probabilities.z + probabilities.w;
    if (probabilitySum <= EPSILON) {
        return bsdfSample;
    }

    float xi = randomFloat(seed);
    bool sampledDirectionIsValid = false;

    if (xi < probabilities.x) {
        sampledDirectionIsValid = sampleDiffuseDirection(normal, seed, bsdfSample.direction);
        bsdfSample.flags = BSDF_EVENT_DIFFUSE | BSDF_EVENT_REFLECTION;
    } else if (xi < probabilities.x + probabilities.y) {
        sampledDirectionIsValid = sampleMicrofacetReflectionDirection(
            material,
            normal,
            incomingDirection,
            seed,
            bsdfSample.direction
        );
        bsdfSample.flags = BSDF_EVENT_GLOSSY | BSDF_EVENT_REFLECTION;
    } else if (xi < probabilities.x + probabilities.y + probabilities.z) {
        sampledDirectionIsValid = sampleMicrofacetTransmissionDirection(
            material,
            normal,
            incomingDirection,
            frontFace,
            seed,
            bsdfSample.direction
        );
        bsdfSample.flags = BSDF_EVENT_GLOSSY | BSDF_EVENT_TRANSMISSION;
        bsdfSample.eta = frontFace == 1u ? (1.0 / materialIor(material)) : materialIor(material);
    } else {
        sampledDirectionIsValid = sampleClearcoatDirection(
            material,
            normal,
            incomingDirection,
            seed,
            bsdfSample.direction
        );
        bsdfSample.flags = BSDF_EVENT_GLOSSY | BSDF_EVENT_REFLECTION;
    }

    if (!sampledDirectionIsValid) {
        return bsdfSample;
    }

    float cosTheta = abs(dot(normal, bsdfSample.direction));
    bsdfSample.bsdf = evaluateBSDF(material, normal, incomingDirection, bsdfSample.direction, frontFace);
    bsdfSample.pdf = pdfBSDF(material, normal, incomingDirection, bsdfSample.direction, frontFace);
    bsdfSample.weight = bsdfSample.bsdf * cosTheta / max(bsdfSample.pdf, EPSILON);
    bsdfSample.valid = bsdfSample.pdf > 0.0
        && cosTheta > 0.0
        && maxComponent(bsdfSample.weight) > 0.0;
    return bsdfSample;
}

BsdfSample sampleBSDF(
    MaterialData material,
    vec3 normal,
    vec3 incomingDirection,
    uint frontFace,
    inout uint seed
) {
    if (isDeltaTransmissionMaterial(material)) {
        return sampleDeltaTransmissionBSDF(material, normal, incomingDirection, frontFace, seed);
    }

    if (isDeltaMetalMaterial(material)) {
        return sampleDeltaMetalBSDF(material, normal, incomingDirection);
    }

    return sampleSurfaceBSDF(material, normal, incomingDirection, frontFace, seed);
}

bool traceVisibility(vec3 position, vec3 normal, vec3 direction, float maxDistance) {
    payload.isMiss = 0u;
    payload.distance = 0.0;
    traceRayEXT(
        tlas,
        gl_RayFlagsTerminateOnFirstHitEXT | gl_RayFlagsSkipClosestHitShaderEXT,
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

float sphereLightPdf(vec3 surfacePosition, vec3 lightCenter, float lightRadius, vec3 lightPosition, vec3 lightNormal) {
    float pdfArea = 1.0 / max(4.0 * PI * lightRadius * lightRadius, EPSILON);
    vec3 toLight = lightPosition - surfacePosition;
    float distanceSquared = dot(toLight, toLight);
    vec3 lightDirection = normalize(toLight);
    float lightCosine = max(dot(lightNormal, -lightDirection), 0.0);

    if (lightCosine <= 0.0) {
        return 0.0;
    }

    return pdfArea * distanceSquared / lightCosine;
}
LightSample sampleEnvironmentLight(
    vec3 surfacePosition,
    vec3 surfaceNormal,
    int lightIndex,
    inout uint seed
) {
    LightSample lightSample = invalidLightSample();
    float selectionPmf = lightSelectionPmf(lightIndex);
    if (selectionPmf <= 0.0) {
        return lightSample;
    }

    int row = sampleEnvironmentMarginalRow(randomFloat(seed));
    int column = sampleEnvironmentConditionalColumn(row, randomFloat(seed));
    vec2 uv = vec2(
        (float(column) + randomFloat(seed)) / float(ENV_MAP_WIDTH),
        (float(row) + randomFloat(seed)) / float(ENV_MAP_HEIGHT)
    );

    lightSample.direction = environmentUvToDirection(uv);
    lightSample.distance = 1e4;
    lightSample.flags = LIGHT_FLAG_ENVIRONMENT;
    lightSample.pdf = selectionPmf * environmentPdfFromIndex(column, row);

    if (lightSample.pdf <= 0.0) {
        return lightSample;
    }

    if (!traceVisibility(surfacePosition, surfaceNormal, lightSample.direction, lightSample.distance)) {
        return lightSample;
    }

    lightSample.radiance = sampleEnvironment(lightSample.direction);
    lightSample.valid = maxComponent(lightSample.radiance) > 0.0;
    return lightSample;
}

LightSample samplePointLight(vec3 surfacePosition, vec3 surfaceNormal, int lightIndex) {
    LightSample lightSample = invalidLightSample();
    GpuLight light = getLight(lightIndex);
    vec3 toLight = light.positionRadius.xyz - surfacePosition;
    float distanceSquared = dot(toLight, toLight);
    lightSample.distance = sqrt(distanceSquared);
    lightSample.direction = toLight / max(lightSample.distance, EPSILON);
    lightSample.pdf = lightSelectionPmf(lightIndex);
    lightSample.flags = LIGHT_FLAG_DELTA | LIGHT_FLAG_FINITE;

    if (lightSample.pdf <= 0.0) {
        return lightSample;
    }

    if (!traceVisibility(
        surfacePosition,
        surfaceNormal,
        lightSample.direction,
        max(lightSample.distance - SHADOW_EPSILON, SHADOW_EPSILON)
    )) {
        return lightSample;
    }

    lightSample.radiance = light.emissionPmf.rgb / max(distanceSquared, EPSILON);
    lightSample.valid = true;
    return lightSample;
}

LightSample sampleDirectionalLight(vec3 surfacePosition, vec3 surfaceNormal, int lightIndex) {
    LightSample lightSample = invalidLightSample();
    GpuLight light = getLight(lightIndex);
    lightSample.direction = normalize(-light.directionType.xyz);
    lightSample.distance = 1e4;
    lightSample.pdf = lightSelectionPmf(lightIndex);
    lightSample.flags = LIGHT_FLAG_DELTA;

    if (lightSample.pdf <= 0.0) {
        return lightSample;
    }

    if (!traceVisibility(surfacePosition, surfaceNormal, lightSample.direction, lightSample.distance)) {
        return lightSample;
    }

    lightSample.radiance = light.emissionPmf.rgb;
    lightSample.valid = true;
    return lightSample;
}

LightSample sampleAreaSphereLight(
    vec3 surfacePosition,
    vec3 surfaceNormal,
    int lightIndex,
    inout uint seed
) {
    LightSample lightSample = invalidLightSample();
    GpuLight light = getLight(lightIndex);
    float radius = max(light.positionRadius.w, 0.01);
    vec3 lightNormal = sampleUniformSphere(seed);
    vec3 lightPosition = light.positionRadius.xyz + radius * lightNormal;

    vec3 toLight = lightPosition - surfacePosition;
    float distanceSquared = dot(toLight, toLight);
    lightSample.distance = sqrt(distanceSquared);
    lightSample.direction = toLight / max(lightSample.distance, EPSILON);
    lightSample.flags = LIGHT_FLAG_FINITE;
    lightSample.radiance = light.emissionPmf.rgb;

    float conditionalPdf = sphereLightPdf(
        surfacePosition,
        light.positionRadius.xyz,
        radius,
        lightPosition,
        lightNormal
    );
    lightSample.pdf = lightSelectionPmf(lightIndex) * conditionalPdf;

    if (lightSample.pdf <= 0.0) {
        return lightSample;
    }

    lightSample.valid = traceVisibility(
        surfacePosition,
        surfaceNormal,
        lightSample.direction,
        max(lightSample.distance - SHADOW_EPSILON, SHADOW_EPSILON)
    );
    return lightSample;
}

LightSample sampleLight(vec3 surfacePosition, vec3 surfaceNormal, inout uint seed) {
    int count = lightCount();
    if (count <= 0) {
        return invalidLightSample();
    }

    float xi = randomFloat(seed);
    int selectedLightIndex = count - 1;
    for (int index = 0; index < count; ++index) {
        if (xi <= lightSelectionCdf(index)) {
            selectedLightIndex = index;
            break;
        }
    }

    int selectedType = lightType(getLight(selectedLightIndex));
    if (selectedType == LIGHT_TYPE_ENVIRONMENT) {
        return sampleEnvironmentLight(surfacePosition, surfaceNormal, selectedLightIndex, seed);
    }
    if (selectedType == LIGHT_TYPE_POINT) {
        return samplePointLight(surfacePosition, surfaceNormal, selectedLightIndex);
    }
    if (selectedType == LIGHT_TYPE_DIRECTIONAL) {
        return sampleDirectionalLight(surfacePosition, surfaceNormal, selectedLightIndex);
    }
    if (selectedType == LIGHT_TYPE_AREA_SPHERE) {
        return sampleAreaSphereLight(surfacePosition, surfaceNormal, selectedLightIndex, seed);
    }

    return invalidLightSample();
}

float lightPdfForMiss(vec3 direction) {
    int environmentLightIndex = findLightByType(LIGHT_TYPE_ENVIRONMENT);
    if (environmentLightIndex < 0) {
        return 0.0;
    }

    return lightSelectionPmf(environmentLightIndex) * environmentPdf(direction);
}

float lightPdfForEmissiveHit(vec3 surfacePosition, vec3 lightPosition, vec3 lightNormal) {
    int areaLightIndex = findLightByType(LIGHT_TYPE_AREA_SPHERE);
    if (areaLightIndex < 0) {
        return 0.0;
    }

    GpuLight light = getLight(areaLightIndex);
    float conditionalPdf = sphereLightPdf(
        surfacePosition,
        light.positionRadius.xyz,
        max(light.positionRadius.w, 0.01),
        lightPosition,
        lightNormal
    );
    return lightSelectionPmf(areaLightIndex) * conditionalPdf;
}

vec3 evaluateDirectLighting(
    MaterialData material,
    vec3 position,
    vec3 normal,
    vec3 incomingDirection,
    uint frontFace,
    inout uint seed
) {
    if (isInvisibleMaterial(material)) {
        return vec3(0.0);
    }

    LightSample lightSample = sampleLight(position, normal, seed);
    if (!lightSample.valid) {
        return vec3(0.0);
    }

    vec3 bsdfValue = evaluateBSDF(material, normal, incomingDirection, lightSample.direction, frontFace);
    float cosTheta = abs(dot(normal, lightSample.direction));
    if (cosTheta <= 0.0 || maxComponent(bsdfValue) <= 0.0) {
        return vec3(0.0);
    }

    float bsdfPdf = pdfBSDF(material, normal, incomingDirection, lightSample.direction, frontFace);
    float weight = isDeltaLightSample(lightSample)
        ? 1.0
        : powerHeuristic(lightSample.pdf, bsdfPdf);

    return bsdfValue * lightSample.radiance * cosTheta * weight / max(lightSample.pdf, EPSILON);
}

void main() {
    uint randSeed = (gl_LaunchIDEXT.y * gl_LaunchSizeEXT.x + gl_LaunchIDEXT.x) ^ constants.seed;

    float u = (float(gl_LaunchIDEXT.x) + randomFloat(randSeed)) / float(gl_LaunchSizeEXT.x - 1);
    float v = (float(gl_LaunchIDEXT.y) + randomFloat(randSeed)) / float(gl_LaunchSizeEXT.y - 1);

    Ray ray = getRay(u, v, randSeed);
    vec3 throughput = vec3(1.0);
    vec3 radiance = vec3(0.0);

    float previousBsdfPdf = 1.0;
    bool previousBounceWasDelta = true;
    vec3 previousSurfacePosition = ray.origin;

    vec3 mediumStack[MAX_MEDIUM_DEPTH];
    int mediumStackSize = 0;

    const int maxDepth = 50;
    for (int depth = 0; depth < maxDepth; depth++) {
        traceRayEXT(
            tlas,
            gl_RayFlagsNoneEXT,
            0xFF,
            0, 0, 0,
            ray.origin,
            EPSILON,
            ray.direction,
            1e4,
            0
        );

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

        if (mediumStackSize > 0) {
            throughput *= exp(-mediumStack[mediumStackSize - 1] * payload.distance);
        }

        vec3 hitPosition = payload.position;
        vec3 surfaceNormal = payload.normal;
        uint materialIndex = payload.material;
        uint frontFace = payload.frontFace;
        MaterialData material = matBuffer.materials[materialIndex];

        if (maxComponent(material.emission.rgb) > 0.0) {
            vec3 emittedRadiance = material.emission.rgb;
            if (depth == 0 || previousBounceWasDelta || !hasVisibleEmissiveLight()) {
                radiance += throughput * emittedRadiance;
            } else {
                float lightPdf = lightPdfForEmissiveHit(previousSurfacePosition, hitPosition, surfaceNormal);
                float weight = lightPdf > 0.0
                    ? powerHeuristic(previousBsdfPdf, lightPdf)
                    : 1.0;
                radiance += throughput * emittedRadiance * weight;
            }
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

        previousSurfacePosition = hitPosition;
        previousBsdfPdf = bsdfSample.pdf;
        previousBounceWasDelta = isDeltaBsdfSample(bsdfSample);

        throughput *= bsdfSample.weight;

        bool isTransmissionEvent = (bsdfSample.flags & BSDF_EVENT_TRANSMISSION) != 0u;
        bool isReflectionEvent = (bsdfSample.flags & BSDF_EVENT_REFLECTION) != 0u;
        if (isTransmissionEvent && !isReflectionEvent) {
            if (frontFace == 1u) {
                if (mediumStackSize < MAX_MEDIUM_DEPTH) {
                    mediumStack[mediumStackSize] = materialAbsorption(material);
                    mediumStackSize++;
                }
            } else if (mediumStackSize > 0) {
                mediumStackSize--;
            }
        }

        ray.origin = offsetRayOrigin(hitPosition, surfaceNormal, bsdfSample.direction);
        ray.direction = bsdfSample.direction;

        if (depth >= 4) {
            float continuationProbability = clamp(maxComponent(throughput), 0.05, 0.95);
            if (randomFloat(randSeed) > continuationProbability) {
                break;
            }
            throughput /= continuationProbability;
        }
    }

    vec4 oldColor = imageLoad(outputImage, ivec2(gl_LaunchIDEXT.xy));
    imageStore(outputImage, ivec2(gl_LaunchIDEXT.xy), oldColor + vec4(radiance, 1.0));
}