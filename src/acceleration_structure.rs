//! 加速结构（Acceleration Structure）构建模块
//! 
//! 包含 BLAS（Bottom-Level Acceleration Structure）和 TLAS（Top-Level Acceleration Structure）的构建逻辑

use ash::{khr, vk, Device};
use crate::buffer::{get_buffer_device_address, BufferResource};

/// 加速结构资源集合
pub struct AccelerationStructureResources {
    pub bottom_as: vk::AccelerationStructureKHR,
    pub bottom_as_buffer: BufferResource,
    pub aabb_buffer: BufferResource,
    pub top_as: vk::AccelerationStructureKHR,
    pub top_as_buffer: BufferResource,
    pub instance_buffer: BufferResource,
}

impl AccelerationStructureResources {
    /// 销毁所有加速结构资源
    pub unsafe fn destroy(
        self,
        device: &Device,
        acceleration_structure: &khr::acceleration_structure::Device,
    ) { unsafe {
        acceleration_structure.destroy_acceleration_structure(self.top_as, None);
        self.top_as_buffer.destroy(device);
        acceleration_structure.destroy_acceleration_structure(self.bottom_as, None);
        self.bottom_as_buffer.destroy(device);
        self.aabb_buffer.destroy(device);
        self.instance_buffer.destroy(device);
    }}
}

/// 创建 Bottom-Level 加速结构（球体 AABB）
pub fn create_bottom_level_as(
    device: &Device,
    acceleration_structure: &khr::acceleration_structure::Device,
    command_pool: vk::CommandPool,
    graphics_queue: vk::Queue,
    device_memory_properties: vk::PhysicalDeviceMemoryProperties,
) -> Result<(vk::AccelerationStructureKHR, BufferResource, BufferResource), vk::Result> {
    let aabb = vk::AabbPositionsKHR::default()
        .min_x(-1.0)
        .max_x(1.0)
        .min_y(-1.0)
        .max_y(1.0)
        .min_z(-1.0)
        .max_z(1.0);

    let mut aabb_buffer = BufferResource::new(
        std::mem::size_of::<vk::AabbPositionsKHR>() as u64,
        vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
            | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
        vk::MemoryPropertyFlags::HOST_VISIBLE
            | vk::MemoryPropertyFlags::HOST_COHERENT
            | vk::MemoryPropertyFlags::DEVICE_LOCAL,
        device,
        device_memory_properties,
    );

    aabb_buffer.store(&[aabb], device);

    let geometry = vk::AccelerationStructureGeometryKHR::default()
        .geometry_type(vk::GeometryTypeKHR::AABBS)
        .geometry(vk::AccelerationStructureGeometryDataKHR {
            aabbs: vk::AccelerationStructureGeometryAabbsDataKHR::default()
                .data(vk::DeviceOrHostAddressConstKHR {
                    device_address: unsafe {
                        get_buffer_device_address(device, aabb_buffer.buffer)
                    },
                })
                .stride(std::mem::size_of::<vk::AabbPositionsKHR>() as u64),
        })
        .flags(vk::GeometryFlagsKHR::OPAQUE);

    let build_range_info = vk::AccelerationStructureBuildRangeInfoKHR::default()
        .first_vertex(0)
        .primitive_count(1)
        .primitive_offset(0)
        .transform_offset(0);

    let geometries = [geometry];

    let mut build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
        .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
        .geometries(&geometries)
        .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
        .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);

    let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();
    unsafe {
        acceleration_structure.get_acceleration_structure_build_sizes(
            vk::AccelerationStructureBuildTypeKHR::DEVICE,
            &build_info,
            &[1],
            &mut size_info,
        )
    };

    let bottom_as_buffer = BufferResource::new(
        size_info.acceleration_structure_size,
        vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
            | vk::BufferUsageFlags::STORAGE_BUFFER,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
        device,
        device_memory_properties,
    );

    let as_create_info = vk::AccelerationStructureCreateInfoKHR::default()
        .ty(build_info.ty)
        .size(size_info.acceleration_structure_size)
        .buffer(bottom_as_buffer.buffer)
        .offset(0);

    let bottom_as =
        unsafe { acceleration_structure.create_acceleration_structure(&as_create_info, None) }?;

    build_info.dst_acceleration_structure = bottom_as;

    let scratch_buffer = BufferResource::new(
        size_info.build_scratch_size,
        vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::STORAGE_BUFFER,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
        device,
        device_memory_properties,
    );

    build_info.scratch_data = vk::DeviceOrHostAddressKHR {
        device_address: unsafe { get_buffer_device_address(device, scratch_buffer.buffer) },
    };

    let build_command_buffer = {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_buffer_count(1)
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY);

        unsafe { device.allocate_command_buffers(&allocate_info) }?[0]
    };

    unsafe {
        device.begin_command_buffer(
            build_command_buffer,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )?;

        let build_infos = [build_info];
        let build_range_infos: &[&[_]] = &[&[build_range_info]];

        acceleration_structure.cmd_build_acceleration_structures(
            build_command_buffer,
            &build_infos,
            build_range_infos,
        );
        device.end_command_buffer(build_command_buffer)?;
        device
            .queue_submit(
                graphics_queue,
                &[vk::SubmitInfo::default().command_buffers(&[build_command_buffer])],
                vk::Fence::null(),
            )
            .expect("queue submit failed.");

        device.queue_wait_idle(graphics_queue)?;
        device.free_command_buffers(command_pool, &[build_command_buffer]);
        scratch_buffer.destroy(device);
    }

    Ok((bottom_as, bottom_as_buffer, aabb_buffer))
}

