use std::collections::{BTreeMap, HashMap};

use game_asset::ecs_module::MaterialManager;
use game_module_macro::ResourceWithoutSerialize;
use glam::Mat4;
use void_public::{
    ComponentId, EcsType, Resource,
    graphics::TextureId,
    material::{DefaultMaterials, MaterialId, UNIFORM_LIMIT},
};
use wgpu::{
    BindGroup, BindGroupLayout, BindingResource, Buffer, BufferAddress, Device, RenderPipeline,
    TextureFormat,
};

use crate::{gpu_config::GpuConfig, types_shapes2d::Vertex2d};

/// `GlobalUniformBuffer` contains global constant data shared across `MaterialPipeline`s.
/// This resource should be filled at the beginning of the frame and considered
/// read-only for the rest of the frame.
#[repr(C)]
#[derive(
    Debug, Default, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable, ResourceWithoutSerialize,
)]
pub struct GlobalUniformBuffer {
    pub view_proj_matrix: Mat4,
}

/// `PipelineManagerBuffers` is a convenience struct that holds 'wgpu' types.
#[derive(Debug)]
struct PipelineManagerBuffers {
    // `global_uniform_buffer` is a small uniform buffer that is bound to all materials and contains global data
    global_uniform_buffer: Buffer,
    global_uniform_bind: BindGroup,
    global_uniform_layout: BindGroupLayout,

    // `scene_instances_buffer` is an array of structs each of byte size [f32; UNIFORM_LIMIT] that stores per-instance data
    scene_instances_buffer: Buffer,
    scene_instances_layout: BindGroupLayout,

    // `scene_instances_indices` maps draw instances to their corresponding index into `scene_instances_buffer`.  `scene_instances_indices`
    // is sorted back-to-front by Z
    scene_instances_indices: Buffer,
    scene_instances_bind: BindGroup,
}

/// `GpuPipelineManager` manages `MaterialPipeline`s and the various buffers bound to them.
#[derive(Debug)]
pub struct GpuPipelineManager {
    material_to_pipeline: BTreeMap<MaterialId, MaterialPipeline>,
    buffers: PipelineManagerBuffers,

    // `scene_instances_intermediate` contains instance draw data that will be
    // written to [`GpuPipelineManager::scene_instances_buffer`] in
    // [`GpuPipelineManager::write_frame_data`].  This is temporary and will
    // be replaced by direct writes into a `gpu_compatible` buffer.
    scene_instances_intermediate: Vec<[f32; UNIFORM_LIMIT]>,
    scene_textures_intermediate: Vec<TextureId>,

    // Maps an instances index in `scene_instances_intermediate` to its
    // corresponding entry in `scene_textures_intermediate`.
    scene_instance_to_texture: HashMap<usize, usize>,

    // Holds post process indices during frame.
    post_process_indices: Vec<u32>,
}

