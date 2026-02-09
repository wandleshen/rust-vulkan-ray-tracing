use ash::util::Align;
use ash::{vk, Device};

#[derive(Clone)]
pub struct BufferResource {
    pub buffer: vk::Buffer,
    pub memory: vk::DeviceMemory,
    pub size: vk::DeviceSize,
}

impl BufferResource {
    pub fn new(
        size: vk::DeviceSize,
        usage: vk::BufferUsageFlags,
        memory_properties: vk::MemoryPropertyFlags,
        device: &Device,
        device_memory_properties: vk::PhysicalDeviceMemoryProperties,
    ) -> Self {
        unsafe {
            let buffer_info = vk::BufferCreateInfo::default()
                .size(size)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);

            let buffer = device.create_buffer(&buffer_info, None).unwrap();

            let memory_req = device.get_buffer_memory_requirements(buffer);

            let memory_index = get_memory_type_index(
                device_memory_properties,
                memory_req.memory_type_bits,
                memory_properties,
            );

            let mut memory_allocate_flags_info = vk::MemoryAllocateFlagsInfo::default()
                .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);

            let mut allocate_info_default = vk::MemoryAllocateInfo::default();

            if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
                allocate_info_default =
                    allocate_info_default.push_next(&mut memory_allocate_flags_info);
            }

            let allocate_info = allocate_info_default
                .allocation_size(memory_req.size)
                .memory_type_index(memory_index);

            let memory = device.allocate_memory(&allocate_info, None).unwrap();

            device.bind_buffer_memory(buffer, memory, 0).unwrap();

            BufferResource {
                buffer,
                memory,
                size,
            }
        }
    }

    pub fn store<T: Copy>(&mut self, data: &[T], device: &Device) {
        unsafe {
            let size = (std::mem::size_of::<T>() * data.len()) as u64;
            assert!(self.size >= size);
            let mapped_ptr = self.map(size, device);
            let mut mapped_slice = Align::new(mapped_ptr, std::mem::align_of::<T>() as u64, size);
            mapped_slice.copy_from_slice(&data);
            self.unmap(device);
        }
    }

    fn map(&mut self, size: vk::DeviceSize, device: &Device) -> *mut std::ffi::c_void {
        unsafe {
            device
                .map_memory(self.memory, 0, size, vk::MemoryMapFlags::empty())
                .unwrap()
        }
    }

    fn unmap(&mut self, device: &Device) {
        unsafe {
            device.unmap_memory(self.memory);
        }
    }

    pub unsafe fn destroy(self, device: &Device) {
        unsafe {
            device.destroy_buffer(self.buffer, None);
            device.free_memory(self.memory, None);
        }
    }
}

pub fn get_memory_type_index(
    device_memory_properties: vk::PhysicalDeviceMemoryProperties,
    mut type_bits: u32,
    properties: vk::MemoryPropertyFlags,
) -> u32 {
    for i in 0..device_memory_properties.memory_type_count {
        if (type_bits & 1) == 1 {
            let memory_types = &device_memory_properties.memory_types;
            if (memory_types[i as usize].property_flags & properties) == properties {
                return i;
            }
        }
        type_bits >>= 1;
    }
    0
}

pub fn aligned_size(value: u32, alignment: u32) -> u32 {
    (value + alignment - 1) & !(alignment - 1)
}

pub unsafe fn get_buffer_device_address(device: &Device, buffer: vk::Buffer) -> u64 {
    unsafe {
        let buffer_device_address_info = vk::BufferDeviceAddressInfo::default().buffer(buffer);
        device.get_buffer_device_address(&buffer_device_address_info)
    }
}