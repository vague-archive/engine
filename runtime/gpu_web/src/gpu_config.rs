use game_module_macro::ResourceWithoutSerialize;

use crate::{ComponentId, EcsType, Resource};

#[derive(Clone, Copy)]
pub enum GpuConfigBackends {
    Default,
    Vulkan,
    DX12,
    Metal,
    BrowserWebGpu,
    GL,
}

// GpuConfig data is hard-coded and read-only.  Eventually, this info will be read from a file
#[derive(ResourceWithoutSerialize)]
pub struct GpuConfig {
    default_circles_vertex_buffer_size_bytes: u64,
    default_scene_instances_buffer_size_bytes: u64,

    // When reallocating vertex/uniform buffers, the final size is calculated by multiplying the requested size by buffer_growth_factor
    buffer_growth_factor: f32,

    vsync: bool,
    high_performance_adapter: bool,
    backends: GpuConfigBackends,
}

impl Default for GpuConfig {
    fn default() -> Self {
        Self {
            default_circles_vertex_buffer_size_bytes: 1048576,
            default_scene_instances_buffer_size_bytes: 134217728,
            buffer_growth_factor: 1.25,
            vsync: true,
            high_performance_adapter: false,
            backends: GpuConfigBackends::Default,
        }
    }
}

impl GpuConfig {
    pub fn default_circles_vertex_buffer_size_bytes(&self) -> u64 {
        self.default_circles_vertex_buffer_size_bytes
    }

    pub fn default_scene_instances_buffer_size_bytes(&self) -> u64 {
        self.default_scene_instances_buffer_size_bytes
    }

    pub fn buffer_growth_factor(&self) -> f32 {
        self.buffer_growth_factor
    }

    pub fn vsync(&self) -> bool {
        self.vsync
    }

    pub fn high_performance_adapter(&self) -> bool {
        self.high_performance_adapter
    }

    pub fn backends(&self) -> GpuConfigBackends {
        self.backends
    }
}