/// 获取加速结构的设备地址
pub fn get_acceleration_structure_device_address(
    acceleration_structure: &khr::acceleration_structure::Device,
    as_handle: vk::AccelerationStructureKHR,
) -> u64 {
    let as_addr_info = vk::AccelerationStructureDeviceAddressInfoKHR::default()
        .acceleration_structure(as_handle);
    unsafe { acceleration_structure.get_acceleration_structure_device_address(&as_addr_info) }
}

/// 创建实例缓冲区
pub fn create_instance_buffer(
    device: &Device,
    instances: &[vk::AccelerationStructureInstanceKHR],
    device_memory_properties: vk::PhysicalDeviceMemoryProperties,
) -> BufferResource {
    let instance_buffer_size =
        std::mem::size_of::<vk::AccelerationStructureInstanceKHR>() * instances.len();

    let mut instance_buffer = BufferResource::new(
        instance_buffer_size as vk::DeviceSize,
        vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
            | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
        vk::MemoryPropertyFlags::HOST_VISIBLE
            | vk::MemoryPropertyFlags::HOST_COHERENT
            | vk::MemoryPropertyFlags::DEVICE_LOCAL,
        device,
        device_memory_properties,
    );

    instance_buffer.store(instances, device);
    instance_buffer
}

/// 创建 Top-Level 加速结构
pub fn create_top_level_as(
    device: &Device,
    acceleration_structure: &khr::acceleration_structure::Device,
    command_pool: vk::CommandPool,
    graphics_queue: vk::Queue,
    device_memory_properties: vk::PhysicalDeviceMemoryProperties,
    instance_buffer: &BufferResource,
    instance_count: u32,
) -> Result<(vk::AccelerationStructureKHR, BufferResource), vk::Result> {
    let build_range_info = vk::AccelerationStructureBuildRangeInfoKHR::default()
        .first_vertex(0)
        .primitive_count(instance_count)
        .primitive_offset(0)
        .transform_offset(0);

    let build_command_buffer = {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_buffer_count(1)
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY);

        unsafe { device.allocate_command_buffers(&allocate_info) }?[0]
    };

    unsafe {
        device.begin_command_buffer(
            build_command_buffer,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )?;
        let memory_barrier = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR);
        device.cmd_pipeline_barrier(
            build_command_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::DependencyFlags::empty(),
            &[memory_barrier],
            &[],
            &[],
        );
    }

    let instances = vk::AccelerationStructureGeometryInstancesDataKHR::default()
        .array_of_pointers(false)
        .data(vk::DeviceOrHostAddressConstKHR {
            device_address: unsafe {
                get_buffer_device_address(device, instance_buffer.buffer)
            },
        });

    let geometry = vk::AccelerationStructureGeometryKHR::default()
        .geometry_type(vk::GeometryTypeKHR::INSTANCES)
        .geometry(vk::AccelerationStructureGeometryDataKHR { instances });

    let geometries = [geometry];

    let mut build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
        .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
        .geometries(&geometries)
        .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
        .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL);

    let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();
    unsafe {
        acceleration_structure.get_acceleration_structure_build_sizes(
            vk::AccelerationStructureBuildTypeKHR::DEVICE,
            &build_info,
            &[build_range_info.primitive_count],
            &mut size_info,
        )
    };

    let top_as_buffer = BufferResource::new(
        size_info.acceleration_structure_size,
        vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
            | vk::BufferUsageFlags::STORAGE_BUFFER,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
        device,
        device_memory_properties,
    );

    let as_create_info = vk::AccelerationStructureCreateInfoKHR::default()
        .ty(build_info.ty)
        .size(size_info.acceleration_structure_size)
        .buffer(top_as_buffer.buffer)
        .offset(0);

    let top_as =
        unsafe { acceleration_structure.create_acceleration_structure(&as_create_info, None) }?;

    build_info.dst_acceleration_structure = top_as;

    let scratch_buffer = BufferResource::new(
        size_info.build_scratch_size,
        vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::STORAGE_BUFFER,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
        device,
        device_memory_properties,
    );

    build_info.scratch_data = vk::DeviceOrHostAddressKHR {
        device_address: unsafe { get_buffer_device_address(device, scratch_buffer.buffer) },
    };

    unsafe {
        let build_infos = [build_info];
        let build_range_infos: &[&[_]] = &[&[build_range_info]];
        acceleration_structure.cmd_build_acceleration_structures(
            build_command_buffer,
            &build_infos,
            build_range_infos,
        );
        device.end_command_buffer(build_command_buffer)?;
        device
            .queue_submit(
                graphics_queue,
                &[vk::SubmitInfo::default().command_buffers(&[build_command_buffer])],
                vk::Fence::null(),
            )
            .expect("queue submit failed.");

        device.queue_wait_idle(graphics_queue)?;
        device.free_command_buffers(command_pool, &[build_command_buffer]);
        scratch_buffer.destroy(device);
    }

    Ok((top_as, top_as_buffer))
}
