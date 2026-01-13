use ash::{khr, vk};
use std::ffi::CStr;

pub struct Swapchain {
    pub swapchain: vk::SwapchainKHR,
    pub images: Vec<vk::Image>,
    pub image_views: Vec<vk::ImageView>,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    pub loader: khr::swapchain::Device,
}

impl Swapchain {
    pub fn new(
        instance: &ash::Instance,
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        surface_loader: &khr::surface::Instance,
        width: u32,
        height: u32,
    ) -> Self {
        let surface_capabilities = unsafe {
            surface_loader
                .get_physical_device_surface_capabilities(physical_device, surface)
                .unwrap()
        };

        let surface_formats = unsafe {
            surface_loader
                .get_physical_device_surface_formats(physical_device, surface)
                .unwrap()
        };

        let surface_format = surface_formats
            .iter()
            .find(|f| {
                f.format == vk::Format::B8G8R8A8_SRGB
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .unwrap_or(&surface_formats[0]);

        let present_modes = unsafe {
            surface_loader
                .get_physical_device_surface_present_modes(physical_device, surface)
                .unwrap()
        };

        let present_mode = present_modes
            .iter()
            .find(|&&m| m == vk::PresentModeKHR::MAILBOX)
            .unwrap_or(&vk::PresentModeKHR::FIFO);

        let image_count = (surface_capabilities.min_image_count + 1)
            .min(surface_capabilities.max_image_count.max(surface_capabilities.min_image_count + 1));

        let extent = if surface_capabilities.current_extent.width != u32::MAX {
            surface_capabilities.current_extent
        } else {
            vk::Extent2D {
                width: width.clamp(
                    surface_capabilities.min_image_extent.width,
                    surface_capabilities.max_image_extent.width,
                ),
                height: height.clamp(
                    surface_capabilities.min_image_extent.height,
                    surface_capabilities.max_image_extent.height,
                ),
            }
        };

        let swapchain_loader = khr::swapchain::Device::new(instance, device);

        let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(surface_capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(*present_mode)
            .clipped(true);

        let swapchain = unsafe {
            swapchain_loader
                .create_swapchain(&swapchain_create_info, None)
                .unwrap()
        };

        let images = unsafe { swapchain_loader.get_swapchain_images(swapchain).unwrap() };

        let image_views: Vec<vk::ImageView> = images
            .iter()
            .map(|&image| {
                let view_info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(surface_format.format)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    });
                unsafe { device.create_image_view(&view_info, None).unwrap() }
            })
            .collect();

        Self {
            swapchain,
            images,
            image_views,
            format: surface_format.format,
            extent,
            loader: swapchain_loader,
        }
    }

    pub fn destroy(&self, device: &ash::Device) {
        unsafe {
            for &view in &self.image_views {
                device.destroy_image_view(view, None);
            }
            self.loader.destroy_swapchain(self.swapchain, None);
        }
    }
}

pub fn check_surface_support(
    surface_loader: &khr::surface::Instance,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    surface: vk::SurfaceKHR,
) -> bool {
    unsafe {
        surface_loader
            .get_physical_device_surface_support(physical_device, queue_family_index, surface)
            .unwrap_or(false)
    }
}

pub fn create_blit_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> (vk::Pipeline, vk::PipelineLayout) {
    let vert_code = include_bytes!(concat!(env!("OUT_DIR"), "/blit.vert.spv"));
    let frag_code = include_bytes!(concat!(env!("OUT_DIR"), "/blit.frag.spv"));

    let vert_module = create_shader_module(device, vert_code);
    let frag_module = create_shader_module(device, frag_code);

    let entry_name = unsafe { CStr::from_bytes_with_nul_unchecked(b"main\0") };

    let shader_stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vert_module)
            .name(entry_name),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(frag_module)
            .name(entry_name),
    ];

    let vertex_input_info = vk::PipelineVertexInputStateCreateInfo::default();

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .line_width(1.0)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::CLOCKWISE);

    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA);

    let color_blend_attachments = [color_blend_attachment];
    let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
        .attachments(&color_blend_attachments);

    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let push_constant_range = vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        .offset(0)
        .size(4); // uint sampleCount

    let push_constant_ranges = [push_constant_range];
    let set_layouts = [descriptor_set_layout];

    let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&set_layouts)
        .push_constant_ranges(&push_constant_ranges);

    let pipeline_layout = unsafe {
        device
            .create_pipeline_layout(&pipeline_layout_info, None)
            .unwrap()
    };

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&shader_stages)
        .vertex_input_state(&vertex_input_info)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisampling)
        .color_blend_state(&color_blending)
        .dynamic_state(&dynamic_state)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    let pipeline = unsafe {
        device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
            .unwrap()[0]
    };

    unsafe {
        device.destroy_shader_module(vert_module, None);
        device.destroy_shader_module(frag_module, None);
    }

    (pipeline, pipeline_layout)
}

