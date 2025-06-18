#![allow(clippy::todo)]

use std::{marker::PhantomData, mem::transmute, num::NonZeroUsize, sync::Arc};

use anyhow::{Error, Result};
use ecs_module::ffi::GpuEcsModule;
use game_asset::{ecs_module::GpuInterface, world_render_manager::WorldRenderManager};
use game_ecs::{
    ComponentBundle, ComponentDefault, ComponentRegistry, CpuFrameData, EcsTypeInfo,
    FrameDataBufferBorrowRef, FrameDataBufferBorrowRefMut, FrameDataBufferRef,
    FrameDataBufferRefMut, GpuFrameData, PartitionIndex, manually_register_resource,
};
use game_module_macro::Resource;
use glam::Vec2;
use gpu_common::Gpu;
use platform::{EcsModule, Platform};
use snapshot::{Deserialize, Deserializer, ReadUninit, Serialize, Serializer, WriteUninit};
use void_public::{
    Aspect, ComponentId, EcsType, LocalToWorld, Resource, Transform,
    colors::Color,
    graphics::{CircleRender, ColorRender, TextRender, TextureId, TextureRender},
    material::MaterialParameters,
};
use wgpu::{
    Adapter, Buffer, CommandEncoder, Device, Instance, Limits, Queue, RequestAdapterOptions,
    Surface, SurfaceConfiguration, SurfaceTarget, SurfaceTexture,
};

pub use crate::platform_library::platform_ecs;
use crate::{
    ecs_module::ffi::MODULE_NAME,
    gpu_config::{GpuConfig, GpuConfigBackends},
    gpu_managers::{
        particle_manager::ParticleEffectManager,
        pipeline_manager::{GlobalUniformBuffer, GpuPipelineManager},
        texture_manager::{GpuTextureManager, RenderTargetType},
    },
    render::draw_list::DrawList,
    types_brush::TextBrush,
};

pub mod ecs_module;
mod gpu_config;
mod gpu_managers;
mod platform_library;
mod render;
mod types_brush;
mod types_shapes2d;

#[derive(Debug)]
pub struct GpuWeb {
    gpu_resource_index: Option<usize>,
    render_viewport: Option<RenderViewport>,
    queue: Arc<Queue>,
    device: Arc<Device>,
    adapter: Adapter,
    surface: Surface<'static>,
    _instance: Instance,
}

/// This struct stores necessary types and managers for dealing with `wgpu`'s
/// APIs for the GPU. This is a platform specific construct, and the intent is
/// that most users will not directly access this. Instead, users will interact
/// with [`GpuInterface`].
#[derive(Debug)]
pub struct GpuResource {
    swapchain_surface: Option<SurfaceTexture>,
    encoder: Option<CommandEncoder>,

    /// If this field is `Some`, the renderer should take the viewport into
    /// account. If it is `None`, the renderer should output to the full game
    /// window.
    render_viewport: Option<RenderViewport>,

    pub queue: Arc<Queue>,
    pub device: Arc<Device>,

    /// The first 6 vertices form a quad for sprite rendering.  The rest are currently used to store circles
    frame_vertex_buffer: Buffer,
    default_brush: TextBrush,
    pub pipeline_manager: GpuPipelineManager,
    pub texture_manager: GpuTextureManager,
}

static mut GPU_RESOURCE_CID: Option<ComponentId> = None;

impl Resource for GpuResource {
    fn new() -> Self {
        unreachable!()
    }
}

impl EcsType for GpuResource {
    fn id() -> ComponentId {
        unsafe { GPU_RESOURCE_CID.expect("ComponentId unassigned") }
    }

    unsafe fn set_id(id: ComponentId) {
        unsafe {
            GPU_RESOURCE_CID = Some(id);
        }
    }

    fn string_id() -> &'static std::ffi::CStr {
        c"gpu_web::GpuResource"
    }
}

impl Serialize for GpuResource {
    fn serialize<W>(&self, _: &mut Serializer<W>) -> snapshot::Result<()>
    where
        W: WriteUninit,
    {
        Ok(())
    }
}

impl Deserialize for GpuResource {
    unsafe fn deserialize<R>(_: &mut Deserializer<R>) -> snapshot::Result<Self>
    where
        R: ReadUninit,
    {
        panic!("use deserialize_in_place()!");
    }