impl GpuPipelineManager {
    /// Call this before using any other function on `GpuPipelineManager`
    pub fn new(device: &Device, gpu_config: &GpuConfig) -> Self {
        // Global uniform buffer and bind group
        let global_uniform_struct_size =
            wgpu::BufferSize::new(size_of::<GlobalUniformBuffer>() as u64).unwrap();
        let max_buffer_binding_size = device.limits().max_uniform_buffer_binding_size;
        let global_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GpuPipelineManager::global_uniform_buffer"),
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            size: max_buffer_binding_size as u64,
        });

        let global_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: Some(global_uniform_struct_size),
                    },
                    count: None,
                }],
                label: Some("GpuPipelineManager::global_uniform_layout"),
            });

        let mut buffer_binding = global_uniform_buffer.as_entire_buffer_binding();
        buffer_binding.size = Some(global_uniform_struct_size);

        let global_uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &global_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(buffer_binding),
            }],
            label: Some("GpuPipelineManager::global_uniform_bind"),
        });

        // Scene instance buffer and bind group
        let scene_instances_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    // Layout entry for `scene_instances_buffer`
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // Layout entry for `scene_instances_indices`
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
                label: Some("GpuPipelineManager::scene_constant_bind_layout"),
            });

        let scene_instances_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GpuPipelineManager::scene_instances_buffer"),
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            size: gpu_config.default_scene_instances_buffer_size_bytes(),
        });

        let scene_buffer_size_bytes = gpu_config.default_scene_instances_buffer_size_bytes();
        let instance_size_bytes = size_of::<[f32; UNIFORM_LIMIT]>() as u64;
        assert!(scene_buffer_size_bytes % instance_size_bytes == 0);

        let num_instances = scene_buffer_size_bytes / instance_size_bytes;
        let scene_instances_indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GpuPipelineManager::scene_instances_indices"),
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            size: num_instances * size_of::<u32>() as u64,
        });
        log::info!(
            "GpuPipelineManager::scene_instance_buffer has size {scene_buffer_size_bytes} and can hold {num_instances} instances"
        );

        let scene_instances_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &scene_instances_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &scene_instances_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &scene_instances_indices,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
            label: Some("GpuPipelineManager::scene_instances_bind"),
        });

        GpuPipelineManager {
            material_to_pipeline: BTreeMap::<MaterialId, MaterialPipeline>::new(),
            buffers: PipelineManagerBuffers {
                global_uniform_buffer,
                global_uniform_bind,
                global_uniform_layout,
                scene_instances_layout,
                scene_instances_buffer,
                scene_instances_indices,
                scene_instances_bind,
            },

            scene_instances_intermediate: Vec::<[f32; UNIFORM_LIMIT]>::new(),
            scene_textures_intermediate: Vec::<TextureId>::new(),

            scene_instance_to_texture: HashMap::<usize, usize>::new(),

            post_process_indices: Vec::<u32>::new(),
        }
    }

    pub fn register_pipeline(
        &mut self,
        material_id: MaterialId,
        surface_format: TextureFormat,
        sample_count: u32,
        device: &Device,
        material_manager: &MaterialManager,
        blend: wgpu::BlendState,
    ) {
        let shader_text = material_manager.generate_shader_text(material_id);
        if shader_text.is_err() {
            log::warn!(
                "GpuPipelineManager::register_pipeline() - Failed to generate shader text.  Pipeline generation failed."
            );
            return;
        }
        let shader_text = shader_text.unwrap();

        // Create a binding for the material's samplers and textures
        // todo: Don't need to duplicate samplers

        let mut texture_entries = Vec::<wgpu::BindGroupLayoutEntry>::new();
        let mut texture_count = 0;
        let material = material_manager.get_material(material_id).unwrap();

        if let Some(material_textures) = material.generate_default_material_textures() {
            texture_count = material_textures.len() as u32;
            for i in 0..texture_count {
                let bind_start = 2 * i;

                // Texture
                texture_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: bind_start,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                });

                // Sampler
                texture_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: bind_start + 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                });
            }
        }

        let scene_instances_layout = &self.buffers.scene_instances_layout;
        let global_uniform_layout = &self.buffers.global_uniform_layout;

        let pipeline_layout = if !texture_entries.is_empty() {
            // Create a pipeline layout that includes textures
            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    entries: &texture_entries,
                    label: Some("MaterialPipeline::texture_bind_group_layout"),
                });
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("MaterialPipeline::pipeline_layout"),
                bind_group_layouts: &[
                    global_uniform_layout,
                    scene_instances_layout,
                    &bind_group_layout,
                ],
                push_constant_ranges: &[],
            })
        } else {
            // Create a pipeline layout w/o textures
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("MaterialPipeline::pipeline_layout"),
                bind_group_layouts: &[global_uniform_layout, scene_instances_layout],
                push_constant_ranges: &[],
            })
        };

        // Push `ErrorFilter::Validation` error scope to avoid panicking on shader compilation fails.
        // This is compiled out on the wasm target since the future returned from `pop_error_scope()` may complete later.
        #[cfg(not(target_family = "wasm"))]
        device.push_error_scope(wgpu::ErrorFilter::Validation);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("MaterialPipeline::shader_module"),
            source: wgpu::ShaderSource::Wgsl(shader_text.into()),
        });

        #[cfg(not(target_family = "wasm"))]
        if let Some(error) = pollster::block_on(device.pop_error_scope()) {
            log::warn!(
                "GpuPipelineManager::register_pipeline() - Shader compile failed with {}",
                error
            );
            self.material_to_pipeline.remove_entry(&material_id);
            return;
        };

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&format!("MaterialPipeline::render_pipeline_{material_id}")),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex2d::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let pipeline = MaterialPipeline {
            render_pipeline,
            texture_count,
        };
        self.material_to_pipeline.insert(material_id, pipeline);
    }

    pub(crate) fn get_pipeline(&self, material_id: MaterialId) -> Option<&MaterialPipeline> {
        self.material_to_pipeline.get(&material_id)
    }

    /// If a material with `material_id` isn't found, return the `MissingOrBroken` pipeline instead
    pub(crate) fn get_pipeline_or_missing(&self, material_id: MaterialId) -> &MaterialPipeline {
        let material_id = if self.material_to_pipeline.contains_key(&material_id) {
            material_id
        } else {
            DefaultMaterials::MissingOrBroken.material_id()
        };
        self.get_pipeline(material_id).unwrap()
    }

    /// Call this each frame before `GpuPipelineManager::add_scene_instance`
    pub(crate) fn clear_frame_data(&mut self) {
        self.scene_instances_intermediate.clear();
        self.scene_textures_intermediate.clear();
        self.scene_instance_to_texture.clear();
        self.post_process_indices.clear();
    }

    /// Adds the constant data for a draw instance
    pub fn add_scene_instance(
        &mut self,
        mat_id: MaterialId,
        user_data: &[f32; UNIFORM_LIMIT],
        texture_ids: &[TextureId],
    ) -> u32 {
        let uniform_idx = self.scene_instances_intermediate.len();
        self.scene_instances_intermediate.push(*user_data);

        let texture_count = {
            let pipeline = self.get_pipeline_or_missing(mat_id);
            pipeline.texture_count
        };

        if !texture_ids.is_empty() && texture_count > 0 {
            let texture_idx = self.scene_textures_intermediate.len();
            self.scene_textures_intermediate
                .extend_from_slice(&texture_ids[0..texture_count as usize]);
            self.scene_instance_to_texture
                .insert(uniform_idx, texture_idx);
        }

        uniform_idx as u32
    }

    pub fn get_uniform_buffer_stride(size_of_struct: usize, step: usize) -> usize {
        let number_of_steps =
            size_of_struct / step + (if size_of_struct % step == 0 { 0 } else { 1 });
        number_of_steps * step
    }

    /// Call this to update the global uniforms buffer before submitting draw calls
    pub fn write_uniforms(
        &mut self,
        queue: &wgpu::Queue,
        offset: BufferAddress,
        global_uniform_buffer: &GlobalUniformBuffer,
    ) {
        queue.write_buffer(
            self.global_uniform_bind().1,
            offset,
            bytemuck::cast_slice(&[*global_uniform_buffer]),
        );
    }

    /// Call this to update scene instance buffers before submitting draw calls
    pub fn write_frame_data(&mut self, queue: &wgpu::Queue, instance_indices: &[u32]) {
        // todo: reallocate the scene instances buffer if they're too small.
        let (_, scene_constants, scene_indices) = self.scene_instances_bind();
        queue.write_buffer(
            scene_constants,
            0,
            bytemuck::cast_slice(&self.scene_instances_intermediate),
        );

        queue.write_buffer(scene_indices, 0, bytemuck::cast_slice(instance_indices));
    }

    pub fn global_uniform_bind(&self) -> (&wgpu::BindGroup, &wgpu::Buffer) {
        let buffer_data = &self.buffers;
        (
            &buffer_data.global_uniform_bind,
            &self.buffers.global_uniform_buffer,
        )
    }

    pub fn scene_instances_bind(&self) -> (&wgpu::BindGroup, &wgpu::Buffer, &wgpu::Buffer) {
        let buffer_data = &self.buffers;
        (
            &buffer_data.scene_instances_bind,
            &buffer_data.scene_instances_buffer,
            &buffer_data.scene_instances_indices,
        )
    }

    pub fn textures_for_draw(
        &self,
        draw_index: u32,
        material_id: MaterialId,
    ) -> Option<&[TextureId]> {
        let pipeline = self.get_pipeline_or_missing(material_id);
        let texture_count = pipeline.texture_count as usize;
        if texture_count == 0 {
            return None;
        }

        let start = *self.scene_instance_to_texture.get(&(draw_index as usize))?;
        let end = start + texture_count;

        Some(&self.scene_textures_intermediate[start..end])
    }

    pub fn get_post_process_indices(&self) -> &[u32] {
        &self.post_process_indices
    }

    pub fn add_post_process_index(&mut self, index: u32) {
        self.post_process_indices.push(index);
    }
}

#[derive(Debug)]
pub(crate) struct MaterialPipeline {
    render_pipeline: RenderPipeline,
    texture_count: u32,
}

impl MaterialPipeline {
    pub fn render_pipeline(&self) -> &RenderPipeline {
        &self.render_pipeline
    }
}

#[cfg(target_family = "wasm")]
unsafe impl Send for MaterialPipeline {}
#[cfg(target_family = "wasm")]
unsafe impl Sync for MaterialPipeline {}