fn create_shader_module(device: &ash::Device, code: &[u8]) -> vk::ShaderModule {
    let code_u32: Vec<u32> = code
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let create_info = vk::ShaderModuleCreateInfo::default().code(&code_u32);

    unsafe { device.create_shader_module(&create_info, None).unwrap() }
}

pub fn create_render_pass(device: &ash::Device, format: vk::Format) -> vk::RenderPass {
    let attachment = vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

    let attachment_ref = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

    let attachment_refs = [attachment_ref];
    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&attachment_refs);

    let dependency = vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);

    let attachments = [attachment];
    let subpasses = [subpass];
    let dependencies = [dependency];

    let render_pass_info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpasses)
        .dependencies(&dependencies);

    unsafe { device.create_render_pass(&render_pass_info, None).unwrap() }
}

pub fn create_framebuffers(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    image_views: &[vk::ImageView],
    extent: vk::Extent2D,
) -> Vec<vk::Framebuffer> {
    image_views
        .iter()
        .map(|&view| {
            let attachments = [view];
            let framebuffer_info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(&attachments)
                .width(extent.width)
                .height(extent.height)
                .layers(1);

            unsafe { device.create_framebuffer(&framebuffer_info, None).unwrap() }
        })
        .collect()
}

/// 窗口模式资源
pub struct WindowedResources {
    pub swapchain: Swapchain,
    pub render_pass: vk::RenderPass,
    pub framebuffers: Vec<vk::Framebuffer>,
    pub blit_pipeline: vk::Pipeline,
    pub blit_pipeline_layout: vk::PipelineLayout,
    pub blit_descriptor_set_layout: vk::DescriptorSetLayout,
    pub blit_descriptor_pool: vk::DescriptorPool,
    pub blit_descriptor_set: vk::DescriptorSet,
    pub image_available_semaphore: vk::Semaphore,
    pub render_finished_semaphore: vk::Semaphore,
    pub in_flight_fence: vk::Fence,
}

impl WindowedResources {
    /// 创建窗口模式所需的所有资源
    pub fn new(
        instance: &ash::Instance,
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        surface_loader: &khr::surface::Instance,
        image_view: vk::ImageView,
        width: u32,
        height: u32,
    ) -> Self {
        let swapchain = Swapchain::new(
            instance,
            device,
            physical_device,
            surface,
            surface_loader,
            width,
            height,
        );

        let render_pass = create_render_pass(device, swapchain.format);
        let framebuffers = create_framebuffers(device, render_pass, &swapchain.image_views, swapchain.extent);

        // 创建 blit descriptor set layout
        let blit_descriptor_set_layout = {
            let bindings = [vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)];

            let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
            unsafe { device.create_descriptor_set_layout(&layout_info, None).unwrap() }
        };

        let (blit_pipeline, blit_pipeline_layout) =
            create_blit_pipeline(device, render_pass, blit_descriptor_set_layout);

        // 创建 blit descriptor pool 和 set
        let blit_descriptor_pool = {
            let pool_sizes = [vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_IMAGE,
                descriptor_count: 1,
            }];
            let pool_info = vk::DescriptorPoolCreateInfo::default()
                .pool_sizes(&pool_sizes)
                .max_sets(1);
            unsafe { device.create_descriptor_pool(&pool_info, None).unwrap() }
        };

        let blit_descriptor_set = {
            let layouts = [blit_descriptor_set_layout];
            let alloc_info = vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(blit_descriptor_pool)
                .set_layouts(&layouts);
            unsafe { device.allocate_descriptor_sets(&alloc_info).unwrap()[0] }
        };

        // 更新 blit descriptor set
        {
            let image_info = vk::DescriptorImageInfo::default()
                .image_view(image_view)
                .image_layout(vk::ImageLayout::GENERAL);
            let image_infos = [image_info];
            let write = vk::WriteDescriptorSet::default()
                .dst_set(blit_descriptor_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&image_infos);
            unsafe { device.update_descriptor_sets(&[write], &[]) };
        }

        // 创建同步对象
        let semaphore_info = vk::SemaphoreCreateInfo::default();
        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        let image_available_semaphore =
            unsafe { device.create_semaphore(&semaphore_info, None).unwrap() };
        let render_finished_semaphore =
            unsafe { device.create_semaphore(&semaphore_info, None).unwrap() };
        let in_flight_fence = unsafe { device.create_fence(&fence_info, None).unwrap() };

        Self {
            swapchain,
            render_pass,
            framebuffers,
            blit_pipeline,
            blit_pipeline_layout,
            blit_descriptor_set_layout,
            blit_descriptor_pool,
            blit_descriptor_set,
            image_available_semaphore,
            render_finished_semaphore,
            in_flight_fence,
        }
    }

    /// 销毁窗口模式资源
    pub unsafe fn destroy(self, device: &ash::Device) {
        device.destroy_semaphore(self.image_available_semaphore, None);
        device.destroy_semaphore(self.render_finished_semaphore, None);
        device.destroy_fence(self.in_flight_fence, None);
        device.destroy_pipeline(self.blit_pipeline, None);
        device.destroy_pipeline_layout(self.blit_pipeline_layout, None);
        device.destroy_descriptor_pool(self.blit_descriptor_pool, None);
        device.destroy_descriptor_set_layout(self.blit_descriptor_set_layout, None);
        for fb in self.framebuffers {
            device.destroy_framebuffer(fb, None);
        }
        device.destroy_render_pass(self.render_pass, None);
        self.swapchain.destroy(device);
    }
}