    unsafe fn deserialize_in_place<R>(&mut self, _: &mut Deserializer<R>) -> snapshot::Result<()>
    where
        R: ReadUninit,
    {
        Ok(())
    }
}

/// This struct describes a viewport within the game's window. The renderer will
/// draw the final render output to this viewport. Described another way, the
/// final render output will effectively be scaled to this viewport's specified
/// position and bounds.
#[derive(Debug, Clone, Copy)]
pub struct RenderViewport {
    /// The position of the viewport from the top left corner of the window, in
    /// pixels.
    pub position: Vec2,
    /// The size of the viewport, in pixels.
    pub bounds: Vec2,
}

// Web MVP is singlethreaded, so this is fine
#[cfg(target_family = "wasm")]
unsafe impl Send for GpuWeb {}
#[cfg(target_family = "wasm")]
unsafe impl Sync for GpuWeb {}
#[cfg(target_family = "wasm")]
unsafe impl Send for GpuResource {}
#[cfg(target_family = "wasm")]
unsafe impl Sync for GpuResource {}

impl GpuWeb {
    pub async fn new<'a, ST: Into<SurfaceTarget<'a>>>(
        width: u32,
        height: u32,
        window: ST,
    ) -> Result<Self> {
        let gpu_config = GpuConfig::default();
        let backends = match gpu_config.backends() {
            GpuConfigBackends::Default => wgpu::Backends::default(),
            GpuConfigBackends::DX12 => wgpu::Backends::DX12,
            GpuConfigBackends::BrowserWebGpu => wgpu::Backends::BROWSER_WEBGPU,
            GpuConfigBackends::Metal => wgpu::Backends::METAL,
            GpuConfigBackends::GL => wgpu::Backends::GL,
            GpuConfigBackends::Vulkan => wgpu::Backends::VULKAN,
        };
        let power_preference = if gpu_config.high_performance_adapter() {
            wgpu::PowerPreference::HighPerformance
        } else {
            wgpu::PowerPreference::LowPower
        };

        let instance = Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });

        let window: SurfaceTarget<'a> = window.into();
        let surface = instance.create_surface(window).unwrap();

        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                compatible_surface: Some(&surface),
                power_preference,
                ..Default::default()
            })
            .await
            .unwrap();

        let mut required_limits = Limits::downlevel_defaults();
        required_limits.max_texture_dimension_1d = 8192;
        required_limits.max_texture_dimension_2d = 8192;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::TEXTURE_COMPRESSION_BC,
                    required_limits,
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();

        let config = GpuWeb::get_surface_config(&adapter, &surface, width, height);
        surface.configure(&device, &config);

        // the surface lives as long as the instance
        let surface = unsafe { transmute::<wgpu::Surface<'_>, wgpu::Surface<'_>>(surface) };

        #[allow(clippy::arc_with_non_send_sync)]
        Ok(Self {
            gpu_resource_index: None,
            render_viewport: None,
            queue: Arc::new(queue),
            device: Arc::new(device),
            adapter,
            surface,
            _instance: instance,
        })
    }

    pub fn get_surface_config(
        adapter: &Adapter,
        surface: &Surface<'_>,
        width: u32,
        height: u32,
    ) -> SurfaceConfiguration {
        // We're forcing non-srgb format for surfaces atm to match the older Painter.
        // However, this should be made a config option in the future
        let present_mode = if GpuConfig::default().vsync() {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };

        let surface_caps = surface.get_capabilities(adapter);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_caps
                .formats
                .iter()
                .copied()
                .find(|f| !f.is_srgb())
                .unwrap(),
            width,
            height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        config
    }

    pub fn gpu_buffer_index(&self) -> &Option<usize> {
        &self.gpu_resource_index
    }

    pub fn set_render_viewport(&mut self, viewport: Option<RenderViewport>) {
        self.render_viewport = viewport;
    }
}

impl Gpu for GpuWeb {
    type Error = Error;

    const MULTI_BUFFERED: bool = false;

    fn multi_buffer_count(&self) -> NonZeroUsize {
        NonZeroUsize::new(1).unwrap()
    }

