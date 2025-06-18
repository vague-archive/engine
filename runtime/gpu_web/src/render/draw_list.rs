use std::ops::Range;

use game_asset::{
    resource_managers::material_manager::material_parameters_extension::MaterialParametersExt,
    world_render_manager::PostProcess,
};
use game_module_macro::ResourceWithoutSerialize;
use glam::{Mat4, Quat, Vec2, Vec3};
use log::error;
use void_public::{
    ComponentId, EcsType, Resource, Transform,
    colors::Color,
    graphics::{ParticleEffectHandle, ParticleRender, TextureId},
    linalg::Vec4,
    material::{DefaultMaterials, MaterialId, MaterialParameters},
};
use wgpu::{DynamicOffset, RenderPass};

use crate::{
    gpu_managers::{
        particle_manager::ParticleEffectManager, pipeline_manager::GpuPipelineManager,
        texture_manager::GpuTextureManager,
    },
    types_brush::{TextBrush, TextSectionParams},
    types_shapes2d::{Shape2d, Vertex2d},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DrawType {
    Circle {
        /// `vertex_range` contains this circle's range of vertices in `GpuResource.frame_vertex_buffer`
        vertex_range: Range<u32>,

        /// `scene_instance_index` is this Draw Object's index into `PipelineManager::scene_instances_buffer`.
        scene_instance_index: u32,
    },
    Text {
        section_index: u32,
    },
    Texture {
        scene_instance_index: u32,
    },
    Particle {
        handle: ParticleEffectHandle,
    },
}

impl DrawType {
    pub fn scene_instance_index(&self) -> Option<u32> {
        match self {
            DrawType::Texture {
                scene_instance_index,
            }
            | DrawType::Circle {
                vertex_range: _,
                scene_instance_index,
            } => Some(*scene_instance_index),
            _ => None,
        }
    }
}

/// `DrawObject` represents all drawable objects (`DrawType`s) that can be sorted, batched, and rendered by `DrawList`.
#[derive(Debug)]
pub struct DrawObject {
    pub z_depth: f32,

    pub draw_type: DrawType,
    material_id: MaterialId,
}

/// Represents a post process draw
#[derive(Debug)]
pub struct PostProcessDrawObject {
    pub material_id: MaterialId,
    pub draw_index: u32,
}

/// `DrawList::batch_draw_objects()` collects `DrawObject`s into `DrawBatch`s.
/// Batches are then rendered in `DrawList::render()`
#[derive(Debug)]

struct DrawBatch {
    range: Range<u32>,
}

/// `DrawList` collects, sorts, and renders `DrawObject`s
#[derive(Debug, ResourceWithoutSerialize)]
pub struct DrawList {
    /// Each frame, `frame_draws` is filled with retained and intermediate draw objects
    /// This intermediate step is temporary until `gpu_compatible` resources are available
    pub frame_draws: Vec<DrawObject>,

    /// Each frame, `post_process_draws` is filled with post process draw objects
    pub post_process_draws: Vec<PostProcessDrawObject>,

    /// The first 6 vertices form a quad used to render sprites.  The remaining space contains
    /// the circles and lines rendered this frame.  Note: It may be worthwhile to cache these
    pub frame_vertices: Vec<Vertex2d>,

    draw_batches: Vec<DrawBatch>,
}

impl Default for DrawList {
    fn default() -> Self {
        // The first 6 vertices form a quad used to render sprites
        let mut frame_vertices = vec![];
        Shape2d::generate_default_quad(&mut frame_vertices);
        Self {
            frame_draws: vec![],
            post_process_draws: vec![],
            frame_vertices,
            draw_batches: vec![],
        }
    }
}

impl DrawList {
    /// Call each frame before adding `DrawObject`s with the `DrawList::add_*` functions
    pub fn clear(&mut self) {
        // Clear frame vertices except for the sprite quad
        self.frame_vertices.resize(6, Vertex2d::default());

        self.frame_draws.clear();

        self.draw_batches.clear();

        self.post_process_draws.clear();
    }

    pub fn add_circle(
        &mut self,
        pipeline_manager: &mut GpuPipelineManager,
        texture_manager: &GpuTextureManager,
        circle: &Shape2d,
        material_parameters: &MaterialParameters,
        model_matrix: &Mat4,
    ) {
        // Add this circle's scene data to PipelineManager::scene_storage.
        let scene_instance_index = DrawList::add_to_scene_buffer(
            pipeline_manager,
            Some(texture_manager),
            material_parameters,
            model_matrix,
            &circle.color,
            &Vec4::from_xyzw(1., 1., 0., 0.),
        );

        // Generate circle vertices and add to the dynamic vertex buffer.
        let vertex_range_start = self.frame_vertices.len();
        self.frame_vertices.extend(circle.generate_circle());

        // Add a `DrawObject` for rendering in `DrawList::render`
        let world_pos = model_matrix.mul_vec4(circle.position.extend(1.0f32));
        self.frame_draws.push(DrawObject {
            z_depth: world_pos.z,
            material_id: material_parameters.material_id(),
            draw_type: DrawType::Circle {
                vertex_range: Range {
                    start: vertex_range_start as u32,
                    end: self.frame_vertices.len() as u32,
                },
                scene_instance_index,
            },
        });
    }

    pub fn add_particle(&mut self, particle: &ParticleRender, z_depth: f32) {
        self.frame_draws.push(DrawObject {
            z_depth,
            draw_type: DrawType::Particle {
                handle: particle.handle(),
            },
            material_id: DefaultMaterials::Sprite.material_id(),
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_texture(
        &mut self,
        pipeline_manager: &mut GpuPipelineManager,
        texture_manager: &GpuTextureManager,
        material_parameters: &MaterialParameters,
        model_matrix: &Mat4,
        z_depth: f32,
        color: &Color,
        uv_scale_offset: &Vec4,
    ) {
        // Add this texture's scene data to PipelineManager::scene_storage.
        let scene_instance_index = DrawList::add_to_scene_buffer(
            pipeline_manager,
            Some(texture_manager),
            material_parameters,
            model_matrix,
            color,
            uv_scale_offset,
        );

        // Add a `DrawObject` for rendering in `DrawList::render`.
        let draw_object = DrawObject {
            draw_type: DrawType::Texture {
                scene_instance_index,
            },
            z_depth,
            material_id: material_parameters.material_id(),
        };
        self.frame_draws.push(draw_object);
    }

    pub fn add_line(
        &mut self,
        pipeline_manager: &mut GpuPipelineManager,
        material_parameters: &MaterialParameters,
        from: &Vec3,
        to: &Vec2,
        color: &Color,
        thickness: f32,
    ) {
        // Add this line's scene data to PipelineManager::scene_storage.
        let scene_instance_index = DrawList::add_to_scene_buffer(
            pipeline_manager,
            None,
            material_parameters,
            &Mat4::IDENTITY,
            color,
            &Vec4::from_xyzw(1., 1., 0., 0.),
        );

        // Calculate line end points.
        let vertex_range_start = self.frame_vertices.len();
        let to = Vec3::new(to.x, to.y, from.z);
        Shape2d::generate_line(
            &mut self.frame_vertices,
            from,
            &Vec3::new(to.x, to.y, from.z),
            thickness,
            *color,
        );

        // Add a `DrawObject` for rendering in `DrawList::render`.
        self.frame_draws.push(DrawObject {
            draw_type: DrawType::Circle {
                vertex_range: Range {
                    start: vertex_range_start as u32,
                    end: self.frame_vertices.len() as u32,
                },
                scene_instance_index,
            },
            z_depth: to.z,
            material_id: material_parameters.material_id(),
        });
    }

    pub fn add_text(&mut self, brush: &mut TextBrush, section_params: &TextSectionParams<'_>) {
        // Add a `DrawObject` representing this text.
        self.frame_draws.push(DrawObject {
            z_depth: section_params.z,
            material_id: DefaultMaterials::Sprite.material_id(),
            draw_type: DrawType::Text {
                section_index: brush.num_sections() as u32,
            },
        });

        // Add a brush section for to processing later in `TextBrush::process_sections()`.
        brush.add_section(section_params);
    }

    // Helper function called by `DrawList::add_*` to fill `GpuPipelineManager`'s scene instance buffer with `DrawObject`s.
    fn add_to_scene_buffer(
        pipeline_manager: &mut GpuPipelineManager,
        texture_manager: Option<&GpuTextureManager>,
        material_parameters: &MaterialParameters,
        model_matrix: &Mat4,
        color: &Color,
        uv_scale_offset: &Vec4,
    ) -> u32 {
        // If using an atlased texture, apply the correct uv_scale_offset
        let uv_scale_offset = if let Some(texture_manager) = texture_manager {
            texture_manager
                .uv_scale_offset(material_parameters.textures[0])
                .unwrap_or([
                    uv_scale_offset.x,
                    uv_scale_offset.y,
                    uv_scale_offset.z,
                    uv_scale_offset.w,
                ])
        } else {
            [
                uv_scale_offset.x,
                uv_scale_offset.y,
                uv_scale_offset.z,
                uv_scale_offset.w,
            ]
        };

        // Get material parameters for this instance.
        let instance_buffer_parameters = &[
            model_matrix.x_axis,
            model_matrix.y_axis,
            model_matrix.z_axis,
            model_matrix.w_axis,
            ***color,
            uv_scale_offset.into(),
        ];
        let uniform_buffer =
            material_parameters.to_uniform_buffer(None, Some(instance_buffer_parameters));

        pipeline_manager.add_scene_instance(
            material_parameters.material_id(),
            &uniform_buffer,
            &material_parameters.textures,
        )
    }

    fn add_postprocess_to_scene_buffer(
        pipeline_manager: &mut GpuPipelineManager,
        post_process: &PostProcess,
    ) -> u32 {
        let mut material_parameters = MaterialParameters::new(*post_process.material_id());
        if let Err(err) =
            material_parameters.update_from_material_uniforms(&post_process.material_uniforms)
        {
            error!(
                "Problem adding PostProcess {} buffer to scene instance: {err}",
                post_process.material_id()
            );
        }
        pipeline_manager.add_scene_instance(
            *post_process.material_id(),
            &material_parameters.data,
            &[],
        )
    }

    pub fn add_postprocess(
        &mut self,
        pipeline_manager: &mut GpuPipelineManager,
        post_process: &PostProcess,
    ) {
        let draw_index = DrawList::add_postprocess_to_scene_buffer(pipeline_manager, post_process);

        self.post_process_draws.push(PostProcessDrawObject {
            material_id: *post_process.material_id(),
            draw_index,
        });
    }

    pub fn batch_draw_objects(&mut self, pipeline_manager: &GpuPipelineManager) {
        // This is a helper function for comparing textures between draw objects.
        let get_batch_textures =
            |scene_index: Option<u32>, material_id: MaterialId| -> Option<&[TextureId]> {
                if let Some(scene_index) = scene_index {
                    pipeline_manager.textures_for_draw(scene_index, material_id)
                } else {
                    None
                }
            };

        let draw_objects = &mut self.frame_draws;
        if draw_objects.is_empty() {
            return;
        }

        // Set up the first batch.
        let mut batch_start = 0;
        let mut batch_textures = get_batch_textures(
            draw_objects[0].draw_type.scene_instance_index(),
            draw_objects[0].material_id,
        );

        let draw_batches = &mut self.draw_batches;
        for i in 1..draw_objects.len() {
            let cur_draw = &draw_objects[i];
            let cur_draw_textures = get_batch_textures(
                cur_draw.draw_type.scene_instance_index(),
                cur_draw.material_id,
            );

            // Determine if we need to start a new batch.
            let new_batch = {
                #[allow(clippy::if_same_then_else)]
                if cur_draw.material_id != draw_objects[batch_start].material_id {
                    true
                } else if std::mem::discriminant(&cur_draw.draw_type)
                    != std::mem::discriminant(&draw_objects[batch_start].draw_type)
                {
                    true
                } else if batch_textures != cur_draw_textures {
                    true
                } else {
                    // todo: only textures supported atm
                    !matches!(
                        cur_draw.draw_type,
                        DrawType::Texture {
                            scene_instance_index: _
                        }
                    )
                }
            };

            if new_batch {
                // Close the previous batch and start a new one.
                let batch_end = i;
                draw_batches.push(DrawBatch {
                    range: batch_start as u32..batch_end as u32,
                });

                batch_start = i;
                batch_textures = get_batch_textures(
                    draw_objects[batch_start].draw_type.scene_instance_index(),
                    draw_objects[batch_start].material_id,
                );
            }
        }

        // Close final batch.
        let last_idx = draw_objects.len();
        draw_batches.push(DrawBatch {
            range: batch_start as u32..last_idx as u32,
        });
    }

    /// Submit the draw calls for the given `render_pass`.
    ///
    /// * `dynamic_uniform_buffer_offset` - The offset from the front of the global uniform buffer for bind groups.
    ///   It is a multiple of the stride of that buffer.
    /// * `text_brush_uniform_buffer_offset` - The offset from the front of the text brush uniform buffer for bind groups.
    ///   It equals a multiple of the stride of that buffer.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        render_pass: &mut RenderPass<'_>,
        device: &wgpu::Device,
        text_brush: &TextBrush,
        dynamic_vertex_buffer: &wgpu::Buffer,
        texture_manager: &mut GpuTextureManager,
        pipeline_manager: &GpuPipelineManager,
        dynamic_uniform_buffer_offset: DynamicOffset,
        text_brush_uniform_buffer_offset: DynamicOffset,
        particle_effect_manager: &mut ParticleEffectManager,
        mat_vp: &Mat4,
    ) {
        // This check needs to happen after encoder.begin_render_pass() so that the color attachment is cleared
        if self.frame_draws.is_empty() {
            return;
        }

        // scene instance buffers stay constant over all `DrawObject`s
        render_pass.set_vertex_buffer(0, dynamic_vertex_buffer.slice(..));
        render_pass.set_bind_group(1, pipeline_manager.scene_instances_bind().0, &[]);

        // set the uniform buffer bind group w/ offset
        render_pass.set_bind_group(
            0,
            pipeline_manager.global_uniform_bind().0,
            &[dynamic_uniform_buffer_offset],
        );

        for batch in &self.draw_batches {
            let first_obj = &self.frame_draws[batch.range.start as usize];
            match &first_obj.draw_type {
                DrawType::Texture {
                    scene_instance_index,
                } => {
                    let pipeline = pipeline_manager.get_pipeline_or_missing(first_obj.material_id);
                    render_pass.set_pipeline(pipeline.render_pipeline());

                    if let Some(textures) = pipeline_manager
                        .textures_for_draw(*scene_instance_index, first_obj.material_id)
                    {
                        texture_manager.set_tex_bind_group(2, render_pass, textures, device);
                    }

                    render_pass.draw(0..6, batch.range.clone());
                }

                DrawType::Circle {
                    vertex_range,
                    scene_instance_index,
                } => {
                    let pipeline = pipeline_manager.get_pipeline_or_missing(first_obj.material_id);
                    render_pass.set_pipeline(pipeline.render_pipeline());

                    if let Some(textures) = pipeline_manager
                        .textures_for_draw(*scene_instance_index, first_obj.material_id)
                    {
                        texture_manager.set_tex_bind_group(2, render_pass, textures, device);
                    }

                    render_pass.draw(vertex_range.clone(), batch.range.clone());
                }

                DrawType::Text { section_index } => {
                    render_pass.set_pipeline(&text_brush.pipeline);
                    render_pass.set_vertex_buffer(0, text_brush.vertex_buffer.slice(..));
                    render_pass.set_bind_group(
                        0,
                        &text_brush.bind_group,
                        &[(256 * section_index) + text_brush_uniform_buffer_offset],
                    );

                    render_pass.draw(0..4, text_brush.section_draw_range(*section_index as usize));

                    // Since text does not use the material system atm, set the render state back for sprite batching.
                    render_pass.set_vertex_buffer(0, dynamic_vertex_buffer.slice(..));
                    render_pass.set_bind_group(
                        0,
                        pipeline_manager.global_uniform_bind().0,
                        &[dynamic_uniform_buffer_offset],
                    );
                }

                DrawType::Particle { handle } => {
                    particle_effect_manager.render_effect(render_pass, handle, mat_vp);

                    // Since particles do not use the material system atm, set the render state back for sprite batching.
                    render_pass.set_vertex_buffer(0, dynamic_vertex_buffer.slice(..));
                    render_pass.set_bind_group(1, pipeline_manager.scene_instances_bind().0, &[]);
                    render_pass.set_bind_group(
                        0,
                        pipeline_manager.global_uniform_bind().0,
                        &[dynamic_uniform_buffer_offset],
                    );
                }
            }
        }
    }

    /// Create a 4x4 homogenous matrix that represents the values of the given `Transform` component
    pub fn mat4_from_transform(transform: &Transform) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            transform.scale.extend(1.0),
            Quat::from_rotation_z(transform.rotation),
            *transform.position,
        )
    }
}
