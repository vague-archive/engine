// Required for ECS code: types like `EventReader` will trip this
#![allow(clippy::needless_pass_by_value)]

use game_asset::{
    ecs_module::GpuInterface,
    resource_managers::{
        material_manager::{materials::MaterialType, uniforms::MaterialUniforms},
        texture_asset_manager::{TextureAssetManager, events::HandleTextureEventParametersBuilder},
    },
    world_render_manager::{PostProcess, WorldRenderManager},
};
use game_module_macro::{ResourceWithoutSerialize, system, system_once};
use glam::{Mat3, Mat4, Quat, Vec2, Vec3};
use log::error;
use platform::Platform;
use void_public::{
    Aspect, Camera, ComponentId, EcsType, Engine, EventReader, EventWriter, FrameConstants,
    LocalToWorld, Query, Resource, Transform, Viewport,
    colors::Color,
    event::graphics::{
        DrawCircle, DrawLine, DrawRectangle, DrawText, MessageFormatType, NewPipeline, NewTexture,
        PipelineFailed, PipelineFailedBuilder, PipelineLoaded, TextureFailed, TextureFailedBuilder,
        TextureLoaded, TextureLoadedBuilder,
    },
    graphics::{CircleRender, ColorRender, ParticleRender, TextRender, TextureId, TextureRender},
    linalg::Vec4,
    material::{DefaultMaterials, MaterialId, MaterialParameters},
};
use wgpu::{
    BufferAddress, DynamicOffset, LoadOp, Operations, RenderPassColorAttachment,
    RenderPassDescriptor, StoreOp,
};

use crate::{
    GpuResource,
    gpu_config::GpuConfig,
    gpu_managers::{
        particle_manager::ParticleEffectManager,
        pipeline_manager::{GlobalUniformBuffer, GpuPipelineManager},
        texture_manager::{DownsampleOptions, RenderTargetType},
    },
    platform_ecs::{
        HandleTextureEventsFn, PlatformFormatType, PlatformTextureFailed, PlatformTextureRead,
    },
    render::draw_list::DrawList,
    types_brush::{TextBrushUniform, TextSectionParams},
    types_shapes2d::{Shape2d, Vertex2d},
};

/// Renderer min and max z values.  (todo unused)
pub(crate) const Z_MIN: f32 = -4096.;
pub(crate) const Z_MAX: f32 = 4096.;

#[derive(Debug)]
pub struct CameraRenderOrder {
    /// The render order of this entry's camera. Higher `render_order` cameras are rendered earlier
    pub render_order: i32,
    /// The entity id of the camera associated with this entry.
    pub entity_id: void_public::EntityId,
}

/// A resource that keeps track of the sorted render order for all camera components across the application.
#[derive(ResourceWithoutSerialize, Default)]
pub struct CameraRenderResource {
    pub resize_camera_render_textures: bool,
    pub camera_entity_id_render_order: Vec<CameraRenderOrder>,
}

#[system_once]
fn initialize_renderer(gpu_interface: &mut GpuInterface, gpu_resource: &mut GpuResource) {
    let texture_manager = &mut gpu_resource.texture_manager;
    let resolve_target = texture_manager.get_render_target(RenderTargetType::ColorResolve);

    let pipeline_manager = &mut gpu_resource.pipeline_manager;
    pipeline_manager.register_pipeline(
        DefaultMaterials::Sprite.material_id(),
        resolve_target.texture.format(),
        4,
        &gpu_resource.device,
        &gpu_interface.material_manager,
        wgpu::BlendState::ALPHA_BLENDING,
    );

    pipeline_manager.register_pipeline(
        DefaultMaterials::PassThru.material_id(),
        resolve_target.texture.format(),
        1,
        &gpu_resource.device,
        &gpu_interface.material_manager,
        wgpu::BlendState::ALPHA_BLENDING,
    );

    pipeline_manager.register_pipeline(
        DefaultMaterials::MissingOrBroken.material_id(),
        resolve_target.texture.format(),
        4,
        &gpu_resource.device,
        &gpu_interface.material_manager,
        wgpu::BlendState::ALPHA_BLENDING,
    );
}

/// This system ensures that each `Camera` component always has a render texture that is correctly sized
/// based on the window size + its viewport dimensions.
#[system]
fn resize_camera_render_textures(
    mut query_cameras: Query<&mut Camera>,
    camera_render_resource: &mut CameraRenderResource,
    gpu_resource: &mut GpuResource,
    aspect: &Aspect,
) {
    if query_cameras.len() < 2 {
        // 1 or fewer cameras don't require camera textures
        return;
    }

    // check for any cameras that don't have render textures
    query_cameras.for_each(|camera| {
        if camera.render_target_texture_id.borrow().is_none() {
            // detected a camera with no camera render texture
            camera
                .render_target_texture_id
                .set(Some(create_camera_render_target(
                    camera,
                    gpu_resource,
                    aspect,
                )));
        } else if camera_render_resource.resize_camera_render_textures {
            if let Some(camera_render_texture_id) = camera.render_target_texture_id.borrow() {
                // replace the existing render texture with one that is the correct dimensions

                // remove the previous render target texture
                gpu_resource
                    .texture_manager
                    .remove_camera_render_target(*camera_render_texture_id);

                // replace with a new render texture
                camera
                    .render_target_texture_id
                    .set(Some(create_camera_render_target(
                        camera,
                        gpu_resource,
                        aspect,
                    )));
            }
        }
    });
    camera_render_resource.resize_camera_render_textures = false;
}

#[system]
fn update_camera_matrices(aspect: &Aspect, mut query_cameras: Query<(&mut Camera, &LocalToWorld)>) {
    query_cameras.for_each(|(camera, local_to_world)| {
        let aspect_ratio = camera
            .aspect_ratio_override
            .unwrap_or(aspect.width / aspect.height);
        let view_matrix = create_view_matrix(local_to_world, &camera.viewport_ratio);
        let proj_matrix =
            create_orthographic_matrix(aspect.width / camera.orthographic_size, aspect_ratio);

        camera.view_matrix = view_matrix.into();
        camera.projection_matrix = proj_matrix.into();
    });
}