    fn window_resized(
        &mut self,
        width: u32,
        height: u32,
        component_registry: &ComponentRegistry,
        cpu_data: &mut CpuFrameData,
    ) {
        if width > 0 && height > 0 {
            let config = GpuWeb::get_surface_config(&self.adapter, &self.surface, width, height);
            self.surface.configure(&self.device, &config);

            let mut gpu_resource = cpu_data.borrow_buffer_mut(self.gpu_resource_index.unwrap());
            let gpu_resource = unsafe { gpu_resource.get_mut_as::<GpuResource>(0).unwrap() };

            let gpu_interface_index = match &component_registry[&GpuInterface::id()].ecs_type_info {
                EcsTypeInfo::Resource(info) => info.buffer_index,
                _ => unreachable!(),
            };

            let mut buffer = cpu_data.borrow_buffer_mut(gpu_interface_index);
            let gpu_interface = unsafe { buffer.get_mut_as::<GpuInterface>(0).unwrap() };

            gpu_resource.texture_manager.create_render_targets(
                gpu_interface,
                &self.device,
                &config,
            );
        }
    }

    fn register_preloaded_texture(
        &mut self,
        cpu_data: &mut CpuFrameData,
        component_registry: &ComponentRegistry,
        texture_id: TextureId,
        path: String,
        data: Vec<u8>,
        width_and_height: (u32, u32),
        use_atlas: bool,
    ) -> (u32, u32) {
        let mut gpu_resource = cpu_data.borrow_buffer_mut(self.gpu_resource_index.unwrap());
        let gpu_resource = unsafe { gpu_resource.get_mut_as::<GpuResource>(0).unwrap() };

        let gpu_interface_index = match &component_registry[&GpuInterface::id()].ecs_type_info {
            EcsTypeInfo::Resource(info) => info.buffer_index,
            _ => unreachable!(),
        };

        let mut buffer = cpu_data.borrow_buffer_mut(gpu_interface_index);
        let gpu_interface = unsafe { buffer.get_mut_as::<GpuInterface>(0).unwrap() };
        if use_atlas {
            gpu_resource.texture_manager.load_texture_into_atlas(
                &data,
                &path,
                texture_id,
                &gpu_resource.device,
                &gpu_resource.queue,
                gpu_interface,
            )
        } else {
            match gpu_resource.texture_manager.load_texture(
                &data,
                texture_id,
                &path,
                width_and_height,
                &gpu_resource.device,
                &gpu_resource.queue,
            ) {
                Ok(dimensions) => dimensions,
                Err(err) => {
                    log::warn!("Error loading texture at {path}: {err}");
                    (0, 0)
                }
            }
        }
    }

    fn register_components(&mut self, _component_registry: &mut ComponentRegistry) {}

