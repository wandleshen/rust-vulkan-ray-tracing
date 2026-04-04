use ash::{Device, vk};

use crate::vulkan_base::create_shader_module;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct DenoisePushConstants {
    pub mode: u32,
    pub step_width: u32,
    pub input_is_ping: u32,
    pub _padding: u32,
}

pub struct DenoisePipelineResources {
    pub pipeline: vk::Pipeline,
    pub pipeline_layout: vk::PipelineLayout,
    pub descriptor_set_layout: vk::DescriptorSetLayout,
    pub descriptor_pool: vk::DescriptorPool,
    pub descriptor_set: vk::DescriptorSet,
}

impl DenoisePipelineResources {
    pub unsafe fn destroy(self, device: &Device) {
        unsafe {
            device.destroy_descriptor_pool(self.descriptor_pool, None);
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            device.destroy_pipeline_layout(self.pipeline_layout, None);
        }
    }
}

pub fn create_denoise_descriptor_set_layout(
    device: &Device,
) -> Result<vk::DescriptorSetLayout, vk::Result> {
    let image_binding = |binding| {
        vk::DescriptorSetLayoutBinding::default()
            .descriptor_count(1)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .binding(binding)
    };

    let buffer_binding = |binding| {
        vk::DescriptorSetLayoutBinding::default()
            .descriptor_count(1)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .binding(binding)
    };

    let bindings = [
        image_binding(0),
        image_binding(1),
        image_binding(2),
        image_binding(3),
        image_binding(4),
        image_binding(5),
        image_binding(6),
        image_binding(7),
        image_binding(8),
        image_binding(9),
        buffer_binding(10),
        buffer_binding(11),
    ];

    unsafe {
        device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
            None,
        )
    }
}

pub fn create_denoise_descriptor_pool_and_set(
    device: &Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<(vk::DescriptorPool, vk::DescriptorSet), vk::Result> {
    let descriptor_sizes = [
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_IMAGE,
            descriptor_count: 10,
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_BUFFER,
            descriptor_count: 2,
        },
    ];

    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .pool_sizes(&descriptor_sizes)
        .max_sets(1);

    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }?;

    let layouts = [descriptor_set_layout];
    let descriptor_sets = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(descriptor_pool)
                .set_layouts(&layouts),
        )
    }?;

    Ok((descriptor_pool, descriptor_sets[0]))
}

pub fn update_denoise_descriptor_set(
    device: &Device,
    descriptor_set: vk::DescriptorSet,
    current_noisy_image_view: vk::ImageView,
    previous_color_image_view: vk::ImageView,
    previous_position_image_view: vk::ImageView,
    previous_normal_roughness_image_view: vk::ImageView,
    current_position_image_view: vk::ImageView,
    current_normal_roughness_image_view: vk::ImageView,
    previous_moments_image_view: vk::ImageView,
    current_moments_image_view: vk::ImageView,
    filter_ping_image_view: vk::ImageView,
    filter_pong_image_view: vk::ImageView,
    frame_uniform_buffer: vk::Buffer,
    previous_frame_uniform_buffer: vk::Buffer,
) {
    let current_noisy_image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(current_noisy_image_view)];
    let previous_color_image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(previous_color_image_view)];
    let previous_position_image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(previous_position_image_view)];
    let previous_normal_roughness_image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(previous_normal_roughness_image_view)];
    let current_position_image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(current_position_image_view)];
    let current_normal_roughness_image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(current_normal_roughness_image_view)];
    let previous_moments_image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(previous_moments_image_view)];
    let current_moments_image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(current_moments_image_view)];
    let filter_ping_image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(filter_ping_image_view)];
    let filter_pong_image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(filter_pong_image_view)];

    let frame_buffer_info = [vk::DescriptorBufferInfo::default()
        .buffer(frame_uniform_buffer)
        .range(vk::WHOLE_SIZE)];
    let previous_frame_buffer_info = [vk::DescriptorBufferInfo::default()
        .buffer(previous_frame_uniform_buffer)
        .range(vk::WHOLE_SIZE)];

    let writes = [
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&current_noisy_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&previous_color_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&previous_position_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(3)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&previous_normal_roughness_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(4)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&current_position_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(5)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&current_normal_roughness_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(6)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&previous_moments_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(7)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&current_moments_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(8)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&filter_ping_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(9)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&filter_pong_image_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(10)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&frame_buffer_info),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(11)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&previous_frame_buffer_info),
    ];

    unsafe {
        device.update_descriptor_sets(&writes, &[]);
    }
}

pub fn create_denoise_pipeline(
    device: &Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<(vk::Pipeline, vk::PipelineLayout), vk::Result> {
    let compute_code = include_bytes!(concat!(env!("OUT_DIR"), "/denoise.comp.spv"));
    let compute_module = unsafe { create_shader_module(device, compute_code) }?;

    let push_constant_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::COMPUTE)
        .offset(0)
        .size(std::mem::size_of::<DenoisePushConstants>() as u32);

    let layouts = [descriptor_set_layout];
    let push_constant_ranges = [push_constant_range];
    let pipeline_layout = unsafe {
        device.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&layouts)
                .push_constant_ranges(&push_constant_ranges),
            None,
        )
    }?;

    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(compute_module)
        .name(c"main");

    let pipeline = unsafe {
        device.create_compute_pipelines(
            vk::PipelineCache::null(),
            &[vk::ComputePipelineCreateInfo::default()
                .stage(stage)
                .layout(pipeline_layout)],
            None,
        )
    }
    .map_err(|(_, err)| err)?[0];

    unsafe {
        device.destroy_shader_module(compute_module, None);
    }

    Ok((pipeline, pipeline_layout))
}

pub fn push_constants_bytes(push_constants: &DenoisePushConstants) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            push_constants as *const DenoisePushConstants as *const u8,
            std::mem::size_of::<DenoisePushConstants>(),
        )
    }
}
