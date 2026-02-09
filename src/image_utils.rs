use ash::{vk, Device};
use bytemuck;
use std::fs::File;
use std::io::Write;

use crate::buffer::get_memory_type_index;

pub struct RenderTargetImage {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub view: vk::ImageView,
}

impl RenderTargetImage {
    pub fn new(
        device: &Device,
        width: u32,
        height: u32,
        format: vk::Format,
        device_memory_properties: vk::PhysicalDeviceMemoryProperties,
    ) -> Result<Self, vk::Result> {
        let image_create_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D::default().width(width).height(height).depth(1))
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::STORAGE
                    | vk::ImageUsageFlags::TRANSFER_SRC,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let image = unsafe { device.create_image(&image_create_info, None) }?;

        let mem_reqs = unsafe { device.get_image_memory_requirements(image) };
        let mem_alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(get_memory_type_index(
                device_memory_properties,
                mem_reqs.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            ));

        let memory = unsafe { device.allocate_memory(&mem_alloc_info, None) }?;
        unsafe { device.bind_image_memory(image, memory, 0) }?;

        let image_view_create_info = vk::ImageViewCreateInfo::default()
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image(image);

        let view = unsafe { device.create_image_view(&image_view_create_info, None) }?;

        Ok(Self { image, memory, view })
    }

    pub unsafe fn destroy(self, device: &Device) {
        unsafe {
            device.destroy_image_view(self.view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
        }
    }
}

pub fn transition_image_to_general(
    device: &Device,
    command_pool: vk::CommandPool,
    graphics_queue: vk::Queue,
    image: vk::Image,
) -> Result<(), vk::Result> {
    let command_buffer = {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_buffer_count(1)
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY);

        unsafe { device.allocate_command_buffers(&allocate_info) }?[0]
    };

    unsafe {
        device.begin_command_buffer(
            command_buffer,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )
    }?;

    let image_barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::empty())
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::GENERAL)
        .image(image)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1),
        );

    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[image_barrier],
        );

        device.end_command_buffer(command_buffer)?;
    }

    let command_buffers = [command_buffer];
    let submit_infos = [vk::SubmitInfo::default().command_buffers(&command_buffers)];

    unsafe {
        device
            .queue_submit(graphics_queue, &submit_infos, vk::Fence::null())
            .expect("Failed to execute queue submit.");

        device.queue_wait_idle(graphics_queue)?;
        device.free_command_buffers(command_pool, &[command_buffer]);
    }

    Ok(())
}

pub fn create_host_visible_image(
    device: &Device,
    width: u32,
    height: u32,
    format: vk::Format,
    device_memory_properties: vk::PhysicalDeviceMemoryProperties,
) -> Result<(vk::Image, vk::DeviceMemory), vk::Result> {
    let dst_image_create_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D::default().width(width).height(height).depth(1))
        .mip_levels(1)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::LINEAR)
        .usage(vk::ImageUsageFlags::TRANSFER_DST)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    let dst_image = unsafe { device.create_image(&dst_image_create_info, None) }?;

    let dst_mem_reqs = unsafe { device.get_image_memory_requirements(dst_image) };
    let dst_mem_alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(dst_mem_reqs.size)
        .memory_type_index(get_memory_type_index(
            device_memory_properties,
            dst_mem_reqs.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        ));

    let dst_device_memory = unsafe { device.allocate_memory(&dst_mem_alloc_info, None) }?;
    unsafe { device.bind_image_memory(dst_image, dst_device_memory, 0) }?;

    Ok((dst_image, dst_device_memory))
}

pub fn copy_image_to_host(
    device: &Device,
    command_pool: vk::CommandPool,
    graphics_queue: vk::Queue,
    src_image: vk::Image,
    dst_image: vk::Image,
    width: u32,
    height: u32,
) -> Result<(), vk::Result> {
    let copy_cmd = {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);

        unsafe { device.allocate_command_buffers(&allocate_info) }?[0]
    };

    let cmd_begin_info = vk::CommandBufferBeginInfo::default();
    unsafe { device.begin_command_buffer(copy_cmd, &cmd_begin_info) }?;

    let image_barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .image(dst_image)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1),
        );

    unsafe {
        device.cmd_pipeline_barrier(
            copy_cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[image_barrier],
        );
    }

    let copy_region = vk::ImageCopy::default()
        .src_subresource(
            vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .layer_count(1),
        )
        .dst_subresource(
            vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .layer_count(1),
        )
        .extent(vk::Extent3D::default().width(width).height(height).depth(1));

    unsafe {
        device.cmd_copy_image(
            copy_cmd,
            src_image,
            vk::ImageLayout::GENERAL,
            dst_image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[copy_region],
        );
    }

    let image_barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::MEMORY_READ)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::GENERAL)
        .image(dst_image)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1),
        );

    unsafe {
        device.cmd_pipeline_barrier(
            copy_cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[image_barrier],
        );

        device.end_command_buffer(copy_cmd)?;

        device
            .queue_submit(
                graphics_queue,
                &[vk::SubmitInfo::default().command_buffers(&[copy_cmd])],
                vk::Fence::null(),
            )
            .expect("Failed to execute queue submit.");

        device.queue_wait_idle(graphics_queue)?;
        device.free_command_buffers(command_pool, &[copy_cmd]);
    }

    Ok(())
}

pub fn save_image_to_png(
    device: &Device,
    dst_device_memory: vk::DeviceMemory,
    dst_image: vk::Image,
    width: u32,
    height: u32,
    n_samples: u32,
    filename: &str,
) {
    let subresource_layout = {
        let subresource = vk::ImageSubresource::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR);
        unsafe { device.get_image_subresource_layout(dst_image, subresource) }
    };

    let data: *const u8 = unsafe {
        device
            .map_memory(
                dst_device_memory,
                0,
                vk::WHOLE_SIZE,
                vk::MemoryMapFlags::empty(),
            )
            .unwrap() as _
    };

    let mut data = unsafe { data.offset(subresource_layout.offset as isize) };

    let mut png_encoder = png::Encoder::new(File::create(filename).unwrap(), width, height);
    png_encoder.set_depth(png::BitDepth::Eight);
    png_encoder.set_color(png::ColorType::Rgba);

    let mut png_writer = png_encoder
        .write_header()
        .unwrap()
        .into_stream_writer_with_size((4 * width) as usize)
        .unwrap();

    let scale = 1.0 / n_samples as f32;
    let gamma = 1.0 / 2.2_f32;

    let mut rows = Vec::new();
    for _ in 0..height {
        let row = unsafe { std::slice::from_raw_parts(data, 4 * 4 * width as usize) };
        let row_f32: &[f32] = bytemuck::cast_slice(row);
        let row_rgba8: Vec<u8> = row_f32
            .chunks(4)
            .flat_map(|pixel| {
                [
                    (256.0 * (pixel[0] * scale).powf(gamma).clamp(0.0, 0.999)) as u8,
                    (256.0 * (pixel[1] * scale).powf(gamma).clamp(0.0, 0.999)) as u8,
                    (256.0 * (pixel[2] * scale).powf(gamma).clamp(0.0, 0.999)) as u8,
                    255u8,
                ]
            })
            .collect();
        rows.push(row_rgba8);
        data = unsafe { data.offset(subresource_layout.row_pitch as isize) };
    }

    for row in rows.iter().rev() {
        png_writer.write_all(row).unwrap();
    }

    png_writer.finish().unwrap();

    unsafe {
        device.unmap_memory(dst_device_memory);
    }
}