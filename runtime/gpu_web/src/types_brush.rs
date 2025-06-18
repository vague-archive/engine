use std::{collections::HashMap, mem::size_of, ops::Range};

use anyhow::{Result, anyhow, bail};
use bytemuck::{Pod, Zeroable, cast_slice};
use game_asset::{
    ecs_module::GpuInterface, resource_managers::texture_asset_manager::EngineTexture,
};
use glam::{Mat4, Vec2, Vec3};
use glyph_brush::{
    BrushAction, BrushError, DefaultSectionHasher, FontId, GlyphBrush, GlyphBrushBuilder,
    GlyphVertex, OwnedSection,
    ab_glyph::{self, FontRef},
};
use lazy_regex::{Lazy, Regex, lazy_regex};
use void_public::{colors::Color, text::TextAlignment};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, BufferAddress,
    BufferBindingType, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites, Device,
    IndexFormat, PipelineLayoutDescriptor, PrimitiveTopology, Queue, RenderPipelineDescriptor,
    SamplerBindingType, ShaderModuleDescriptor, ShaderSource, ShaderStages, TextureFormat,
    TextureSampleType, TextureViewDimension, VertexAttribute, VertexFormat,
};

use crate::{GpuWeb, gpu_managers::texture_manager::Texture};

static TEXT_MARKUP_REGEX: Lazy<Regex> =
    lazy_regex!(r#"<([a-zA-Z0-9\/]+)(?:=")*([a-zA-Z0-9-_]*)"*>"#);

#[repr(C, align(256))]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TextBrushUniform {
    transform: Mat4,
    padding: [u64; 24],
}

#[derive(Debug)]
struct TextBrushSection {
    owned_section: OwnedSection,
    // model-view-projection matrix for this section
    section_transform: Mat4,
    // The draw range in `TextBrush::section_vertices` && `TextBrush::vertex_buffer`
    draw_instances_range: Range<u32>,
}

/// Convenience struct for passing section parameters into `TextBrush::add_section`
#[derive(Debug)]
pub struct TextSectionParams<'a> {
    pub text: &'a str,
    pub matrix: Mat4,
    pub bounds_size: Vec2,
    pub z: f32,
    pub font_size: f32,
    pub color: Color,
    pub alignment: TextAlignment,
}

impl Default for TextSectionParams<'_> {
    fn default() -> Self {
        Self {
            text: "",
            matrix: Mat4::IDENTITY,
            bounds_size: Vec2::new(f32::MAX, f32::MAX),
            z: 0.,
            font_size: 32.,
            color: Color::new(1., 1., 1., 1.),
            alignment: TextAlignment::Left,
        }
    }
}

#[derive(Debug)]
pub(crate) struct TextBrush {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group: BindGroup,
    pub vertex_buffer: wgpu::Buffer,

    glyph_brush: GlyphBrush<TextVertex, glyph_brush::Extra, FontRef<'static>, DefaultSectionHasher>,

    // Filled when users call `TextBrush::add_section` to add text for rendering
    sections: Vec<TextBrushSection>,

    // Intermediate Vec filled by `GlyphBrush` in `TextBrush::process_sections`.  It currently is
    // written to `TextBrush::vertex_buffer` before rendering
    sections_vertices: Vec<TextVertex>,

    uniform_buffer: wgpu::Buffer,
    text_atlas: Texture,

    name_to_font_id: HashMap<String, FontId>,
}

const BUFFER_NUMBER_OF_TEXT_INSTANCES: usize = 256;

