use std::{ffi::c_void, ptr::NonNull, sync::Arc};

use ash::{prelude::VkResult, vk};

use crate::vulkan::VulkanDevice;

use super::MemoryBlock;

pub struct Buffer {
    memory: Arc<MemoryBlock>,
    handle: vk::Buffer,
    size: u64,
    offset: u64,
    ptr: Option<NonNull<c_void>>,
}

impl Buffer {
    /// # Errors
    pub fn new(
        device: Arc<VulkanDevice>,
        size: u64,
        usage: vk::BufferUsageFlags,
        property_flags: vk::MemoryPropertyFlags,
    ) -> VkResult<Arc<Self>> {
        let create_info = vk::BufferCreateInfo::default().size(size).usage(usage);

        let buffer = unsafe { device.create_buffer(&create_info, None) }?;
        let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };

        let memory = MemoryBlock::new(device.clone(), requirements, property_flags)?;
        unsafe { device.bind_buffer_memory(buffer, memory.memory, 0) }?;

        let ptr = if property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE) {
            let ptr = unsafe {
                device.map_memory(memory.handle(), 0, size, vk::MemoryMapFlags::empty())
            }?;
            NonNull::new(ptr)
        } else {
            None
        };

        Ok(Self {
            memory: Arc::new(memory),
            handle: buffer,
            size,
            offset: 0,
            ptr,
        }
        .into())
    }

    pub fn write<T: Copy>(&self, offset: usize, data: &[T]) {
        let Some(ptr) = self.ptr else {
            panic!("trying to write to a buffer that isnt devcie local");
        };

        let ptr = unsafe { ptr.as_ptr().cast::<T>().add(offset) };

        let len = data.len().max(self.size as usize / size_of::<T>());

        let slice = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        slice.copy_from_slice(data);
    }

    #[must_use]
    pub fn read<T: Copy>(&self) -> &[T] {
        let Some(ptr) = self.ptr else {
            panic!("trying to read from a buffer that isnt devcie local");
        };

        let ptr = unsafe { ptr.as_ptr().cast::<T>() };

        unsafe { std::slice::from_raw_parts(ptr, self.size as usize / size_of::<T>()) }
    }

    #[must_use]
    pub fn handle(&self) -> vk::Buffer {
        self.handle
    }
    #[must_use]
    pub fn mem_ref(&self) -> &MemoryBlock {
        &self.memory
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            self.memory.device.destroy_buffer(self.handle, None);
        }
    }
}
