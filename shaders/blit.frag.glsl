#version 460

layout(location = 0) in vec2 fragTexCoord;
layout(location = 0) out vec4 outColor;

layout(binding = 0, rgba32f) uniform readonly image2D accumulationImage;

layout(push_constant) uniform PushConstants {
    uint sampleCount;
} constants;

void main() {
    ivec2 texSize = imageSize(accumulationImage);
    ivec2 texCoord = ivec2(fragTexCoord * vec2(texSize));
    texCoord = clamp(texCoord, ivec2(0), texSize - 1);
    
    vec4 accumulated = imageLoad(accumulationImage, texCoord);
    vec3 color = accumulated.rgb / float(constants.sampleCount);
    
    outColor = vec4(color, 1.0);
}
