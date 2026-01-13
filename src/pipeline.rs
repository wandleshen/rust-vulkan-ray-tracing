//! Ray Tracing 管线模块
//!
//! 包含 RT 管线创建、Descriptor Set 配置、Shader Binding Table 构建

use ash::{khr, vk, Device};
use std::ffi::CStr;

use crate::buffer::{aligned_size, get_buffer_device_address, BufferResource};
use crate::vulkan_base::create_shader_module;

/// Ray Tracing 管线资源
pub struct RayTracingPipelineResources {
    pub pipeline: vk::Pipeline,
    pub pipeline_layout: vk::PipelineLayout,
    pub descriptor_set_layout: vk::DescriptorSetLayout,
    pub descriptor_pool: vk::DescriptorPool,
    pub descriptor_set: vk::DescriptorSet,
    pub shader_binding_table_buffer: BufferResource,
    pub sbt_raygen_region: vk::StridedDeviceAddressRegionKHR,
    pub sbt_miss_region: vk::StridedDeviceAddressRegionKHR,
    pub sbt_hit_region: vk::StridedDeviceAddressRegionKHR,
    pub sbt_call_region: vk::StridedDeviceAddressRegionKHR,
}

impl RayTracingPipelineResources {
    /// 销毁管线资源
    pub unsafe fn destroy(self, device: &Device) { unsafe {
        device.destroy_descriptor_pool(self.descriptor_pool, None);
        self.shader_binding_table_buffer.destroy(device);
        device.destroy_pipeline(self.pipeline, None);
        device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
        device.destroy_pipeline_layout(self.pipeline_layout, None);
    }}
}

/// 创建 Descriptor Set Layout
pub fn create_descriptor_set_layout(device: &Device) -> Result<vk::DescriptorSetLayout, vk::Result> {
    let bindings = [
        vk::DescriptorSetLayoutBinding::default()
            .descriptor_count(1)
            .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .stage_flags(vk::ShaderStageFlags::RAYGEN_KHR)
            .binding(0),
        vk::DescriptorSetLayoutBinding::default()
            .descriptor_count(1)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .stage_flags(vk::ShaderStageFlags::RAYGEN_KHR)
            .binding(1),
        vk::DescriptorSetLayoutBinding::default()
            .descriptor_count(1)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .stage_flags(vk::ShaderStageFlags::RAYGEN_KHR)
            .binding(2),
    ];

    unsafe {
        device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
            None,
        )
    }
}