#[system]
fn sort_camera_render_order(
    camera_render_resource: &mut CameraRenderResource,
    mut query_cameras: Query<(&void_public::EntityId, &Camera)>,
) {
    camera_render_resource.camera_entity_id_render_order.clear();

    // add all cameras
    query_cameras.for_each(|(entity_id, camera)| {
        camera_render_resource
            .camera_entity_id_render_order
            .push(CameraRenderOrder {
                render_order: camera.render_order,
                entity_id: **entity_id,
            });
    });

    // sort based on render order
    camera_render_resource
        .camera_entity_id_render_order
        .sort_by(|a, b| b.render_order.cmp(&a.render_order));
}

#[system]
fn begin_frame(
    aspect: &Aspect,
    mut query_cameras: Query<(&mut Camera, &LocalToWorld)>,
    draw_list: &mut DrawList,
    global_uniform_buffer: &mut GlobalUniformBuffer,
    gpu_resource: &mut GpuResource,
    camera_render_resource: &CameraRenderResource,
) {
    let min_uniform_buffer_offset_alignment = gpu_resource
        .device
        .as_ref()
        .limits()
        .min_uniform_buffer_offset_alignment as usize;

    // get the stride for the global uniform buffer
    let global_uniform_buffer_stride = GpuPipelineManager::get_uniform_buffer_stride(
        size_of::<GlobalUniformBuffer>(),
        min_uniform_buffer_offset_alignment,
    );

    // write the camera view-projection matrices into the uniform buffer at offsets per camera
    if !query_cameras.is_empty() {
        let mut camera_index = 0;
        for camera_render_order in &camera_render_resource.camera_entity_id_render_order {
            let Some(mut binding) = query_cameras.get_entity_mut(camera_render_order.entity_id)
            else {
                camera_index += 1;
                continue;
            };

            let (camera, _) = binding.unpack();

            global_uniform_buffer.view_proj_matrix =
                *(camera.projection_matrix * camera.view_matrix);
            // write global uniforms at offset
            gpu_resource.pipeline_manager.write_uniforms(
                &gpu_resource.queue,
                (camera_index * global_uniform_buffer_stride) as BufferAddress,
                global_uniform_buffer,
            );

            camera_index += 1;
        }
    } else {
        // no cameras in the scene, write the fallback values into the uniform buffers
        global_uniform_buffer.view_proj_matrix =
            create_orthographic_matrix(aspect.width, aspect.width / aspect.height);

        // write global uniforms
        gpu_resource
            .pipeline_manager
            .write_uniforms(&gpu_resource.queue, 0, global_uniform_buffer);

        // write text uniforms
        gpu_resource.default_brush.write_uniforms(
            &gpu_resource.queue,
            &global_uniform_buffer.view_proj_matrix,
            0,
        );
    }

    gpu_resource.pipeline_manager.clear_frame_data();
    gpu_resource.default_brush.clear_sections();
    draw_list.clear();
}

#[system]
fn add_retained_to_draw_list(
    mut texture_query: Query<(
        &TextureRender,
        &Transform,
        &LocalToWorld,
        &Color,
        &MaterialParameters,
    )>,
    mut color_query: Query<(&ColorRender, &Transform, &LocalToWorld, &Color)>,
    mut circle_query: Query<(&CircleRender, &Transform, &LocalToWorld, &Color)>,
    mut text_query: Query<(&TextRender, &Transform, &LocalToWorld, &Color)>,
    draw_list: &mut DrawList,
    gpu_resource: &mut GpuResource,
) {
    fn get_model_matrix_data(transform: &Transform, local_to_world: &LocalToWorld) -> (Mat4, f32) {
        let world_pos = local_to_world.mul_vec4(transform.position.extend(1.0f32));
        (***local_to_world, world_pos.z)
    }

    let pipeline_manager = &mut gpu_resource.pipeline_manager;
    let mut default_mat_params = MaterialParameters::new(DefaultMaterials::Sprite.material_id());

    texture_query.for_each(
        |&mut (sprite, transform, local_to_world, color, material_param)| {
            if !sprite.visible {
                return;
            }

            // If the texture has the default sprite material and is missing textures in its MaterialParameters,
            // use the sprite's asset id instead.  This happens if the user did not add a MaterialParameters component
            // to the sprite's entity
            let params = if material_param.material_id() == DefaultMaterials::Sprite.material_id()
                && material_param.textures[0] == TextureAssetManager::missing_texture_id()
            {
                default_mat_params.textures[0] = sprite.texture_id;
                &default_mat_params
            } else {
                material_param
            };

            let rect = sprite.uv_region;
            let uv_scale_offset = Vec4::from_xyzw(
                rect.dimensions.x,
                rect.dimensions.y,
                rect.position.x,
                rect.position.y,
            );

            let (matrix, z) = get_model_matrix_data(transform, local_to_world);
            draw_list.add_texture(
                pipeline_manager,
                &gpu_resource.texture_manager,
                params,
                &matrix,
                z,
                color,
                &uv_scale_offset,
            );
        },
    );

    color_query.for_each(|&mut (color_draw, transform, local_to_world, color)| {
        if !color_draw.visible {
            return;
        }

        let (matrix, z) = get_model_matrix_data(transform, local_to_world);

        default_mat_params.textures[0] = TextureAssetManager::white_texture_id();
        draw_list.add_texture(
            pipeline_manager,
            &gpu_resource.texture_manager,
            &default_mat_params,
            &matrix,
            z,
            color,
            &Vec4::from_xyzw(0., 0., 1., 1.),
        );
    });

    default_mat_params.textures[0] = TextureAssetManager::white_texture_id();
    circle_query.for_each(|&mut (circle_draw, transform, local_to_world, color)| {
        if !circle_draw.visible {
            return;
        }

        let (matrix, _) = get_model_matrix_data(transform, local_to_world);
        let circle = Shape2d {
            num_sides: circle_draw.num_sides as usize,
            position: *transform.position,
            color: *color,
        };
        draw_list.add_circle(
            pipeline_manager,
            &gpu_resource.texture_manager,
            &circle,
            &default_mat_params,
            &matrix,
        );
    });

    text_query.for_each(|&mut (text_draw, transform, local_to_world, color)| {
        if !text_draw.visible {
            return;
        }

        let (matrix, z) = get_model_matrix_data(transform, local_to_world);
        let draw_text_input = TextSectionParams {
            text: &String::from_utf8_lossy(&text_draw.text),
            matrix,
            bounds_size: *text_draw.bounds_size,
            z,
            font_size: text_draw.font_size,
            color: *color,
            alignment: text_draw.alignment,
        };
        draw_list.add_text(&mut gpu_resource.default_brush, &draw_text_input);
    });
}