    fn register_resources(
        &mut self,
        cpu_data: &mut CpuFrameData,
        component_registry: &mut ComponentRegistry,
    ) {
        let gpu_config = GpuConfig::default();

        // Set up `GpuResource`.

        let frame_vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Gpu::frame_vertex_buffer"),
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            size: gpu_config.default_circles_vertex_buffer_size_bytes(),
        });

        let (aspect_width, aspect_height) = cpu_data
            .get_resource(component_registry, |aspect: &Aspect| {
                (aspect.width as u32, aspect.height as u32)
            });

        let config =
            GpuWeb::get_surface_config(&self.adapter, &self.surface, aspect_width, aspect_height);

        let gpu_resource =
            cpu_data.get_resource_mut(component_registry, |gpu_interface: &mut GpuInterface| {
                GpuResource {
                    swapchain_surface: None,
                    encoder: None,
                    render_viewport: None,
                    default_brush: TextBrush::new(self, gpu_interface, aspect_width, aspect_height),
                    texture_manager: GpuTextureManager::new(
                        gpu_interface,
                        &self.device,
                        &self.queue,
                        &config,
                    ),
                    pipeline_manager: GpuPipelineManager::new(&self.device, &gpu_config),
                    frame_vertex_buffer,
                    queue: self.queue.clone(),
                    device: self.device.clone(),
                }
            });

        let gpu_resource_index =
            manually_register_resource(gpu_resource, MODULE_NAME, component_registry, cpu_data);

        self.gpu_resource_index = Some(gpu_resource_index);

        // Set up `GpuConfig`.

        manually_register_resource(gpu_config, MODULE_NAME, component_registry, cpu_data);

        // Set up `GlobalUniformBuffer`.

        manually_register_resource(
            GlobalUniformBuffer::default(),
            MODULE_NAME,
            component_registry,
            cpu_data,
        );

        // Set up `WorldRenderManager`.

        manually_register_resource(
            WorldRenderManager::default(),
            MODULE_NAME,
            component_registry,
            cpu_data,
        );

        // Set up `DrawList`.

        manually_register_resource(
            DrawList::default(),
            MODULE_NAME,
            component_registry,
            cpu_data,
        );

        // Set up `ParticleEffectManager`.

        // Get the format/sample_count of the render target that PEM will draw
        // to so this info can be passed to its constructor.
        let (target_format, target_sample_count) =
            cpu_data.get_resource(component_registry, |gpu_resource: &GpuResource| {
                let color_msaa = gpu_resource
                    .texture_manager
                    .get_render_target(RenderTargetType::ColorMsaa);

                (
                    color_msaa.texture.format(),
                    color_msaa.texture.sample_count(),
                )
            });

        let particle_effect_manager =
            ParticleEffectManager::new(&self.device, target_format, target_sample_count);

        manually_register_resource(
            particle_effect_manager,
            MODULE_NAME,
            component_registry,
            cpu_data,
        );
    }

    fn component_groupings(&mut self) -> Vec<Vec<ComponentId>> {
        Vec::new()
    }

    fn component_bundles(&mut self) -> Vec<ComponentBundle> {
        Vec::from([
            ComponentBundle {
                source_components: Vec::from([Transform::id()]),
                bundled_components: Vec::from([ComponentDefault {
                    id: LocalToWorld::id(),
                    default_value: bytemuck::cast_slice(&[LocalToWorld::default()]).into(),
                }]),
            },
            ComponentBundle {
                source_components: Vec::from([TextureRender::id()]),
                bundled_components: Vec::from([
                    ComponentDefault {
                        id: Transform::id(),
                        default_value: bytemuck::cast_slice(&[Transform::default()]).into(),
                    },
                    ComponentDefault {
                        id: LocalToWorld::id(),
                        default_value: bytemuck::cast_slice(&[LocalToWorld::default()]).into(),
                    },
                    ComponentDefault {
                        id: Color::id(),
                        default_value: bytemuck::cast_slice(&[Color::default()]).into(),
                    },
                    ComponentDefault {
                        id: MaterialParameters::id(),
                        default_value: bytemuck::cast_slice(&[MaterialParameters::default()])
                            .into(),
                    },
                ]),
            },
            ComponentBundle {
                source_components: Vec::from([ColorRender::id()]),
                bundled_components: Vec::from([
                    ComponentDefault {
                        id: Transform::id(),
                        default_value: bytemuck::cast_slice(&[Transform::default()]).into(),
                    },
                    ComponentDefault {
                        id: LocalToWorld::id(),
                        default_value: bytemuck::cast_slice(&[LocalToWorld::default()]).into(),
                    },
                    ComponentDefault {
                        id: Color::id(),
                        default_value: bytemuck::cast_slice(&[Color::default()]).into(),
                    },
                ]),
            },
            ComponentBundle {
                source_components: Vec::from([CircleRender::id()]),
                bundled_components: Vec::from([
                    ComponentDefault {
                        id: Transform::id(),
                        default_value: bytemuck::cast_slice(&[Transform::default()]).into(),
                    },
                    ComponentDefault {
                        id: LocalToWorld::id(),
                        default_value: bytemuck::cast_slice(&[LocalToWorld::default()]).into(),
                    },
                    ComponentDefault {
                        id: Color::id(),
                        default_value: bytemuck::cast_slice(&[Color::default()]).into(),
                    },
                ]),
            },
            ComponentBundle {
                source_components: Vec::from([TextRender::id()]),
                bundled_components: Vec::from([
                    ComponentDefault {
                        id: Transform::id(),
                        default_value: bytemuck::cast_slice(&[Transform::default()]).into(),
                    },
                    ComponentDefault {
                        id: LocalToWorld::id(),
                        default_value: bytemuck::cast_slice(&[LocalToWorld::default()]).into(),
                    },
                    ComponentDefault {
                        id: Color::id(),
                        default_value: bytemuck::cast_slice(&[Color::default()]).into(),
                    },
                ]),
            },
        ])
    }

    fn single_buffer_components(&mut self) -> Vec<ComponentId> {
        Vec::new()
    }

    fn component_archetype_keys(&mut self) -> Vec<ComponentId> {
        Vec::new()
    }

    fn ecs_module<P: Platform>(&self) -> Box<dyn EcsModule> {
        Box::new(GpuEcsModule::<P>::new())
    }

    fn begin_frame(&mut self, cpu_data: &mut CpuFrameData) {
        let mut gpu_resource = cpu_data.borrow_buffer_mut(self.gpu_resource_index.unwrap());
        let gpu_resource = unsafe { gpu_resource.get_mut_as::<GpuResource>(0).unwrap() };

        gpu_resource.render_viewport = self.render_viewport;
        gpu_resource.swapchain_surface = self.surface.get_current_texture().unwrap().into();
        gpu_resource.encoder = self
            .device
            .create_command_encoder(&Default::default())
            .into();
    }

    fn submit_frame(&mut self, cpu_data: &mut CpuFrameData) {
        let mut gpu_resource = cpu_data.borrow_buffer_mut(self.gpu_resource_index.unwrap());
        let gpu_resource = unsafe { gpu_resource.get_mut_as::<GpuResource>(0).unwrap() };

        let swapchain_surface = gpu_resource.swapchain_surface.take().unwrap();
        let encoder = gpu_resource.encoder.take().unwrap();

        self.queue.submit(Some(encoder.finish()));
        swapchain_surface.present();
    }

    fn destroy(self, _: &mut CpuFrameData) {}
}

