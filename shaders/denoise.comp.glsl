#version 460

layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;

const float EPSILON = 1e-5;
const uint DENOISE_MODE_TEMPORAL = 0u;
const uint DENOISE_MODE_ATROUS = 1u;
const vec3 LUMINANCE_WEIGHTS = vec3(0.2126, 0.7152, 0.0722);

layout(binding = 0, set = 0, rgba32f) uniform readonly image2D currentNoisyImage;
layout(binding = 1, set = 0, rgba32f) uniform readonly image2D previousColorImage;
layout(binding = 2, set = 0, rgba32f) uniform readonly image2D previousPositionImage;
layout(binding = 3, set = 0, rgba32f) uniform readonly image2D previousNormalRoughnessImage;
layout(binding = 4, set = 0, rgba32f) uniform readonly image2D currentPositionImage;
layout(binding = 5, set = 0, rgba32f) uniform readonly image2D currentNormalRoughnessImage;
layout(binding = 6, set = 0, rgba32f) uniform readonly image2D previousMomentsImage;
layout(binding = 7, set = 0, rgba32f) uniform image2D currentMomentsImage;
layout(binding = 8, set = 0, rgba32f) uniform image2D filterPingImage;
layout(binding = 9, set = 0, rgba32f) uniform image2D filterPongImage;

layout(binding = 10, set = 0) readonly buffer FrameDataBuffer {
    vec4 origin;
    vec4 lowerLeftCorner;
    vec4 horizontal;
    vec4 vertical;
    vec4 basisULensRadius;
    vec4 basisVPadding;
} frameData;

layout(binding = 11, set = 0) readonly buffer PreviousFrameDataBuffer {
    vec4 origin;
    vec4 lowerLeftCorner;
    vec4 horizontal;
    vec4 vertical;
    vec4 basisULensRadius;
    vec4 basisVPadding;
} previousFrameData;

layout(push_constant) uniform PushConstants {
    uint mode;
    uint stepWidth;
    uint inputIsPing;
    uint _padding;
} constants;

struct NeighborhoodStats {
    vec3 mean;
    vec3 sigma;
    float lumaMean;
    float lumaVariance;
};

float luminance(vec3 color) {
    return dot(color, LUMINANCE_WEIGHTS);
}

vec3 frameCenter(
    vec4 origin,
    vec4 lowerLeftCorner,
    vec4 horizontal,
    vec4 vertical
) {
    return lowerLeftCorner.xyz + 0.5 * horizontal.xyz + 0.5 * vertical.xyz;
}

vec3 frameForward(
    vec4 origin,
    vec4 lowerLeftCorner,
    vec4 horizontal,
    vec4 vertical
) {
    return normalize(frameCenter(origin, lowerLeftCorner, horizontal, vertical) - origin.xyz);
}

bool projectWorldPositionToFrameUv(
    vec3 worldPosition,
    vec4 origin,
    vec4 lowerLeftCorner,
    vec4 horizontal,
    vec4 vertical,
    out vec2 uv
) {
    vec3 planeCenter = frameCenter(origin, lowerLeftCorner, horizontal, vertical);
    vec3 forward = normalize(planeCenter - origin.xyz);
    float focusDistance = length(planeCenter - origin.xyz);
    vec3 relative = worldPosition - origin.xyz;
    float depth = dot(relative, forward);
    if (depth <= EPSILON) {
        uv = vec2(-1.0);
        return false;
    }

    vec3 projectedPoint = origin.xyz + relative * (focusDistance / depth);
    vec3 fromLowerLeft = projectedPoint - lowerLeftCorner.xyz;
    float horizontalLengthSquared = max(dot(horizontal.xyz, horizontal.xyz), EPSILON);
    float verticalLengthSquared = max(dot(vertical.xyz, vertical.xyz), EPSILON);
    uv = vec2(
        dot(fromLowerLeft, horizontal.xyz) / horizontalLengthSquared,
        dot(fromLowerLeft, vertical.xyz) / verticalLengthSquared
    );
    return uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0;
}

bool reprojectWorldPositionToPreviousUv(vec3 worldPosition, out vec2 uv) {
    return projectWorldPositionToFrameUv(
        worldPosition,
        previousFrameData.origin,
        previousFrameData.lowerLeftCorner,
        previousFrameData.horizontal,
        previousFrameData.vertical,
        uv
    );
}

float temporalCameraStillness() {
    float translationDelta = length(frameData.origin.xyz - previousFrameData.origin.xyz);
    vec3 currentForward = frameForward(
        frameData.origin,
        frameData.lowerLeftCorner,
        frameData.horizontal,
        frameData.vertical
    );
    vec3 previousForward = frameForward(
        previousFrameData.origin,
        previousFrameData.lowerLeftCorner,
        previousFrameData.horizontal,
        previousFrameData.vertical
    );
    float rotationDelta = 1.0 - clamp(dot(currentForward, previousForward), 0.0, 1.0);
    return 1.0 - clamp(translationDelta * 4.0 + rotationDelta * 400.0, 0.0, 1.0);
}