/// 渲染到 swapchain
pub fn render_to_swapchain(
    device: &ash::Device,
    command_pool: vk::CommandPool,
    graphics_queue: vk::Queue,
    resources: &WindowedResources,
    sampled: u32,
) -> Result<(), vk::Result> {
    unsafe {
        device.wait_for_fences(&[resources.in_flight_fence], true, u64::MAX)?;
        device.reset_fences(&[resources.in_flight_fence])?;
    }

    let acquire_result = unsafe {
        resources.swapchain.loader.acquire_next_image(
            resources.swapchain.swapchain,
            u64::MAX,
            resources.image_available_semaphore,
            vk::Fence::null(),
        )
    };

    let image_index = match acquire_result {
        Ok((index, _)) => index,
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) | Err(vk::Result::SUBOPTIMAL_KHR) => {
            return Ok(());
        }
        Err(e) => return Err(e),
    };

    // 分配 blit 命令缓冲区
    let blit_cmd = {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_buffer_count(1)
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY);
        unsafe { device.allocate_command_buffers(&allocate_info) }?[0]
    };

    unsafe {
        device.begin_command_buffer(blit_cmd, &vk::CommandBufferBeginInfo::default())?;

        let clear_values = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        }];

        let render_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(resources.render_pass)
            .framebuffer(resources.framebuffers[image_index as usize])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: resources.swapchain.extent,
            })
            .clear_values(&clear_values);

        device.cmd_begin_render_pass(blit_cmd, &render_pass_info, vk::SubpassContents::INLINE);

        device.cmd_bind_pipeline(blit_cmd, vk::PipelineBindPoint::GRAPHICS, resources.blit_pipeline);
        device.cmd_bind_descriptor_sets(
            blit_cmd,
            vk::PipelineBindPoint::GRAPHICS,
            resources.blit_pipeline_layout,
            0,
            &[resources.blit_descriptor_set],
            &[],
        );

        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: resources.swapchain.extent.width as f32,
            height: resources.swapchain.extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: resources.swapchain.extent,
        };
        device.cmd_set_viewport(blit_cmd, 0, &[viewport]);
        device.cmd_set_scissor(blit_cmd, 0, &[scissor]);

        device.cmd_push_constants(
            blit_cmd,
            resources.blit_pipeline_layout,
            vk::ShaderStageFlags::FRAGMENT,
            0,
            &sampled.to_le_bytes(),
        );

        device.cmd_draw(blit_cmd, 3, 1, 0, 0);

        device.cmd_end_render_pass(blit_cmd);
        device.end_command_buffer(blit_cmd)?;
    }

    // 提交 blit 命令
    let wait_semaphores = [resources.image_available_semaphore];
    let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
    let signal_semaphores = [resources.render_finished_semaphore];
    let command_buffers_submit = [blit_cmd];

    let submit_info = vk::SubmitInfo::default()
        .wait_semaphores(&wait_semaphores)
        .wait_dst_stage_mask(&wait_stages)
        .command_buffers(&command_buffers_submit)
        .signal_semaphores(&signal_semaphores);

    unsafe {
        device.queue_submit(graphics_queue, &[submit_info], resources.in_flight_fence)?;
    }

    // Present
    let swapchains = [resources.swapchain.swapchain];
    let image_indices = [image_index];
    let present_info = vk::PresentInfoKHR::default()
        .wait_semaphores(&signal_semaphores)
        .swapchains(&swapchains)
        .image_indices(&image_indices);

    unsafe {
        let _ = resources.swapchain.loader.queue_present(graphics_queue, &present_info);
        device.queue_wait_idle(graphics_queue)?;
        device.free_command_buffers(command_pool, &[blit_cmd]);
    }

    Ok(())
}