#[system]
fn add_immediate_to_draw_list(
    gpu_resource: &mut GpuResource,
    draw_list: &mut DrawList,
    circle_events: EventReader<DrawCircle>,
    line_events: EventReader<DrawLine>,
    text_events: EventReader<DrawText<'_>>,
    rectangle_events: EventReader<DrawRectangle<'_>>,
) {
    let pipeline_manager = &mut gpu_resource.pipeline_manager;

    let mut material_param = MaterialParameters::new(DefaultMaterials::Sprite.material_id());
    for event in &circle_events {
        // todo: use transform with circle_events
        let position = event.position();
        let radius = event.radius();
        let transform = Transform {
            position: void_public::linalg::Vec3::from_xyz(position.x(), position.y(), event.z()),
            scale: void_public::linalg::Vec2::new(Vec2::new(radius, radius)),
            skew: void_public::linalg::Vec2::new(Vec2::ZERO),
            pivot: void_public::linalg::Vec2::new(Vec2::new(0.5, 0.5)),
            rotation: event.rotation(),
            _padding: 0.,
        };

        let circle = Shape2d {
            num_sides: event.subdivisions().try_into().unwrap_or(3),
            position: *transform.position,
            color: event.color().into(),
        };

        draw_list.add_circle(
            pipeline_manager,
            &gpu_resource.texture_manager,
            &circle,
            &material_param,
            &Mat4::from_scale_rotation_translation(
                transform.scale.extend(1f32),
                Quat::from_rotation_z(transform.rotation),
                *transform.position,
            ),
        );
    }

    for event in &line_events {
        draw_list.add_line(
            pipeline_manager,
            &material_param,
            &Vec3::new(event.from().x(), event.from().y(), event.z()),
            &event.to().into(),
            &event.color().into(),
            event.thickness(),
        );
    }

    for event in &text_events {
        let text_input = TextSectionParams {
            text: event.text().unwrap(),
            matrix: DrawList::mat4_from_transform(&event.transform().unwrap().into()),
            bounds_size: event.bounds().unwrap().into(),
            z: event.z(),
            font_size: event.font_size(),
            color: event.color().unwrap().into(),
            alignment: event.text_alignment().into(),
        };
        draw_list.add_text(&mut gpu_resource.default_brush, &text_input);
    }

    for event in &rectangle_events {
        let transform = &event.transform().unwrap().into();

        if let Some(asset_id) = event.asset_id() {
            material_param.textures[0] = asset_id.into();
        }
        draw_list.add_texture(
            pipeline_manager,
            &gpu_resource.texture_manager,
            &material_param,
            &DrawList::mat4_from_transform(transform),
            event
                .transform()
                .unwrap_or(&void_public::event::Transform::default())
                .position()
                .z(),
            &event.color().unwrap().into(),
            &Vec4::from_xyzw(1., 1., 0., 0.),
        );
    }
}

#[system]
fn add_postprocesses_to_draw_list(
    draw_list: &mut DrawList,
    gpu_resource: &mut GpuResource,
    world_render_manager: &mut WorldRenderManager,
) {
    let pipeline_manager = &mut gpu_resource.pipeline_manager;
    if world_render_manager.should_generate_down_samples {
        for _ in DOWNSAMPLE_PASSES {
            let downsample_postprocess = PostProcess::new(
                DefaultMaterials::PassThru.material_id(),
                MaterialUniforms::empty(DefaultMaterials::PassThru.material_id()),
            );
            draw_list.add_postprocess(pipeline_manager, &downsample_postprocess);
        }
    }
    for post_process in world_render_manager.postprocesses() {
        draw_list.add_postprocess(pipeline_manager, post_process);
    }
}

#[system]
fn add_particles_to_draw_list(
    gpu_resource: &mut GpuResource,
    gpu_interface: &mut GpuInterface,
    frame_constants: &FrameConstants,
    particle_effect_manager: &mut ParticleEffectManager,
    mut particle_query: Query<(&mut ParticleRender, &Transform, &LocalToWorld)>,
    draw_list: &mut DrawList,
) {
    particle_effect_manager.begin_frame();

    particle_query.for_each(|(particle, transform, local_to_world)| {
        let already_registered = particle_effect_manager.contains_effect(&particle.handle());

        if !particle.visible() {
            if already_registered {
                // Destroy the `ParticleEffect` if its `ParticleRender` is no longer visible
                particle_effect_manager.destroy_effect(&particle.handle());
            }
            return;
        }

        if !already_registered {
            // Register a handle and create a `ParticleEffect`
            particle.init_effect(particle_effect_manager);

            particle_effect_manager.create_effect_from_id(
                gpu_interface,
                particle.handle(),
                &particle.descriptor_id(),
                &Mat3::from_mat4_minor(****local_to_world, 2, 2),
            );
        } else {
            particle_effect_manager.set_transform(
                &particle.handle(),
                &Mat3::from_mat4_minor(****local_to_world, 2, 2),
            );
        }

        if particle_effect_manager.update_effect(&particle.handle(), frame_constants.delta_time) {
            let z_depth = local_to_world.mul_vec4(transform.position.extend(1.0f32)).z;
            draw_list.add_particle(particle, z_depth);
        }
    });

    particle_effect_manager
        .post_update_effects(&gpu_resource.device, &mut gpu_resource.texture_manager);
}