/// 创建 Ray Tracing 管线
pub fn create_ray_tracing_pipeline(
    device: &Device,
    rt_pipeline: &khr::ray_tracing_pipeline::Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<(vk::Pipeline, vk::PipelineLayout, usize), vk::Result> {
    let push_constant_range = vk::PushConstantRange::default()
        .offset(0)
        .size(4)
        .stage_flags(vk::ShaderStageFlags::RAYGEN_KHR);

    // 加载编译好的 SPIR-V shader 文件
    const RAYGEN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/raygen.rgen.spv"));
    const MISS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/miss.rmiss.spv"));
    const CLOSESTHIT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/closesthit.rchit.spv"));
    const INTERSECTION: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/intersection.rint.spv"));

    let raygen_module = unsafe { create_shader_module(device, RAYGEN)? };
    let miss_module = unsafe { create_shader_module(device, MISS)? };
    let closesthit_module = unsafe { create_shader_module(device, CLOSESTHIT)? };
    let intersection_module = unsafe { create_shader_module(device, INTERSECTION)? };

    let layouts = [descriptor_set_layout];
    let pipeline_layout = unsafe {
        device.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&layouts)
                .push_constant_ranges(&[push_constant_range]),
            None,
        )
    }?;

    let shader_groups = vec![
        // group0 = [ raygen ]
        vk::RayTracingShaderGroupCreateInfoKHR::default()
            .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
            .general_shader(0)
            .closest_hit_shader(vk::SHADER_UNUSED_KHR)
            .any_hit_shader(vk::SHADER_UNUSED_KHR)
            .intersection_shader(vk::SHADER_UNUSED_KHR),
        // group1 = [ miss ]
        vk::RayTracingShaderGroupCreateInfoKHR::default()
            .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
            .general_shader(1)
            .closest_hit_shader(vk::SHADER_UNUSED_KHR)
            .any_hit_shader(vk::SHADER_UNUSED_KHR)
            .intersection_shader(vk::SHADER_UNUSED_KHR),
        // group2 = [ chit + intersection ]
        vk::RayTracingShaderGroupCreateInfoKHR::default()
            .ty(vk::RayTracingShaderGroupTypeKHR::PROCEDURAL_HIT_GROUP)
            .general_shader(vk::SHADER_UNUSED_KHR)
            .closest_hit_shader(3)
            .any_hit_shader(vk::SHADER_UNUSED_KHR)
            .intersection_shader(2),
    ];

    let entry_point = unsafe { CStr::from_bytes_with_nul_unchecked(b"main\0") };

    let shader_stages = vec![
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::RAYGEN_KHR)
            .module(raygen_module)
            .name(entry_point),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::MISS_KHR)
            .module(miss_module)
            .name(entry_point),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::INTERSECTION_KHR)
            .module(intersection_module)
            .name(entry_point),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::CLOSEST_HIT_KHR)
            .module(closesthit_module)
            .name(entry_point),
    ];

    let shader_groups_len = shader_groups.len();

    let pipeline = unsafe {
        rt_pipeline.create_ray_tracing_pipelines(
            vk::DeferredOperationKHR::null(),
            vk::PipelineCache::null(),
            &[vk::RayTracingPipelineCreateInfoKHR::default()
                .stages(&shader_stages)
                .groups(&shader_groups)
                .max_pipeline_ray_recursion_depth(1)
                .layout(pipeline_layout)],
            None,
        )
    }
    .expect("Failed to create ray tracing pipeline")[0];

    unsafe {
        device.destroy_shader_module(raygen_module, None);
        device.destroy_shader_module(miss_module, None);
        device.destroy_shader_module(closesthit_module, None);
        device.destroy_shader_module(intersection_module, None);
    }

    Ok((pipeline, pipeline_layout, shader_groups_len))
}

/// 创建 Descriptor Pool 和 Descriptor Set
pub fn create_descriptor_pool_and_set(
    device: &Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<(vk::DescriptorPool, vk::DescriptorSet), vk::Result> {
    let descriptor_sizes = [
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
            descriptor_count: 1,
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_IMAGE,
            descriptor_count: 1,
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_BUFFER,
            descriptor_count: 1,
        },
    ];

    let descriptor_pool_info = vk::DescriptorPoolCreateInfo::default()
        .pool_sizes(&descriptor_sizes)
        .max_sets(1);

    let descriptor_pool = unsafe { device.create_descriptor_pool(&descriptor_pool_info, None) }?;

    let descriptor_counts = [1];

    let mut count_allocate_info = vk::DescriptorSetVariableDescriptorCountAllocateInfo::default()
        .descriptor_counts(&descriptor_counts);

    let descriptor_sets = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(descriptor_pool)
                .set_layouts(&[descriptor_set_layout])
                .push_next(&mut count_allocate_info),
        )
    }?;

    Ok((descriptor_pool, descriptor_sets[0]))
}

/// 更新 Descriptor Set
pub fn update_descriptor_set(
    device: &Device,
    descriptor_set: vk::DescriptorSet,
    top_as: vk::AccelerationStructureKHR,
    image_view: vk::ImageView,
    material_buffer: vk::Buffer,
) {
    let accel_structs = [top_as];
    let mut accel_info = vk::WriteDescriptorSetAccelerationStructureKHR::default()
        .acceleration_structures(&accel_structs);

    let mut accel_write = vk::WriteDescriptorSet::default()
        .dst_set(descriptor_set)
        .dst_binding(0)
        .dst_array_element(0)
        .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
        .push_next(&mut accel_info);
    accel_write.descriptor_count = 1;

    let image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::GENERAL)
        .image_view(image_view)];

    let image_write = vk::WriteDescriptorSet::default()
        .dst_set(descriptor_set)
        .dst_binding(1)
        .dst_array_element(0)
        .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
        .image_info(&image_info);

    let buffer_info = [vk::DescriptorBufferInfo::default()
        .buffer(material_buffer)
        .range(vk::WHOLE_SIZE)];

    let buffers_write = vk::WriteDescriptorSet::default()
        .dst_set(descriptor_set)
        .dst_binding(2)
        .dst_array_element(0)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .buffer_info(&buffer_info);

    unsafe {
        device.update_descriptor_sets(&[accel_write, image_write, buffers_write], &[]);
    }
}