float temporalMaxHistory(float roughness) {
    float historyLength = mix(12.0, 48.0, clamp(roughness, 0.0, 1.0));
    historyLength *= mix(0.5, 1.5, temporalCameraStillness());
    return clamp(historyLength, 2.0, 64.0);
}

ivec2 clampImageCoord(ivec2 coord, ivec2 imageSizePixels) {
    return clamp(coord, ivec2(0), imageSizePixels - 1);
}

ivec2 uvToImageCoordinate(vec2 uv, ivec2 imageSizePixels) {
    vec2 clampedUv = clamp(uv, vec2(0.0), vec2(0.999999));
    return clampImageCoord(ivec2(clampedUv * vec2(imageSizePixels)), imageSizePixels);
}

NeighborhoodStats computeNeighborhoodStats(ivec2 pixelCoord, ivec2 imageSizePixels) {
    NeighborhoodStats stats;
    vec3 mean = vec3(0.0);
    vec3 secondMoment = vec3(0.0);
    float lumaMean = 0.0;
    float lumaSecondMoment = 0.0;
    float sampleCount = 0.0;

    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            ivec2 sampleCoord = clampImageCoord(pixelCoord + ivec2(x, y), imageSizePixels);
            vec3 sampleColor = imageLoad(currentNoisyImage, sampleCoord).rgb;
            float sampleLuma = luminance(sampleColor);
            mean += sampleColor;
            secondMoment += sampleColor * sampleColor;
            lumaMean += sampleLuma;
            lumaSecondMoment += sampleLuma * sampleLuma;
            sampleCount += 1.0;
        }
    }

    mean /= sampleCount;
    secondMoment /= sampleCount;
    lumaMean /= sampleCount;
    lumaSecondMoment /= sampleCount;

    stats.mean = mean;
    stats.sigma = sqrt(max(secondMoment - mean * mean, vec3(0.0)));
    stats.lumaMean = lumaMean;
    stats.lumaVariance = max(lumaSecondMoment - lumaMean * lumaMean, 0.0);
    return stats;
}

float kernelWeight(int index) {
    if (index == 0 || index == 4) {
        return 1.0 / 16.0;
    }
    if (index == 1 || index == 3) {
        return 1.0 / 4.0;
    }
    return 3.0 / 8.0;
}

vec4 loadAtrousInput(ivec2 pixelCoord) {
    return constants.inputIsPing == 1u
        ? imageLoad(filterPingImage, pixelCoord)
        : imageLoad(filterPongImage, pixelCoord);
}

void storeAtrousOutput(ivec2 pixelCoord, vec4 value) {
    if (constants.inputIsPing == 1u) {
        imageStore(filterPongImage, pixelCoord, value);
    } else {
        imageStore(filterPingImage, pixelCoord, value);
    }
}

void runTemporalPass(ivec2 pixelCoord, ivec2 imageSizePixels) {
    vec3 currentColor = imageLoad(currentNoisyImage, pixelCoord).rgb;
    vec4 currentPositionData = imageLoad(currentPositionImage, pixelCoord);
    vec4 currentNormalRoughnessData = imageLoad(currentNormalRoughnessImage, pixelCoord);
    bool currentValid = currentPositionData.w > 0.5;
    float roughness = currentNormalRoughnessData.w;
    vec3 currentNormal = currentValid
        ? normalize(currentNormalRoughnessData.xyz)
        : vec3(0.0, 1.0, 0.0);

    NeighborhoodStats stats = computeNeighborhoodStats(pixelCoord, imageSizePixels);
    float currentLuma = luminance(currentColor);
    float historyLength = 1.0;
    vec3 temporalColor = currentColor;
    float filteredM1 = currentLuma;
    float filteredM2 = currentLuma * currentLuma;
    float filteredVariance = max(stats.lumaVariance, 1e-6);

    if (currentValid && previousFrameData.origin.w > 0.5) {
        vec2 previousUv;
        if (reprojectWorldPositionToPreviousUv(currentPositionData.xyz, previousUv)) {
            ivec2 previousCoord = uvToImageCoordinate(previousUv, imageSize(previousColorImage));
            vec4 previousPositionData = imageLoad(previousPositionImage, previousCoord);
            vec4 previousNormalRoughnessData = imageLoad(previousNormalRoughnessImage, previousCoord);

            if (previousPositionData.w > 0.5) {
                vec3 previousNormal = normalize(previousNormalRoughnessData.xyz);
                float viewDistance = length(currentPositionData.xyz - frameData.origin.xyz);
                float positionTolerance = max(0.02, mix(0.01, 0.03, roughness) * max(viewDistance, 1.0));
                float positionError = length(previousPositionData.xyz - currentPositionData.xyz);
                float normalThreshold = mix(0.98, 0.85, roughness);
                float normalAlignment = dot(previousNormal, currentNormal);

                if (positionError <= positionTolerance && normalAlignment >= normalThreshold) {
                    vec4 historyData = imageLoad(previousColorImage, previousCoord);
                    vec4 previousMoments = imageLoad(previousMomentsImage, previousCoord);
                    vec3 clampBias = vec3(0.05);
                    vec3 clampMin = stats.mean - 2.5 * stats.sigma - clampBias;
                    vec3 clampMax = stats.mean + 2.5 * stats.sigma + clampBias;
                    vec3 clampedHistory = clamp(historyData.rgb, clampMin, clampMax);

                    float maxHistory = temporalMaxHistory(roughness);
                    float previousHistoryLength = clamp(historyData.a, 1.0, maxHistory - 1.0);
                    historyLength = min(previousHistoryLength + 1.0, maxHistory);
                    float historyWeight = previousHistoryLength / historyLength;

                    temporalColor = mix(currentColor, clampedHistory, historyWeight);
                    filteredM1 = mix(currentLuma, previousMoments.x, historyWeight);
                    filteredM2 = mix(currentLuma * currentLuma, previousMoments.y, historyWeight);
                    filteredVariance = max(filteredM2 - filteredM1 * filteredM1, stats.lumaVariance * 0.25);
                }
            }
        }
    }

    imageStore(filterPingImage, pixelCoord, vec4(temporalColor, historyLength));
    imageStore(currentMomentsImage, pixelCoord, vec4(filteredM1, filteredM2, filteredVariance, 1.0));
}