#[system]
#[allow(clippy::too_many_arguments)]
fn render_draw_list(
    gpu_interface: &mut GpuInterface,
    gpu_resource: &mut GpuResource,
    particle_effect_manager: &mut ParticleEffectManager,
    mut query_cameras: Query<&Camera>,
    camera_render_resource: &CameraRenderResource,
    draw_list: &mut DrawList,
    gpu_config: &GpuConfig,
    global_uniforms: &GlobalUniformBuffer,
) {
    let min_uniform_buffer_offset_alignment = gpu_resource
        .device
        .as_ref()
        .limits()
        .min_uniform_buffer_offset_alignment as usize;

    // get the stride for the global uniform buffer
    let uniform_stride = GpuPipelineManager::get_uniform_buffer_stride(
        size_of::<GlobalUniformBuffer>(),
        min_uniform_buffer_offset_alignment,
    );

    // since every individual text section writes its MVP into the uniform buffer, we need to additionally
    // increase the stride by the number of sections
    let text_uniform_stride = GpuPipelineManager::get_uniform_buffer_stride(
        size_of::<TextBrushUniform>() * gpu_resource.default_brush.num_sections(),
        min_uniform_buffer_offset_alignment,
    );

    let _ = gpu_resource.default_brush.process_sections(
        &gpu_resource.device,
        &gpu_resource.queue,
        gpu_interface,
    );

    // Sort draws by depth and collect the indices
    draw_list
        .frame_draws
        .sort_by(|a, b| a.z_depth.total_cmp(&b.z_depth));

    // Todo: Draw Objects that don't use `scene_instance_index` are still added to `instance_indices` with a 0 index.
    let mut instance_indices: Vec<u32> = draw_list
        .frame_draws
        .iter()
        .map(|a| a.draw_type.scene_instance_index().unwrap_or(0))
        .collect();
    instance_indices.extend_from_slice(
        draw_list
            .post_process_draws
            .iter()
            .map(|draw_object| draw_object.draw_index)
            .collect::<Vec<_>>()
            .as_slice(),
    );

    // Write scene constants to gpu buffers
    gpu_resource
        .pipeline_manager
        .write_frame_data(&gpu_resource.queue, &instance_indices);

    // Write circle vertices to gpu buffers
    if !draw_list.frame_vertices.is_empty() {
        let src_shapes_size_bytes = draw_list.frame_vertices.len() * size_of::<Vertex2d>();
        if gpu_resource.frame_vertex_buffer.size() < src_shapes_size_bytes as u64 {
            // Resize `frame_vertex_buffer` if needed
            gpu_resource.frame_vertex_buffer =
                gpu_resource.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Gpu::frame_vertex_buffer"),
                    mapped_at_creation: false,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    size: (gpu_config.buffer_growth_factor() * src_shapes_size_bytes as f32) as u64,
                });
        }

        gpu_resource.queue.write_buffer(
            &gpu_resource.frame_vertex_buffer,
            0,
            bytemuck::cast_slice(draw_list.frame_vertices.as_slice()),
        );
    }

    // Begin render pass and submit draws
    let encoder = gpu_resource.encoder.as_mut().unwrap();

    // batch up the draw objects for the upcoming render passes
    draw_list.batch_draw_objects(&gpu_resource.pipeline_manager);

    if query_cameras.is_empty() {
        gpu_resource.default_brush.write_uniforms(
            &gpu_resource.queue,
            &global_uniforms.view_proj_matrix,
            0 as BufferAddress,
        );

        render_without_camera(
            draw_list,
            gpu_resource,
            particle_effect_manager,
            &global_uniforms.view_proj_matrix,
        );

        particle_effect_manager.end_frame(&gpu_resource.device, &gpu_resource.queue, 1);
        return;
    }

    let mut camera_index = 0;
    for camera_render_order in &camera_render_resource.camera_entity_id_render_order {
        let Some(mut binding) = query_cameras.get_entity_mut(camera_render_order.entity_id) else {
            continue;
        };

        let camera = binding.unpack();

        if !camera.is_enabled {
            // skip disabled cameras
            camera_index += 1;
            continue;
        }

        // write into text brush uniform buffer at offset
        let vp_matrix = camera.projection_matrix * camera.view_matrix;
        gpu_resource.default_brush.write_uniforms(
            &gpu_resource.queue,
            &vp_matrix,
            (camera_index * text_uniform_stride) as BufferAddress,
        );

        // determine the render pass render target
        let texture_manager = &mut gpu_resource.texture_manager;
        let render_pass_target = camera.render_target_texture_id.borrow().map_or_else(
            || texture_manager.get_render_target(RenderTargetType::ColorMsaa),
            |index| {
                texture_manager
                    .camera_render_target(*index)
                    .unwrap_or(texture_manager.get_render_target(RenderTargetType::ColorMsaa))
            },
        );

        // construct the render pass
        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("camera_render_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &render_pass_target.view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(wgpu::Color {
                        r: camera.clear_color.r().into(),
                        g: camera.clear_color.g().into(),
                        b: camera.clear_color.b().into(),
                        a: camera.clear_color.a().into(),
                    }),
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });

        // render to the render target
        draw_list.render(
            &mut render_pass,
            &gpu_resource.device,
            &gpu_resource.default_brush,
            &gpu_resource.frame_vertex_buffer,
            texture_manager,
            &gpu_resource.pipeline_manager,
            (camera_index * uniform_stride) as DynamicOffset,
            (camera_index * text_uniform_stride) as DynamicOffset,
            particle_effect_manager,
            &(camera.projection_matrix * camera.view_matrix),
        );

        // drop the render pass so we can encode the texture blit
        drop(render_pass);

        if let Some(cam_render_target_index) = camera.render_target_texture_id.borrow() {
            let texture_manager = &mut gpu_resource.texture_manager;

            // grab the ColorMSAA render target
            let destination_render_target =
                texture_manager.get_render_target(RenderTargetType::ColorMsaa);

            // grab the camera's render target
            let camera_render_target = texture_manager
                .camera_render_target(*cam_render_target_index)
                .unwrap();

            // determine how much of the camera's viewport to copy
            let mut destination_copy_info = destination_render_target.texture.as_image_copy();
            destination_copy_info.origin.x =
                (camera.viewport_ratio.x * destination_copy_info.texture.width() as f32) as u32;
            destination_copy_info.origin.y =
                (camera.viewport_ratio.y * destination_copy_info.texture.height() as f32) as u32;

            // blit to the ColorMSAA texture
            encoder.copy_texture_to_texture(
                camera_render_target.texture.as_image_copy(),
                destination_copy_info,
                camera_render_target.texture.size(),
            );
        }

        camera_index += 1;
    }
    particle_effect_manager.end_frame(&gpu_resource.device, &gpu_resource.queue, camera_index);

    // Resolve full-screen MSAA target
    let color_msaa = gpu_resource
        .texture_manager
        .get_render_target(RenderTargetType::ColorMsaa);
    let color_resolve = gpu_resource
        .texture_manager
        .get_render_target(RenderTargetType::ColorResolve);

    let _ = encoder.begin_render_pass(&RenderPassDescriptor {
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &color_msaa.view,
            resolve_target: Some(&color_resolve.view),
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
        })],
        ..Default::default()
    });
}