/// 创建 Shader Binding Table
pub fn create_shader_binding_table(
    device: &Device,
    rt_pipeline: &khr::ray_tracing_pipeline::Device,
    pipeline: vk::Pipeline,
    rt_pipeline_properties: &vk::PhysicalDeviceRayTracingPipelinePropertiesKHR,
    shader_groups_len: usize,
    device_memory_properties: vk::PhysicalDeviceMemoryProperties,
) -> Result<
    (
        BufferResource,
        vk::StridedDeviceAddressRegionKHR,
        vk::StridedDeviceAddressRegionKHR,
        vk::StridedDeviceAddressRegionKHR,
        vk::StridedDeviceAddressRegionKHR,
    ),
    vk::Result,
> {
    let incoming_table_data = unsafe {
        rt_pipeline.get_ray_tracing_shader_group_handles(
            pipeline,
            0,
            shader_groups_len as u32,
            shader_groups_len * rt_pipeline_properties.shader_group_handle_size as usize,
        )
    }?;

    let handle_size_aligned = aligned_size(
        rt_pipeline_properties.shader_group_handle_size,
        rt_pipeline_properties.shader_group_base_alignment,
    );

    let table_size = shader_groups_len * handle_size_aligned as usize;
    let mut table_data = vec![0u8; table_size];

    for i in 0..shader_groups_len {
        table_data[i * handle_size_aligned as usize
            ..i * handle_size_aligned as usize
                + rt_pipeline_properties.shader_group_handle_size as usize]
            .copy_from_slice(
                &incoming_table_data[i * rt_pipeline_properties.shader_group_handle_size as usize
                    ..i * rt_pipeline_properties.shader_group_handle_size as usize
                        + rt_pipeline_properties.shader_group_handle_size as usize],
            );
    }

    let mut shader_binding_table_buffer = BufferResource::new(
        table_size as u64,
        vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
            | vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR
            | vk::BufferUsageFlags::STORAGE_BUFFER,
        vk::MemoryPropertyFlags::HOST_VISIBLE
            | vk::MemoryPropertyFlags::HOST_COHERENT
            | vk::MemoryPropertyFlags::DEVICE_LOCAL,
        device,
        device_memory_properties,
    );

    shader_binding_table_buffer.store(&table_data, device);

    let handle_size_aligned = handle_size_aligned as u64;
    let sbt_address =
        unsafe { get_buffer_device_address(device, shader_binding_table_buffer.buffer) };

    let sbt_raygen_region = vk::StridedDeviceAddressRegionKHR::default()
        .device_address(sbt_address)
        .size(handle_size_aligned)
        .stride(handle_size_aligned);

    let sbt_miss_region = vk::StridedDeviceAddressRegionKHR::default()
        .device_address(sbt_address + handle_size_aligned)
        .size(handle_size_aligned)
        .stride(handle_size_aligned);

    let sbt_hit_region = vk::StridedDeviceAddressRegionKHR::default()
        .device_address(sbt_address + 2 * handle_size_aligned)
        .size(handle_size_aligned)
        .stride(handle_size_aligned);

    let sbt_call_region = vk::StridedDeviceAddressRegionKHR::default();

    Ok((
        shader_binding_table_buffer,
        sbt_raygen_region,
        sbt_miss_region,
        sbt_hit_region,
        sbt_call_region,
    ))
}

/// 创建材质缓冲区
pub fn create_material_buffer<T: Copy>(
    device: &Device,
    materials: &[T],
    device_memory_properties: vk::PhysicalDeviceMemoryProperties,
) -> BufferResource {
    let buffer_size = (materials.len() * std::mem::size_of::<T>()) as vk::DeviceSize;

    let mut material_buffer = BufferResource::new(
        buffer_size,
        vk::BufferUsageFlags::STORAGE_BUFFER,
        vk::MemoryPropertyFlags::HOST_VISIBLE
            | vk::MemoryPropertyFlags::HOST_COHERENT
            | vk::MemoryPropertyFlags::DEVICE_LOCAL,
        device,
        device_memory_properties,
    );
    material_buffer.store(materials, device);

    material_buffer
}