void runAtrousPass(ivec2 pixelCoord, ivec2 imageSizePixels) {
    vec4 centerData = loadAtrousInput(pixelCoord);
    vec3 centerColor = centerData.rgb;
    float centerHistoryLength = centerData.a;
    vec4 centerPositionData = imageLoad(currentPositionImage, pixelCoord);
    vec4 centerNormalRoughnessData = imageLoad(currentNormalRoughnessImage, pixelCoord);
    bool centerValid = centerPositionData.w > 0.5;
    vec3 centerNormal = centerValid
        ? normalize(centerNormalRoughnessData.xyz)
        : vec3(0.0, 1.0, 0.0);
    float roughness = centerNormalRoughnessData.w;
    float centerVariance = max(imageLoad(currentMomentsImage, pixelCoord).z, 1e-6);
    float roughnessFilterStrength = smoothstep(0.18, 0.50, roughness);
    float historyFilterStrength = smoothstep(3.0, 10.0, centerHistoryLength);
    float varianceFilterStrength = smoothstep(0.002, 0.02, centerVariance);
    float filterStrength = 0.45
        * roughnessFilterStrength
        * mix(0.35, 1.0, historyFilterStrength)
        * mix(0.5, 1.0, varianceFilterStrength);
    float colorSigma = max(
        0.01,
        mix(0.015, 1.5 * sqrt(centerVariance) + 0.01, roughnessFilterStrength)
    );

    vec3 accumulatedColor = vec3(0.0);
    float accumulatedWeight = 0.0;

    for (int y = -2; y <= 2; y++) {
        for (int x = -2; x <= 2; x++) {
            ivec2 sampleCoord = clampImageCoord(
                pixelCoord + ivec2(x, y) * int(constants.stepWidth),
                imageSizePixels
            );
            vec4 sampleData = loadAtrousInput(sampleCoord);
            vec4 samplePositionData = imageLoad(currentPositionImage, sampleCoord);
            vec4 sampleNormalRoughnessData = imageLoad(currentNormalRoughnessImage, sampleCoord);
            bool sampleValid = samplePositionData.w > 0.5;

            float weight = kernelWeight(x + 2) * kernelWeight(y + 2);

            if (centerValid != sampleValid) {
                continue;
            }

            if (centerValid && sampleValid) {
                vec3 sampleNormal = normalize(sampleNormalRoughnessData.xyz);
                float viewDistance = length(centerPositionData.xyz - frameData.origin.xyz);
                float depthSigma = max(0.01, mix(0.01, 0.10, roughness) * max(viewDistance, 1.0));
                float depthWeight = exp(
                    -abs(dot(samplePositionData.xyz - centerPositionData.xyz, centerNormal))
                    / depthSigma
                );
                float normalExponent = mix(256.0, 24.0, roughness);
                float normalWeight = pow(max(dot(centerNormal, sampleNormal), 0.0), normalExponent);
                weight *= depthWeight * normalWeight;
            }

            float colorDelta = abs(luminance(sampleData.rgb) - luminance(centerColor));
            float colorWeight = exp(-colorDelta / colorSigma);
            weight *= colorWeight;

            accumulatedColor += sampleData.rgb * weight;
            accumulatedWeight += weight;
        }
    }

    vec3 filteredColor = accumulatedWeight > EPSILON
        ? accumulatedColor / accumulatedWeight
        : centerColor;
    vec3 finalColor = mix(centerColor, filteredColor, filterStrength);
    storeAtrousOutput(pixelCoord, vec4(finalColor, centerHistoryLength));
}

void main() {
    ivec2 pixelCoord = ivec2(gl_GlobalInvocationID.xy);
    ivec2 imageSizePixels = imageSize(currentPositionImage);
    if (pixelCoord.x >= imageSizePixels.x || pixelCoord.y >= imageSizePixels.y) {
        return;
    }

    if (constants.mode == DENOISE_MODE_TEMPORAL) {
        runTemporalPass(pixelCoord, imageSizePixels);
    } else {
        runAtrousPass(pixelCoord, imageSizePixels);
    }
}
