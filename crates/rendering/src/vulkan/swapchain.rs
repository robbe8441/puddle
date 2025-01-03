use super::{MemoryBlock, VulkanDevice};
use ash::prelude::VkResult;
use ash::vk;
use std::cell::UnsafeCell;
use std::sync::Arc;

pub struct SwapchainImage {
    pub main_image: vk::Image, // does not need to be destroyed manually
    pub main_view: vk::ImageView,
    pub depth_image: vk::Image,
    pub depth_memory: MemoryBlock,
    pub depth_view: vk::ImageView,
    pub available: vk::Fence, // also does not need to be destroyed
}

impl SwapchainImage {
    pub unsafe fn destroy(&self, device: &VulkanDevice) {
        device.destroy_image_view(self.main_view, None);
        device.destroy_image_view(self.depth_view, None);
        device.destroy_image(self.depth_image, None);
    }
}

pub struct Swapchain {
    device: Arc<VulkanDevice>,
    pub handle: UnsafeCell<vk::SwapchainKHR>,
    pub loader: ash::khr::swapchain::Device,
    pub images: UnsafeCell<Vec<SwapchainImage>>,
    pub create_info: UnsafeCell<vk::SwapchainCreateInfoKHR<'static>>,
}

impl Swapchain {
    /// # Safety
    /// # Errors
    pub unsafe fn new(device: Arc<VulkanDevice>, image_extent: [u32; 2]) -> VkResult<Self> {
        let surface_capabilities = device
            .surface_loader
            .get_physical_device_surface_capabilities(device.pdevice, device.surface)?;

        let surface_format = device
            .surface_loader
            .get_physical_device_surface_formats(device.pdevice, device.surface)?[0];

        let surface_resolution = match surface_capabilities.current_extent.width {
            u32::MAX => vk::Extent2D {
                width: image_extent[0],
                height: image_extent[1],
            },
            _ => surface_capabilities.current_extent,
        };

        let pre_transform = if surface_capabilities
            .supported_transforms
            .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
        {
            vk::SurfaceTransformFlagsKHR::IDENTITY
        } else {
            surface_capabilities.current_transform
        };

        let present_modes = device
            .surface_loader
            .get_physical_device_surface_present_modes(device.pdevice, device.surface)?;

        let present_mode = present_modes
            .iter()
            .copied()
            .find(|&mode| mode == vk::PresentModeKHR::MAILBOX)
            .unwrap_or(vk::PresentModeKHR::FIFO);

        let mut desired_image_count = surface_capabilities.min_image_count + 1;
        if surface_capabilities.max_image_count > 0
            && desired_image_count > surface_capabilities.max_image_count
        {
            desired_image_count = surface_capabilities.max_image_count;
        }

        let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(device.surface)
            .min_image_count(desired_image_count)
            .image_color_space(surface_format.color_space)
            .image_format(surface_format.format)
            .image_extent(surface_resolution)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(pre_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .image_array_layers(1);

        let swapchain_loader = ash::khr::swapchain::Device::new(&device.instance, &device);

        let swapchain = swapchain_loader.create_swapchain(&swapchain_create_info, None)?;

        let images = Self::create_swapchain_images(
            device.clone(),
            &swapchain_loader,
            swapchain,
            surface_format.format,
            image_extent,
        )?;

        Ok(Self {
            device,
            handle: UnsafeCell::new(swapchain),
            loader: swapchain_loader,
            create_info: UnsafeCell::new(swapchain_create_info),
            images: UnsafeCell::new(images),
        })
    }

    unsafe fn create_swapchain_images(
        device: Arc<VulkanDevice>,
        swapchain_loader: &ash::khr::swapchain::Device,
        swapchain: vk::SwapchainKHR,
        format: vk::Format,
        image_extent: [u32; 2],
    ) -> VkResult<Vec<SwapchainImage>> {
        let swapchain_images = swapchain_loader.get_swapchain_images(swapchain)?;

        Ok(swapchain_images
            .iter()
            .map(|&main_image| {
                let components = vk::ComponentMapping::default()
                    .r(vk::ComponentSwizzle::IDENTITY)
                    .g(vk::ComponentSwizzle::IDENTITY)
                    .b(vk::ComponentSwizzle::IDENTITY)
                    .a(vk::ComponentSwizzle::IDENTITY);

                let subresource_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1);

                let info = vk::ImageViewCreateInfo::default()
                    .image(main_image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .components(components)
                    .subresource_range(subresource_range);

                let main_view = device.create_image_view(&info, None).unwrap();

                let depth_format = vk::Format::R32_SFLOAT;
                let depth_image_info = vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(depth_format)
                    .extent(vk::Extent3D {
                        width: image_extent[0],
                        height: image_extent[1],
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT);

                let depth_image = device.create_image(&depth_image_info, None).unwrap();

                let memory_requirements = device.get_image_memory_requirements(depth_image);
                let depth_memory = MemoryBlock::new(
                    device.clone(),
                    memory_requirements,
                    vk::MemoryPropertyFlags::DEVICE_LOCAL,
                )
                .unwrap();

                device
                    .bind_image_memory(depth_image, depth_memory.handle(), 0)
                    .unwrap();

                let subresource = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1);

                let depth_view_info = vk::ImageViewCreateInfo::default()
                    .image(depth_image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(depth_format)
                    .subresource_range(subresource);

                let depth_view = device.create_image_view(&depth_view_info, None).unwrap();

                SwapchainImage {
                    main_image,
                    main_view,
                    depth_image,
                    depth_memory,
                    depth_view,
                    available: vk::Fence::null(),
                }
            })
            .collect())
    }

    /// # Safety
    /// there must not currently be written on to one of the swapchain images
    /// the pointer to the swapchain handle is now invalid
    /// # Errors
    /// if there was an issue allocating new images
    /// for example if no space if left
    pub unsafe fn recreate(&self, device: Arc<VulkanDevice>, new_extent: [u32; 2]) -> VkResult<()> {
        let handle = self.handle.get();

        let image_extent = vk::Extent2D {
            width: new_extent[0],
            height: new_extent[1],
        };

        (*self.create_info.get()).image_extent = image_extent;

        let create_info = vk::SwapchainCreateInfoKHR {
            old_swapchain: *handle,
            ..*self.create_info.get()
        };

        *handle = self.loader.create_swapchain(&create_info, None)?;

        for image in &*self.images.get() {
            image.destroy(&device);
        }

        self.loader
            .destroy_swapchain(create_info.old_swapchain, None);

        *self.images.get() = Self::create_swapchain_images(
            device,
            &self.loader,
            *handle,
            create_info.image_format,
            new_extent,
        )?;

        Ok(())
    }

    pub fn image_format(&self) -> vk::Format {
        unsafe { (*self.create_info.get()).image_format }
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        unsafe {
            for image in &*self.images.get() {
                image.destroy(&self.device);
            }

            let handle = *self.handle.get();
            self.loader.destroy_swapchain(handle, None);
        }
    }
}

unsafe fn get_supported_format(
    device: &VulkanDevice,
    candidates: &[vk::Format],
    tiling: vk::ImageTiling,
    features: vk::FormatFeatureFlags,
) -> Option<vk::Format> {
    candidates.iter().copied().find(|f| {
        let properties = device
            .instance
            .get_physical_device_format_properties(device.pdevice, *f);

        match tiling {
            vk::ImageTiling::LINEAR => properties.linear_tiling_features.contains(features),
            vk::ImageTiling::OPTIMAL => properties.optimal_tiling_features.contains(features),
            _ => false,
        }
    })
}

unsafe fn get_depth_format(device: &VulkanDevice) -> Option<vk::Format> {
    let candidates = &[
        vk::Format::D32_SFLOAT,
        vk::Format::D32_SFLOAT_S8_UINT,
        vk::Format::D24_UNORM_S8_UINT,
    ];

    get_supported_format(
        device,
        candidates,
        vk::ImageTiling::OPTIMAL,
        vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT,
    )
}