impl GpuFrameData for GpuWeb {
    type FrameDataBufferBorrowRef = DataBufferBorrowRef;

    type FrameDataBufferBorrowRefMut = DataBufferBorrowRefMut;

    type FrameDataBufferRef<'a> = DataBufferRef<'a>;

    type FrameDataBufferRefMut<'a> = DataBufferRefMut<'a>;

    fn new_buffer(&mut self, _cpu_data: &mut CpuFrameData, _stride: usize) -> usize {
        todo!()
    }

    fn allocate_buffer_partition(&mut self, _index: usize) -> PartitionIndex {
        todo!()
    }

    fn buffers_len(&self) -> usize {
        todo!()
    }

    fn buffer_total_len(&self, _index: usize) -> usize {
        todo!()
    }

    fn borrow_buffer(
        &self,
        _index: usize,
        _partition: PartitionIndex,
    ) -> Self::FrameDataBufferBorrowRef {
        todo!()
    }

    fn borrow_buffer_prev(
        &self,
        _index: usize,
        _partition: PartitionIndex,
    ) -> Self::FrameDataBufferBorrowRef {
        unreachable!()
    }

    fn borrow_buffer_mut(
        &self,
        _index: usize,
        _partition: PartitionIndex,
    ) -> Self::FrameDataBufferBorrowRefMut {
        todo!()
    }

    fn get_buffer_mut(
        &mut self,
        _cpu_data: &mut CpuFrameData,
        _index: usize,
        _partition: PartitionIndex,
    ) -> Self::FrameDataBufferRefMut<'_> {
        todo!()
    }

    fn get_buffer_prev(
        &mut self,
        _index: usize,
        _partition: PartitionIndex,
        _frames_behind: usize,
    ) -> Self::FrameDataBufferRef<'_> {
        unreachable!()
    }
}

#[derive(Debug)]
pub struct DataBufferBorrowRef;

impl FrameDataBufferBorrowRef for DataBufferBorrowRef {
    unsafe fn get_as<T>(&self, _index: usize) -> Option<&T> {
        todo!()
    }

    unsafe fn get_with_offset_as<T>(&self, _index: usize, _offset: usize) -> Option<&T> {
        todo!()
    }

    fn len(&self) -> usize {
        todo!()
    }

    fn is_empty(&self) -> bool {
        todo!()
    }

    fn has_been_copied_this_frame(&self) -> bool {
        todo!()
    }