/// This is the fallback render logic if there is no entity with a camera component. This exists to be
/// backwards compatible with existing modules. This may be removed in favor of rendering
/// nothing when there is no spawned camera.
fn render_without_camera(
    draw_list: &mut DrawList,
    gpu_resource: &mut GpuResource,
    particle_effect_manager: &mut ParticleEffectManager,
    view_proj_matrix: &Mat4,
) {
    // construct single render pass that draws directly to `ColorMSAA`
    let encoder = gpu_resource.encoder.as_mut().unwrap();
    let texture_manager = &mut gpu_resource.texture_manager;
    let render_pass_target = texture_manager.get_render_target(RenderTargetType::ColorMsaa);
    let color_resolve = texture_manager.get_render_target(RenderTargetType::ColorResolve);

    let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("default_render_pass"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &render_pass_target.view,
            resolve_target: Some(&color_resolve.view),
            ops: Operations {
                load: LoadOp::Clear(wgpu::Color::BLACK),
                store: StoreOp::Store,
            },
        })],
        ..Default::default()
    });

    // submit draw calls
    draw_list.render(
        &mut render_pass,
        &gpu_resource.device,
        &gpu_resource.default_brush,
        &gpu_resource.frame_vertex_buffer,
        texture_manager,
        &gpu_resource.pipeline_manager,
        0,
        0,
        particle_effect_manager,
        view_proj_matrix,
    );
}

const DOWNSAMPLE_PASSES: [DownsampleOptions; 2] = [
    DownsampleOptions {
        name: "downsample_1x",
        render_target_texture_index: RenderTargetType::ColorDownSample1x,
        sampled_render_target_texture_index: RenderTargetType::ColorResolve,
    },
    DownsampleOptions {
        name: "downsample_2x",
        render_target_texture_index: RenderTargetType::ColorDownSample2x,
        sampled_render_target_texture_index: RenderTargetType::ColorDownSample1x,
    },
];