impl TextBrush {
    pub fn new(
        gpu: &GpuWeb,
        gpu_interface: &mut GpuInterface,
        aspect_width: u32,
        aspect_height: u32,
    ) -> TextBrush {
        let config =
            GpuWeb::get_surface_config(&gpu.adapter, &gpu.surface, aspect_width, aspect_height);

        let device = &gpu.device;
        let brush_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("brush.wgsl"),
            source: ShaderSource::Wgsl(include_str!("assets/shaders/brush.wgsl").into()),
        });

        // Disabling redraw caching for immediate mode.  Will be addressed with retained mode rendering changes.
        let glyph_brush_builder = GlyphBrushBuilder::using_fonts(vec![]).cache_redraws(false);
        let glyph_brush = glyph_brush_builder.build();

        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("TextBrush::uniform_buffer"),
            size: size_of::<[TextBrushUniform; BUFFER_NUMBER_OF_TEXT_INSTANCES]>() as BufferAddress,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        let mut mapped_range = uniform_buffer.slice(..).get_mapped_range_mut();
        let uniform_buf = TextBrushUniform {
            transform: Mat4::IDENTITY,
            padding: Default::default(),
        };
        let uniform_buf = vec![uniform_buf; BUFFER_NUMBER_OF_TEXT_INSTANCES];
        mapped_range.copy_from_slice(bytemuck::cast_slice(&uniform_buf));
        drop(mapped_range);
        uniform_buffer.unmap();

        let next_texture_id = gpu_interface
            .texture_asset_manager
            .register_next_texture_id();
        let engine_texture = EngineTexture::new(
            next_texture_id,
            &format!("TextBrush::text_atlas_{next_texture_id}").into(),
            glyph_brush.texture_dimensions().0 as usize,
            glyph_brush.texture_dimensions().1 as usize,
            false,
        );

        gpu_interface
            .texture_asset_manager
            .insert_engine_texture(&engine_texture)
            .unwrap();

        let text_atlas = Texture::new_empty(
            "TextBrush::text_atlas",
            device,
            glyph_brush.texture_dimensions(),
            1,
            next_texture_id,
            TextureFormat::R8Unorm,
            false,
        );

        let vertex_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("TextPipeline::vertex_buffer"),
            size: (size_of::<TextVertex>() * BUFFER_NUMBER_OF_TEXT_INSTANCES) as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (bind_group, bind_group_layout) =
            TextBrush::generate_bind_group(device, &uniform_buffer, &text_atlas);

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("TextPipeline::pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("TextPipeline::pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &brush_shader,
                entry_point: Some("vs_main"),
                buffers: &[TextVertex::desc()],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: PrimitiveTopology::TriangleStrip,
                strip_index_format: Some(IndexFormat::Uint16),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 4,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &brush_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format: config.format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
            cache: None,
        });

        let mut brush = TextBrush {
            glyph_brush,
            pipeline,
            sections: vec![],
            sections_vertices: vec![],
            uniform_buffer,
            text_atlas,
            vertex_buffer,
            bind_group,
            name_to_font_id: HashMap::<String, FontId>::new(),
        };

        let _ = brush.add_font("default", include_bytes!("assets/OpenSans-Regular.ttf"));

        brush
    }

    pub fn add_font(&mut self, font_name: &str, font_bytes: &'static [u8]) -> Result<()> {
        if self.name_to_font_id.contains_key(font_name) {
            bail!("TextBrush::add_font() - A font with name {font_name} already exists.");
        }

        let font_ref = FontRef::try_from_slice(font_bytes)?;
        let font_id = self.glyph_brush.add_font(font_ref);
        self.name_to_font_id.insert(font_name.to_string(), font_id);
        Ok(())
    }

    fn generate_bind_group(
        device: &Device,
        uniform_buffer: &wgpu::Buffer,
        texture: &Texture,
    ) -> (BindGroup, BindGroupLayout) {
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("TextBrush::bind_group_layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            size_of::<TextBrushUniform>() as u64
                        ),
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("TextBrush::bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: uniform_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(size_of::<TextBrushUniform>() as u64),
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&texture.view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&texture.sampler),
                },
            ],
        });
        (bind_group, bind_group_layout)
    }

    pub fn num_sections(&self) -> usize {
        self.sections.len()
    }

    pub fn section_draw_range(&self, section_index: usize) -> Range<u32> {
        self.sections[section_index].draw_instances_range.clone()
    }

    // For immediate mode rendering, call clear_sections() at the top of the frame, add text via add_section(), and process_sections() before rendering
    pub fn clear_sections(&mut self) {
        self.sections.clear();
        self.sections_vertices.clear();
    }

    pub fn add_section(&mut self, params: &TextSectionParams<'_>) {
        let text = params.text;
        let text_matrix = params.matrix;
        let bounds_size = params.bounds_size;
        let default_font_size = params.font_size;
        let default_color = params.color;
        let alignment = params.alignment;

        let mut text_spans = vec![];
        let mut font_stack = vec![FontId(0)];
        let mut color_stack = vec![default_color];
        let mut size_stack = vec![default_font_size];
        let mut last_match_end = 0;

        for (_, [cmd, value], cur_match) in TEXT_MARKUP_REGEX.captures_iter(text).map(|captures| {
            let (group_1, group_2) = captures.extract();
            (group_1, group_2, captures.get(0).unwrap())
        }) {
            let font = *font_stack.last().unwrap_or(&FontId(0));
            let color = *color_stack.last().unwrap_or(&default_color);
            let size = *size_stack.last().unwrap_or(&default_font_size);

            match cmd {
                "font" => {
                    if let Some(font_id) = self.name_to_font_id.get(value) {
                        font_stack.push(*font_id);
                    } else {
                        log::warn!("TextBrush::add_section() - Could not find font {value}");
                    }
                }

                "/font" => {
                    font_stack.pop();
                }

                "color" => {
                    let new_color = u32::from_str_radix(value, 16).unwrap_or(0xFFFFFFFF_u32);
                    color_stack.push(Color::from(new_color));
                }

                "/color" => {
                    color_stack.pop();
                }

                "size" => {
                    if let Ok(size) = value.parse() {
                        size_stack.push(size);
                    } else {
                        log::warn!("TextBrush::add_section() - Could change font size to {value}");
                    }
                }

                "/size" => {
                    size_stack.pop();
                }

                _ => {
                    // Not a command
                    continue;
                }
            };

            if last_match_end != cur_match.start() {
                text_spans.push(
                    glyph_brush::Text::new(&text[last_match_end..cur_match.start()])
                        .with_scale(size)
                        .with_color(color.to_array())
                        .with_font_id(font),
                );
            }
            last_match_end = cur_match.end();
        }

        if last_match_end != text.len() {
            let font = *font_stack.last().unwrap_or(&FontId(0));
            let color = *color_stack.last().unwrap_or(&default_color);
            let size = *size_stack.last().unwrap_or(&default_font_size);

            text_spans.push(
                glyph_brush::Text::new(&text[last_match_end..])
                    .with_scale(size)
                    .with_color(color.to_array())
                    .with_font_id(font),
            );
        }

        // Build a glpyh brush Section and add text/formatting for each text_span created
        let mut owned_section = glyph_brush::Section::default();
        for span in text_spans {
            owned_section = owned_section.add_text(span);
        }

        let h_align = match alignment {
            TextAlignment::Left => glyph_brush::HorizontalAlign::Left,
            TextAlignment::Center => glyph_brush::HorizontalAlign::Center,
            TextAlignment::Right => glyph_brush::HorizontalAlign::Right,
        };

        let bounds_size = {
            let width = if bounds_size.x >= 1. {
                bounds_size.x
            } else {
                f32::INFINITY
            };

            let height = if bounds_size.y >= 1. {
                bounds_size.y
            } else {
                f32::INFINITY
            };

            (width, height)
        };

        let owned_section = owned_section
            .with_layout(
                glyph_brush::Layout::default()
                    .h_align(h_align)
                    .v_align(glyph_brush::VerticalAlign::Center)
                    .line_breaker(glyph_brush::BuiltInLineBreaker::UnicodeLineBreaker),
            )
            .with_bounds(bounds_size)
            .to_owned();

        self.sections.push(TextBrushSection {
            owned_section,
            section_transform: text_matrix,
            draw_instances_range: 0..0,
        });
    }

    pub fn process_sections(
        &mut self,
        device: &Device,
        queue: &Queue,
        gpu_interface: &mut GpuInterface,
    ) -> Result<()> {
        if self.sections.is_empty() {
            return Ok(());
        }

        for section in &mut self.sections {
            self.glyph_brush.queue(&section.owned_section);
            loop {
                let brush_action = self.glyph_brush.process_queued(
                    |rectangle, data| {
                        queue.write_texture(
                            wgpu::ImageCopyTexture {
                                texture: &self.text_atlas.texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: rectangle.min[0],
                                    y: rectangle.min[1],
                                    z: 0,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            data,
                            wgpu::ImageDataLayout {
                                offset: 0,
                                bytes_per_row: Some(rectangle.width()),
                                rows_per_image: Some(rectangle.height()),
                            },
                            wgpu::Extent3d {
                                width: rectangle.width(),
                                height: rectangle.height(),
                                depth_or_array_layers: 1,
                            },
                        );
                    },
                    |vertex_data| vertex_data.into(),
                );

                match brush_action {
                    Ok(action) => {
                        break match action {
                            BrushAction::Draw(text_vertices) => {
                                let start_vertex = self.sections_vertices.len() as u32;
                                section.draw_instances_range =
                                    start_vertex..start_vertex + text_vertices.len() as u32;
                                self.sections_vertices.extend(text_vertices);
                            }
                            BrushAction::ReDraw => {}
                        };
                    }
                    Err(BrushError::TextureTooSmall { suggested }) => {
                        log::warn!("TextBrush::process_sections() Resizing cache texture.");

                        let max_image_dimension = device.limits().max_texture_dimension_2d;
                        let (width, height) = {
                            if suggested.0 > max_image_dimension
                                || suggested.1 > max_image_dimension
                            {
                                if self.glyph_brush.texture_dimensions().0 < max_image_dimension
                                    || self.glyph_brush.texture_dimensions().1 < max_image_dimension
                                {
                                    (max_image_dimension, max_image_dimension)
                                } else {
                                    return Err(anyhow!(
                                        "TextBrush::process_sections() Requested texture size ({}, {}) larger than device max ({}, {})",
                                        suggested.0,
                                        suggested.1,
                                        max_image_dimension,
                                        max_image_dimension
                                    ));
                                }
                            } else {
                                suggested
                            }
                        };
                        let next_texture_id = gpu_interface
                            .texture_asset_manager
                            .register_next_texture_id();
                        let internal_texture = EngineTexture::new(
                            next_texture_id,
                            &format!("TextPipeline::text_atlas{next_texture_id}").into(),
                            width as usize,
                            height as usize,
                            false,
                        );
                        gpu_interface
                            .texture_asset_manager
                            .insert_engine_texture(&internal_texture)
                            .unwrap();
                        self.text_atlas = Texture::new_empty(
                            "TextPipeline::text_atlas",
                            device,
                            (width, height),
                            1,
                            next_texture_id,
                            TextureFormat::R8Unorm,
                            false,
                        );
                        self.bind_group = TextBrush::generate_bind_group(
                            device,
                            &self.uniform_buffer,
                            &self.text_atlas,
                        )
                        .0;
                        self.glyph_brush.resize_texture(width, height);
                    }
                }
            }
        }

        Ok(())
    }

    pub fn write_uniforms(
        &self,
        queue: &Queue,
        view_proj_matrix: &Mat4,
        dynamic_uniform_offset: BufferAddress,
    ) {
        let brush_uniforms = self
            .sections
            .iter()
            .map(|section| TextBrushUniform {
                transform: (*view_proj_matrix) * section.section_transform,
                padding: Default::default(),
            })
            .collect::<Vec<_>>();

        queue.write_buffer(
            &self.uniform_buffer,
            dynamic_uniform_offset,
            cast_slice(&brush_uniforms),
        );
        queue.write_buffer(&self.vertex_buffer, 0, cast_slice(&self.sections_vertices));
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TextVertex {
    pub color: Color,
    pub top_left: Vec3,
    pub bottom_right: Vec2,
    pub tex_start_uvs: Vec2,
    pub tex_end_uvs: Vec2,
    _padding: [f32; 3],
}

impl From<GlyphVertex<'_>> for TextVertex {
    fn from(
        GlyphVertex {
            mut tex_coords,
            pixel_coords,
            bounds,
            extra,
        }: GlyphVertex<'_>,
    ) -> Self {
        let mut rect = ab_glyph::Rect {
            min: ab_glyph::point(pixel_coords.min.x, pixel_coords.min.y),
            max: ab_glyph::point(pixel_coords.max.x, pixel_coords.max.y),
        };
        if rect.max.x > bounds.max.x {
            let old_width = rect.width();
            rect.max.x = bounds.max.x;
            tex_coords.max.x = tex_coords.min.x + tex_coords.width() * rect.width() / old_width;
        }
        if rect.min.x < bounds.min.x {
            let old_width = rect.width();
            rect.min.x = bounds.min.x;
            tex_coords.min.x = tex_coords.max.x - tex_coords.width() * rect.width() / old_width;
        }
        if rect.max.y > bounds.max.y {
            let old_height = rect.height();
            rect.max.y = bounds.max.y;
            tex_coords.max.y = tex_coords.min.y + tex_coords.height() * rect.height() / old_height;
        }
        if rect.min.y < bounds.min.y {
            let old_height = rect.height();
            rect.min.y = bounds.min.y;
            tex_coords.min.y = tex_coords.max.y - tex_coords.height() * rect.height() / old_height;
        }

        Self {
            // because our world-space coordinate system is y-up, but glyph-brush vertex region
            // coordinates are y-down, negate the y values for `top_left` and `bottom_right`
            top_left: Vec3::new(rect.min.x, -rect.min.y, extra.z),
            bottom_right: Vec2::new(rect.max.x, -rect.max.y),
            tex_start_uvs: Vec2::new(tex_coords.min.x, tex_coords.min.y),
            tex_end_uvs: Vec2::new(tex_coords.max.x, tex_coords.max.y),
            color: extra.color.into(),
            _padding: [0.0; 3],
        }
    }
}

impl TextVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<TextVertex>() as BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                VertexAttribute {
                    // color
                    format: VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 0,
                },
                VertexAttribute {
                    // top_left
                    format: VertexFormat::Float32x3,
                    offset: size_of::<[f32; 4]>() as BufferAddress,
                    shader_location: 1,
                },
                VertexAttribute {
                    // bottom_right
                    format: VertexFormat::Float32x2,
                    offset: size_of::<[f32; 4 + 3]>() as BufferAddress,
                    shader_location: 2,
                },
                VertexAttribute {
                    // uv_top_left
                    format: VertexFormat::Float32x2,
                    offset: size_of::<[f32; 4 + 3 + 2]>() as BufferAddress,
                    shader_location: 3,
                },
                VertexAttribute {
                    // uv_bottom_right
                    format: VertexFormat::Float32x2,
                    offset: size_of::<[f32; 4 + 3 + 2 + 2]>() as BufferAddress,
                    shader_location: 4,
                },
            ],
        }
    }
}