    fn get_ptr(&self, _index: usize) -> *const std::mem::MaybeUninit<u8> {
        todo!()
    }
}

#[derive(Debug)]
pub struct DataBufferBorrowRefMut;

impl FrameDataBufferBorrowRef for DataBufferBorrowRefMut {
    unsafe fn get_as<T>(&self, _index: usize) -> Option<&T> {
        todo!()
    }

    unsafe fn get_with_offset_as<T>(&self, _index: usize, _offset: usize) -> Option<&T> {
        todo!()
    }

    fn len(&self) -> usize {
        todo!()
    }

    fn is_empty(&self) -> bool {
        todo!()
    }

    fn has_been_copied_this_frame(&self) -> bool {
        todo!()
    }

    fn get_ptr(&self, _index: usize) -> *const std::mem::MaybeUninit<u8> {
        todo!()
    }
}

impl FrameDataBufferBorrowRefMut for DataBufferBorrowRefMut {
    unsafe fn get_mut_as<T>(&mut self, _index: usize) -> Option<&mut T> {
        todo!()
    }

    unsafe fn get_mut_with_offset_as<T>(
        &mut self,
        _index: usize,
        _offset: usize,
    ) -> Option<&mut T> {
        todo!()
    }

    fn mark_has_been_copied_this_frame(&mut self) {
        todo!()
    }

    fn get_mut_ptr(&mut self, _index: usize) -> *mut std::mem::MaybeUninit<u8> {
        todo!()
    }
}

#[derive(Debug)]
pub struct DataBufferRef<'a>(PhantomData<&'a ()>);

impl<'a> FrameDataBufferRef<'a> for DataBufferRef<'a> {}

impl FrameDataBufferBorrowRef for DataBufferRef<'_> {
    unsafe fn get_as<T>(&self, _index: usize) -> Option<&T> {
        todo!()
    }

    unsafe fn get_with_offset_as<T>(&self, _index: usize, _offset: usize) -> Option<&T> {
        todo!()
    }

    fn len(&self) -> usize {
        todo!()
    }

    fn is_empty(&self) -> bool {
        todo!()
    }

    fn has_been_copied_this_frame(&self) -> bool {
        todo!()
    }

    fn get_ptr(&self, _index: usize) -> *const std::mem::MaybeUninit<u8> {
        todo!()
    }
}

#[derive(Debug)]
pub struct DataBufferRefMut<'a>(PhantomData<&'a ()>);

impl<'a> FrameDataBufferRefMut<'a> for DataBufferRefMut<'a> {
    fn write<T>(&mut self, _index: usize, _val: T) {
        todo!()
    }

    fn push<T>(&mut self, _val: T) {
        todo!()
    }

    fn grow(&mut self) -> &mut [std::mem::MaybeUninit<u8>] {
        todo!()
    }

    unsafe fn pop<T>(&mut self) -> Option<T> {
        todo!()
    }

    fn swap_remove(&mut self, _index: usize) {
        todo!()
    }
}

impl FrameDataBufferBorrowRef for DataBufferRefMut<'_> {
    unsafe fn get_as<T>(&self, _index: usize) -> Option<&T> {
        todo!()
    }

    unsafe fn get_with_offset_as<T>(&self, _index: usize, _offset: usize) -> Option<&T> {
        todo!()
    }

    fn len(&self) -> usize {
        todo!()
    }

    fn is_empty(&self) -> bool {
        todo!()
    }

    fn has_been_copied_this_frame(&self) -> bool {
        todo!()
    }

    fn get_ptr(&self, _index: usize) -> *const std::mem::MaybeUninit<u8> {
        todo!()
    }
}

impl FrameDataBufferBorrowRefMut for DataBufferRefMut<'_> {
    unsafe fn get_mut_as<T>(&mut self, _index: usize) -> Option<&mut T> {
        todo!()
    }

    unsafe fn get_mut_with_offset_as<T>(
        &mut self,
        _index: usize,
        _offset: usize,
    ) -> Option<&mut T> {
        todo!()
    }

    fn mark_has_been_copied_this_frame(&mut self) {
        todo!()
    }

    fn get_mut_ptr(&mut self, _index: usize) -> *mut std::mem::MaybeUninit<u8> {
        todo!()
    }
}