#[system]
fn render_post_process(
    gpu_resource: &mut GpuResource,
    gpu_interface: &mut GpuInterface,
    world_render_manager: &mut WorldRenderManager,
    draw_list: &DrawList,
) {
    let texture_manager = &mut gpu_resource.texture_manager;
    let pipeline_manager = &mut gpu_resource.pipeline_manager;

    let pass_thru_mat_id = DefaultMaterials::PassThru.material_id();
    let encoder = gpu_resource.encoder.as_mut().unwrap();

    // Create downsampled scene textures if needed
    if world_render_manager.should_generate_down_samples {
        for downsample_pass in DOWNSAMPLE_PASSES {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(format!("render_post_process::{}", downsample_pass.name).as_str()),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &texture_manager
                        .get_render_target(downsample_pass.render_target_texture_index)
                        .view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            let pipeline = pipeline_manager.get_pipeline(pass_thru_mat_id).unwrap();
            render_pass.set_pipeline(pipeline.render_pipeline());
            render_pass.set_vertex_buffer(0, gpu_resource.frame_vertex_buffer.slice(..));
            render_pass.set_bind_group(0, pipeline_manager.global_uniform_bind().0, &[0]);
            render_pass.set_bind_group(1, pipeline_manager.scene_instances_bind().0, &[]);

            texture_manager.set_tex_bind_group(
                1,
                &mut render_pass,
                &[
                    texture_manager
                        .get_render_target_id(downsample_pass.sampled_render_target_texture_index),
                    texture_manager
                        .get_render_target_id(downsample_pass.sampled_render_target_texture_index),
                ],
                &gpu_resource.device,
            );

            render_pass.draw(0..6, 0..1);
        }
    }

    // Custom post processes
    let mut ping_pong = 0;
    for post_process in world_render_manager.postprocesses() {
        let material_id = post_process.material_id();
        let Some(material) = gpu_interface
            .material_manager
            .materials()
            .get(**material_id as usize)
        else {
            error!(
                "Material ID {material_id} not found when attempted post process pass, post process not applied"
            );
            continue;
        };

        let view = if ping_pong == 0 {
            &texture_manager
                .get_render_target(RenderTargetType::PostProcess)
                .view
        } else {
            &texture_manager
                .get_render_target(RenderTargetType::ColorResolve)
                .view
        };

        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some(format!("render_postprocess::material_{}", material.name()).as_str()),
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });

        let Some(pipeline) = pipeline_manager.get_pipeline(*post_process.material_id()) else {
            error!(
                "Material ID {} does not have pipeline, post process not applied",
                post_process.material_id()
            );
            continue;
        };

        render_pass.set_pipeline(pipeline.render_pipeline());
        render_pass.set_vertex_buffer(0, gpu_resource.frame_vertex_buffer.slice(..));
        render_pass.set_bind_group(0, pipeline_manager.global_uniform_bind().0, &[0]);
        render_pass.set_bind_group(1, pipeline_manager.scene_instances_bind().0, &[]);

        let texture_list = if ping_pong == 0 {
            [texture_manager.get_render_target_id(RenderTargetType::ColorResolve)]
        } else {
            [texture_manager.get_render_target_id(RenderTargetType::PostProcess)]
        };

        texture_manager.set_tex_bind_group(
            2,
            &mut render_pass,
            &texture_list,
            &gpu_resource.device,
        );

        let index_start = if let Some(post_process_draw_object) = draw_list
            .post_process_draws
            .iter()
            .find(|post_process_draw| &post_process_draw.material_id == material_id)
        {
            post_process_draw_object.draw_index
        } else {
            0
        };

        render_pass.draw(0..6, index_start..(index_start + 1_u32));
        ping_pong = (ping_pong + 1) & 1;
    }

    // Swap chain
    {
        let swapchain_texture = &gpu_resource.swapchain_surface.as_ref().unwrap().texture;
        let swapchain_texture_view = swapchain_texture.create_view(&Default::default());

        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("render_postprocess::swap chain"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &swapchain_texture_view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });

        if let Some(viewport) = &gpu_resource.render_viewport {
            // Clamp viewport to within the window bounds.
            let x = viewport
                .position
                .x
                .min((swapchain_texture.width() - 1) as f32)
                .max(0.);
            let y = viewport
                .position
                .y
                .min((swapchain_texture.height() - 1) as f32)
                .max(0.);
            let width = viewport
                .bounds
                .x
                .min(swapchain_texture.width() as f32 - x)
                .max(1.);
            let height = viewport
                .bounds
                .y
                .min(swapchain_texture.height() as f32 - y)
                .max(1.);

            render_pass.set_viewport(x, y, width, height, 0., 1.);
        }

        let pipeline = pipeline_manager.get_pipeline(pass_thru_mat_id).unwrap();
        render_pass.set_pipeline(pipeline.render_pipeline());
        render_pass.set_vertex_buffer(0, gpu_resource.frame_vertex_buffer.slice(..));
        render_pass.set_bind_group(0, pipeline_manager.global_uniform_bind().0, &[0]);
        render_pass.set_bind_group(1, pipeline_manager.scene_instances_bind().0, &[]);

        let texture_list = if ping_pong == 0 {
            [texture_manager.get_render_target_id(RenderTargetType::ColorResolve)]
        } else {
            [texture_manager.get_render_target_id(RenderTargetType::PostProcess)]
        };

        texture_manager.set_tex_bind_group(
            2,
            &mut render_pass,
            &texture_list,
            &gpu_resource.device,
        );

        render_pass.draw(0..6, 0..1);
    }
}

fn create_orthographic_matrix(width: f32, aspect_ratio: f32) -> Mat4 {
    let recip_aspect_ratio = aspect_ratio.recip();
    let half_width = width * 0.5;
    let half_height = half_width * recip_aspect_ratio;
    void_public::linalg::create_orthographic_matrix(
        -half_width,
        half_width,
        -half_height,
        half_height,
        Z_MIN,
        Z_MAX,
    )
}

fn create_view_matrix(local_to_world: &LocalToWorld, viewport_rect: &Viewport) -> Mat4 {
    let viewport_matrix = Mat4::from_scale(Vec3::new(
        viewport_rect.width.recip(),
        viewport_rect.height.recip(),
        1.0,
    ));
    viewport_matrix * local_to_world.inverse()
}

/// Create a camera render texture for the given `camera`. Returns the camera texture id associated
/// with the texture.
fn create_camera_render_target(
    camera: &Camera,
    gpu_resource: &mut GpuResource,
    aspect: &Aspect,
) -> u32 {
    gpu_resource.texture_manager.add_camera_render_target(
        &gpu_resource.device,
        (camera.viewport_ratio.width * aspect.width) as u32,
        (camera.viewport_ratio.height * aspect.height) as u32,
    )
}

#[system]
fn send_texture_events_to_platform(new_texture_event_reader: EventReader<NewTexture<'_>>) {
    for event in &new_texture_event_reader {
        Engine::call_with_builder::<HandleTextureEventsFn>(|builder| {
            let asset_path = builder.create_string(event.asset_path().unwrap_or("@@invalid_path"));
            let mut handle_texture_params_builder =
                HandleTextureEventParametersBuilder::new(builder);
            handle_texture_params_builder.add_id(event.id());
            handle_texture_params_builder.add_asset_path(asset_path);
            handle_texture_params_builder.add_insert_in_atlas(event.insert_in_atlas());
            handle_texture_params_builder.finish()
        });
    }
}

fn write_texture_failed(
    texture_id: u32,
    texture_path: &str,
    reason: &str,
    event_writer: &mut EventWriter<TextureFailed<'_>>,
) {
    event_writer.write_builder(|builder| {
        let texture_path = builder.create_string(texture_path);
        let reason = builder.create_string(reason);
        let mut texture_failed_builder = TextureFailedBuilder::new(builder);
        texture_failed_builder.add_id(texture_id);
        texture_failed_builder.add_asset_path(texture_path);
        texture_failed_builder.add_reason(reason);
        texture_failed_builder.finish()
    });
}

const INVALID_PATH: &str = "@@invalid_path@@";

#[system]
fn send_platform_events_to_gpu_interface(
    gpu_resource: &mut GpuResource,
    gpu_interface: &mut GpuInterface,
    texture_loaded_reader: EventReader<PlatformTextureRead<'_>>,
    texture_failed_reader: EventReader<PlatformTextureFailed<'_>>,
    texture_loaded_writer: EventWriter<TextureLoaded<'_>>,
    mut texture_failed_writer: EventWriter<TextureFailed<'_>>,
) {
    for event in &texture_loaded_reader {
        let texture_path = event.asset_path().unwrap_or(INVALID_PATH);
        let format = match event.format() {
            PlatformFormatType::Png => MessageFormatType::Png,
            PlatformFormatType::Jpeg => MessageFormatType::Jpeg,
            unaccounted_for_type => {
                let reason = format!(
                    "Platform type {unaccounted_for_type:?} currently unhandled, cannot process texture {} at path {texture_path}",
                    event.id()
                );
                write_texture_failed(
                    event.id(),
                    texture_path,
                    &reason,
                    &mut texture_failed_writer,
                );
                continue;
            }
        };
        let texture_manager = &mut gpu_resource.texture_manager;
        let data = if let Some(data) = event.data() {
            data.bytes()
        } else {
            let reason = format!(
                "Could not read data to load into GPU for {} with path {texture_path}",
                event.id()
            );
            write_texture_failed(
                event.id(),
                texture_path,
                &reason,
                &mut texture_failed_writer,
            );
            continue;
        };
        let texture_id = TextureId(event.id());
        if event.insert_in_atlas() {
            texture_manager.load_texture_into_atlas(
                data,
                texture_path,
                texture_id,
                &gpu_resource.device,
                &gpu_resource.queue,
                gpu_interface,
            );
        } else if let Err(error) = texture_manager.load_texture(
            data,
            texture_id,
            texture_path,
            (event.width(), event.height()),
            &gpu_resource.device,
            &gpu_resource.queue,
        ) {
            let reason = format!(
                "Could not load texture {texture_id} with path {texture_path} into the GPU: {error}"
            );
            write_texture_failed(
                event.id(),
                texture_path,
                &reason,
                &mut texture_failed_writer,
            );
            continue;
        }
        let version = TextureAssetManager::generate_hash(data);
        texture_loaded_writer.write_builder(|builder| {
            let asset_path = builder.create_string(texture_path);
            let version = builder.create_vector(&*version);
            let mut texture_loaded_builder = TextureLoadedBuilder::new(builder);
            texture_loaded_builder.add_id(event.id());
            texture_loaded_builder.add_asset_path(asset_path);
            texture_loaded_builder.add_format(format);
            texture_loaded_builder.add_version(version);
            texture_loaded_builder.add_width(event.width());
            texture_loaded_builder.add_height(event.height());
            texture_loaded_builder.add_in_atlas(event.insert_in_atlas());
            texture_loaded_builder.finish()
        });
    }

    for event in &texture_failed_reader {
        let texture_path = event.asset_path().unwrap_or(INVALID_PATH);
        let reason = event
            .error_reason()
            .unwrap_or("Error reason lost in message");
        write_texture_failed(event.id(), texture_path, reason, &mut texture_failed_writer);
    }
}

fn write_pipeline_failed(
    pipeline_id: u32,
    material_id: u32,
    reason: &str,
    event_writer: &mut EventWriter<PipelineFailed<'_>>,
) {
    event_writer.write_builder(|builder| {
        let reason = builder.create_string(reason);
        let mut pipeline_failed_builder = PipelineFailedBuilder::new(builder);
        pipeline_failed_builder.add_id(pipeline_id);
        pipeline_failed_builder.add_material_id(material_id);
        pipeline_failed_builder.add_reason(reason);
        pipeline_failed_builder.finish()
    });
}

#[system]
fn handle_pipeline_events(
    gpu_interface: &GpuInterface,
    gpu_resource: &mut GpuResource,
    new_pipeline_event_reader: EventReader<NewPipeline>,
    loaded_pipeline_event_writer: EventWriter<PipelineLoaded>,
    mut failed_pipeline_event_writer: EventWriter<PipelineFailed<'_>>,
) {
    for pipeline_event in &new_pipeline_event_reader {
        let resolve_target = gpu_resource
            .texture_manager
            .get_render_target(RenderTargetType::ColorResolve);
        let Some(material_id) = pipeline_event.material_id().into() else {
            let reason = format!(
                "Could not load pipeline {} because material id {} was not found",
                pipeline_event.id(),
                pipeline_event.material_id()
            );
            write_pipeline_failed(
                pipeline_event.id(),
                pipeline_event.material_id(),
                &reason,
                &mut failed_pipeline_event_writer,
            );
            continue;
        };
        let material_id = MaterialId(material_id);
        let Some(material) = gpu_interface.material_manager.get_material(material_id) else {
            let reason = format!("Could not find material {material_id} to register pipeline");
            write_pipeline_failed(
                pipeline_event.id(),
                pipeline_event.material_id(),
                &reason,
                &mut failed_pipeline_event_writer,
            );
            continue;
        };
        gpu_resource.pipeline_manager.register_pipeline(
            material_id,
            resolve_target.texture.format(),
            match material.material_type() {
                MaterialType::Sprite => 4,
                MaterialType::PostProcessing => 1,
            },
            &gpu_resource.device,
            &gpu_interface.material_manager,
            wgpu::BlendState::ALPHA_BLENDING,
        );

        loaded_pipeline_event_writer.write(PipelineLoaded::new(
            pipeline_event.id(),
            pipeline_event.material_id(),
        ));
    }
}

// =========== Codegen Below ===========

#[allow(unused, clippy::all)]
pub mod ffi {
    use std::{
        borrow::Cow,
        error::Error,
        ffi::{CStr, c_void},
        marker::PhantomData,
        mem::MaybeUninit,
    };

    use platform::{DeserializeReadFn, EcsModule, EcsSystemFn, SerializeWriteFn};
    use void_public::{ArgType, ComponentId, ComponentType};

    use super::*;

    pub const MODULE_NAME: &'static str = "gpu_web";

    include!(concat!(env!("OUT_DIR"), "/ffi.rs"));

    struct EcsSystemFnC(unsafe extern "C" fn(*const *const c_void) -> i32);

    impl EcsSystemFn for EcsSystemFnC {
        unsafe fn call(
            &mut self,
            ptr: *const *const c_void,
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            let res = unsafe { (self.0)(ptr) };

            if res == 0 {
                Ok(())
            } else {
                Err(format!("error code ({res})").into())
            }
        }
    }

    pub struct GpuEcsModule<P: Platform>(PhantomData<P>);

    impl<P: Platform> GpuEcsModule<P> {
        pub fn new() -> Self {
            Self(PhantomData)
        }
    }

    impl<P: Platform> EcsModule for GpuEcsModule<P> {
        fn void_target_version(&self) -> u32 {
            void_target_version()
        }

        fn init(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
            let res = init();

            if res == 0 {
                Ok(())
            } else {
                Err(format!("error code ({res})").into())
            }
        }

        fn deinit(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
            let res = deinit();

            if res == 0 {
                Ok(())
            } else {
                Err(format!("error code ({res})").into())
            }
        }

        fn module_name(&self) -> Cow<'_, str> {
            unsafe { CStr::from_ptr(module_name()).to_string_lossy() }
        }

        fn set_component_id(&mut self, string_id: &CStr, component_id: ComponentId) {
            unsafe {
                set_component_id(string_id.as_ptr(), component_id);
            }
        }

        fn resource_init(
            &self,
            string_id: &CStr,
            val: &mut [std::mem::MaybeUninit<u8>],
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            let res = unsafe { resource_init(string_id.as_ptr(), val.as_mut_ptr().cast()) };

            if res == 0 {
                Ok(())
            } else {
                Err(format!("error code ({res})").into())
            }
        }

        fn resource_deserialize(
            &self,
            string_id: &CStr,
            val: &mut [std::mem::MaybeUninit<u8>],
            read: DeserializeReadFn<'_>,
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            Ok(())
        }

        fn resource_serialize(
            &self,
            string_id: &CStr,
            val: &[std::mem::MaybeUninit<u8>],
            write: SerializeWriteFn<'_>,
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            Ok(())
        }

        fn component_deserialize_json(
            &self,
            string_id: &CStr,
            dest_buffer: &mut [MaybeUninit<u8>],
            json_string: &str,
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            Ok(())
        }

        fn component_string_id(&self, index: usize) -> Option<Cow<'_, CStr>> {
            unsafe {
                let ptr = component_string_id(index);
                if ptr.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr(ptr).into())
                }
            }
        }

        fn component_size(&self, string_id: &CStr) -> usize {
            unsafe { component_size(string_id.as_ptr()) }
        }

        fn component_align(&self, string_id: &CStr) -> usize {
            unsafe { component_align(string_id.as_ptr()) }
        }

        fn component_type(&self, string_id: &CStr) -> ComponentType {
            unsafe { component_type(string_id.as_ptr()) }
        }

        fn component_async_completion_callable(&self, string_id: &CStr) -> Cow<'_, CStr> {
            unsafe {
                let ptr = component_async_completion_callable(string_id.as_ptr());
                assert!(
                    !ptr.is_null(),
                    "component_async_completion_callable returned null"
                );
                CStr::from_ptr(ptr).into()
            }
        }

        fn systems_len(&self) -> usize {
            systems_len()
        }

        fn system_name(&self, system_index: usize) -> Cow<'_, CStr> {
            unsafe { CStr::from_ptr(system_name(system_index)).into() }
        }

        fn system_is_once(&self, system_index: usize) -> bool {
            system_is_once(system_index)
        }

        fn system_fn(&self, system_index: usize) -> Box<dyn EcsSystemFn> {
            Box::new(EcsSystemFnC(system_fn(system_index)))
        }

        fn system_args_len(&self, system_index: usize) -> usize {
            system_args_len(system_index)
        }

        fn system_arg_type(&self, system_index: usize, arg_index: usize) -> ArgType {
            system_arg_type(system_index, arg_index)
        }

        fn system_arg_component(&self, system_index: usize, arg_index: usize) -> Cow<'_, CStr> {
            unsafe { CStr::from_ptr(system_arg_component(system_index, arg_index)).into() }
        }

        fn system_arg_event(&self, system_index: usize, arg_index: usize) -> Cow<'_, CStr> {
            unsafe { CStr::from_ptr(system_arg_event(system_index, arg_index)).into() }
        }

        fn system_query_args_len(&self, system_index: usize, arg_index: usize) -> usize {
            system_query_args_len(system_index, arg_index)
        }

        fn system_query_arg_type(
            &self,
            system_index: usize,
            arg_index: usize,
            query_index: usize,
        ) -> ArgType {
            system_query_arg_type(system_index, arg_index, query_index)
        }

        fn system_query_arg_component(
            &self,
            system_index: usize,
            arg_index: usize,
            query_index: usize,
        ) -> Cow<'_, CStr> {
            unsafe {
                CStr::from_ptr(system_query_arg_component(
                    system_index,
                    arg_index,
                    query_index,
                ))
                .into()
            }
        }
    }
}
