use std::{borrow::Cow, collections::HashMap, path::Path};

use anyhow::{Error, Result};
use game_asset::{
    ecs_module::GpuInterface,
    resource_managers::texture_asset_manager::{
        EngineTexture, MISSING_TEXTURE_TEXTURE_ID, TextureAssetManager,
    },
};
use strum::{Display, EnumCount, EnumIter, IntoEnumIterator};
use void_public::graphics::TextureId;
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, Device, Queue, RenderPass, SurfaceConfiguration, TextureFormat,
    TextureUsages,
};

#[derive(Clone, Copy, Debug)]
pub struct AtlasRegion {
    pub top_left: (u32, u32),
    pub bottom_right: (u32, u32),
}

impl AtlasRegion {
    pub fn width(&self) -> u32 {
        self.bottom_right.0 - self.top_left.0
    }

    pub fn height(&self) -> u32 {
        self.bottom_right.1 - self.top_left.1
    }
}

#[derive(Debug)]
pub struct TextureAtlas {
    atlas_texture_id: TextureId, // Currently set to the TextureId of the first texture added to this atlas
    texture_to_region: HashMap<TextureId, AtlasRegion>,
    allocated_regions: Vec<AtlasRegion>,
    free_regions: Vec<AtlasRegion>,
}

impl TextureAtlas {
    pub fn new(atlas_texture_id: TextureId) -> Self {
        TextureAtlas {
            atlas_texture_id,
            texture_to_region: HashMap::<TextureId, AtlasRegion>::new(),
            allocated_regions: vec![],
            free_regions: vec![AtlasRegion {
                top_left: (0, 0),
                bottom_right: (
                    GpuTextureManager::atlas_width_height(),
                    GpuTextureManager::atlas_width_height(),
                ),
            }],
        }
    }

    pub fn get_atlas_texture_id(&self) -> TextureId {
        self.atlas_texture_id
    }

    pub fn get_region(&self, texture_id: &TextureId) -> Option<&AtlasRegion> {
        self.texture_to_region.get(texture_id)
    }

    pub fn get_region_uv_scale_offset(&self, texture_id: &TextureId) -> Option<[f32; 4]> {
        let region = self.get_region(texture_id)?;
        let atlas_size = GpuTextureManager::atlas_width_height() as f32;
        Some([
            region.width() as f32 / atlas_size,
            region.height() as f32 / atlas_size,
            region.top_left.0 as f32 / atlas_size,
            region.top_left.1 as f32 / atlas_size,
        ])
    }

    pub fn load_texture_into_atlas(
        &mut self,
        texture_image: &image::DynamicImage,
        texture_name: &str,
        texture_id: TextureId,
    ) -> Result<(u32, u32, u32, u32)> {
        let Some(i) = self.free_regions.iter().position(|region| {
            region.width() >= texture_image.width() && region.height() >= texture_image.height()
        }) else {
            return Err(Error::msg("Out of space"));
        };

        let source_region = self.free_regions.remove(i);

        let new_region = AtlasRegion {
            top_left: source_region.top_left,
            bottom_right: (
                source_region.top_left.0 + texture_image.width(),
                source_region.top_left.1 + texture_image.height(),
            ),
        };
        self.texture_to_region.insert(texture_id, new_region);

        let msg = format!(
            "Adding {texture_name} with dimensions {}x{} into atlas region {},{} with texture id {texture_id}",
            texture_image.width(),
            texture_image.height(),
            new_region.top_left.0,
            new_region.top_left.1,
        );
        log::info!("{}", msg);

        if source_region.bottom_right.1 - new_region.bottom_right.1 > 8 {
            let source_split = AtlasRegion {
                top_left: (source_region.top_left.0, new_region.bottom_right.1),
                bottom_right: (source_region.bottom_right.0, source_region.bottom_right.1),
            };
            self.free_regions.push(source_split);

            log::trace!(
                "New Free Block at {},{} - {},{}",
                source_split.top_left.0,
                source_split.top_left.1,
                source_split.bottom_right.0,
                source_split.bottom_right.1
            );
        }

        if source_region.bottom_right.0 - new_region.bottom_right.0 > 8 {
            let source_split = AtlasRegion {
                top_left: (new_region.bottom_right.0, new_region.top_left.1),
                bottom_right: (source_region.bottom_right.0, new_region.bottom_right.1),
            };
            log::trace!(
                "New Free Block at {},{} - {},{}",
                source_split.top_left.0,
                source_split.top_left.1,
                source_split.bottom_right.0,
                source_split.bottom_right.1
            );

            self.free_regions.push(source_split);
        }
        self.allocated_regions.push(new_region);
        Ok((
            new_region.top_left.0,
            new_region.top_left.1,
            texture_image.width(),
            texture_image.height(),
        ))
    }
}

#[derive(Debug)]
pub struct TextureMetaData {
    texture_id: TextureId,
    width: u32,
    height: u32,
}

impl TextureMetaData {
    pub fn texture_id(&self) -> TextureId {
        self.texture_id
    }
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

#[derive(Debug)]
pub struct Texture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    metadata: TextureMetaData,
}

impl Texture {
    pub fn metadata(&self) -> &TextureMetaData {
        &self.metadata
    }
    pub fn from_bytes(
        texture_bytes: &[u8],
        texture_id: TextureId,
        filename: &str,
        width_and_height: (u32, u32),
        device: &Device,
        queue: &Queue,
    ) -> Result<Self> {
        let (texture_format, width_and_height, bytes_per_row, texture_bytes) = {
            match Path::new(filename).extension() {
                Some(os_str) if os_str == "dxt1" => (
                    TextureFormat::Bc1RgbaUnorm,
                    (
                        width_and_height.0.next_multiple_of(4),
                        width_and_height.1.next_multiple_of(4),
                    ),
                    width_and_height.0.next_multiple_of(4) * 2,
                    Cow::from(texture_bytes),
                ),
                Some(os_str) if os_str == "dxt4" => (
                    TextureFormat::Bc3RgbaUnorm,
                    (
                        width_and_height.0.next_multiple_of(4),
                        width_and_height.1.next_multiple_of(4),
                    ),
                    width_and_height.0.next_multiple_of(4) * 4,
                    Cow::from(texture_bytes),
                ),
                _ => {
                    let texture_image = image::load_from_memory(texture_bytes)?;
                    (
                        TextureFormat::Rgba8Unorm,
                        (texture_image.width(), texture_image.height()),
                        4 * texture_image.width(),
                        Cow::from(texture_image.to_rgba8().to_vec()),
                    )
                }
            }
        };

        let new_texture = Texture::new_empty(
            filename,
            device,
            width_and_height,
            1,
            texture_id,
            texture_format,
            false,
        );

        queue.write_texture(
            wgpu::ImageCopyTexture {
                aspect: wgpu::TextureAspect::All,
                texture: &new_texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
            },
            texture_bytes.as_ref(),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: width_and_height.0,
                height: width_and_height.1,
                depth_or_array_layers: 1,
            },
        );
        Ok(new_texture)
    }

    pub fn new_empty(
        label: &str,
        device: &Device,
        texture_dimensions: (u32, u32),
        sample_count: u32,
        texture_id: TextureId,
        format: TextureFormat,
        is_target: bool,
    ) -> Self {
        let usage = if is_target {
            // render targets may need to be read from, so they need the COPY_SRC and COPY_DEST flags
            TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST
        } else {
            TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: texture_dimensions.0,
                height: texture_dimensions.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Texture {
            texture,
            view,
            sampler,
            metadata: TextureMetaData {
                texture_id,
                width: texture_dimensions.0,
                height: texture_dimensions.1,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Display, EnumCount, EnumIter, PartialEq, Eq)]
pub enum RenderTargetType {
    ColorMsaa,
    ColorResolve,
    ColorDownSample1x,
    ColorDownSample2x,
    PostProcess,
    DownSample2xScratch,
}

#[derive(Debug)]
pub struct DownsampleOptions {
    pub name: &'static str,
    pub render_target_texture_index: RenderTargetType,
    pub sampled_render_target_texture_index: RenderTargetType,
}

#[derive(Debug)]
pub struct GpuTextureManager {
    asset_to_texture: HashMap<TextureId, Texture>,
    texture_atlases: Vec<TextureAtlas>,
    target_to_texture_id: Vec<TextureId>,
    tex_bind_group_cache: HashMap<Vec<TextureId>, BindGroup>,
    camera_textures: Vec<(u32, Texture)>,
    next_camera_texture_id: u32,
}

const WHITE_TEXTURE_BYTES: [u32; 4] = [0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff];
const MISSING_TEXTURE_BYTES: [u32; 4] = [0xffff00ff, 0xffff00ff, 0xffff00ff, 0xffff00ff];

impl GpuTextureManager {
    pub fn new(
        gpu_interface: &mut GpuInterface,
        device: &Device,
        queue: &Queue,
        config: &SurfaceConfiguration,
    ) -> Self {
        let white_texture = Texture::new_empty(
            "GpuTextureManager::white_texture",
            device,
            (2, 2),
            1,
            TextureAssetManager::white_texture_id(),
            TextureFormat::Rgba8Unorm,
            false,
        );
        queue.write_texture(
            wgpu::ImageCopyTexture {
                aspect: wgpu::TextureAspect::All,
                texture: &white_texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
            },
            bytemuck::cast_slice(WHITE_TEXTURE_BYTES.as_slice()),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * 2),
                rows_per_image: Some(2),
            },
            wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
        );

        let missing_texture = Texture::new_empty(
            "GpuTextureManager::missing_texture",
            device,
            (2, 2),
            1,
            TextureAssetManager::missing_texture_id(),
            TextureFormat::Rgba8Unorm,
            false,
        );
        queue.write_texture(
            wgpu::ImageCopyTexture {
                aspect: wgpu::TextureAspect::All,
                texture: &missing_texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
            },
            bytemuck::cast_slice(MISSING_TEXTURE_BYTES.as_slice()),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * 2),
                rows_per_image: Some(2),
            },
            wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
        );

        let mut asset_to_texture = HashMap::<TextureId, Texture>::new();
        asset_to_texture.insert(TextureAssetManager::white_texture_id(), white_texture);
        asset_to_texture.insert(TextureAssetManager::missing_texture_id(), missing_texture);

        let mut gpu_texture_manager = GpuTextureManager {
            asset_to_texture,
            texture_atlases: Default::default(),
            target_to_texture_id: vec![],
            tex_bind_group_cache: HashMap::<Vec<TextureId>, BindGroup>::new(),
            next_camera_texture_id: 0,
            camera_textures: Vec::new(),
        };

        let target_to_texture_id =
            gpu_texture_manager.create_render_targets(gpu_interface, device, config);
        gpu_texture_manager.target_to_texture_id = target_to_texture_id;

        gpu_texture_manager
    }

    pub fn load_texture(
        &mut self,
        texture_bytes: &[u8],
        texture_id: TextureId,
        texture_name: &str,
        width_and_height: (u32, u32),
        device: &Device,
        queue: &Queue,
    ) -> Result<(u32, u32)> {
        let texture = Texture::from_bytes(
            texture_bytes,
            texture_id,
            texture_name,
            width_and_height,
            device,
            queue,
        )?;
        let width_and_height = (texture.metadata.width, texture.metadata.height);
        self.asset_to_texture.insert(texture_id, texture);

        log::info!("Loaded {texture_name} with texture id {texture_id}");

        Ok(width_and_height)
    }

    pub fn get_texture(&self, texture_id: TextureId) -> Option<&Texture> {
        self.asset_to_texture.get(&texture_id).or_else(|| {
            for texture_atlas in &self.texture_atlases {
                if texture_atlas.texture_to_region.contains_key(&texture_id) {
                    return self.asset_to_texture.get(&texture_atlas.atlas_texture_id);
                }
            }

            None
        })
    }

    pub fn get_texture_or_missing(&self, handle: TextureId) -> &Texture {
        if let Some(texture) = self.get_texture(handle) {
            texture
        } else {
            self.asset_to_texture
                .get(&TextureAssetManager::missing_texture_id())
                .unwrap()
        }
    }

    pub fn atlas_region(&self, texture_id: TextureId) -> Option<&AtlasRegion> {
        for texture_atlas in &self.texture_atlases {
            if let Some(texture_atlas) = texture_atlas.get_region(&texture_id) {
                return Some(texture_atlas);
            }
        }
        None
    }

    pub fn uv_scale_offset(&self, texture_id: TextureId) -> Option<[f32; 4]> {
        for texture_atlas in &self.texture_atlases {
            if let Some(uv_scale_offset) = texture_atlas.get_region_uv_scale_offset(&texture_id) {
                return Some(uv_scale_offset);
            }
        }
        None
    }

    pub fn load_texture_into_atlas(
        &mut self,
        texture_bytes: &[u8],
        texture_name: &str,
        texture_id: TextureId,
        device: &Device,
        queue: &Queue,
        gpu_interface: &mut GpuInterface,
    ) -> (u32, u32) {
        let texture_label = format!("Texture_Atlas_{}", self.texture_atlases.len());
        if self.texture_atlases.is_empty() {
            let next_texture_id = gpu_interface
                .texture_asset_manager
                .register_next_texture_id();
            let engine_texture = EngineTexture::new(
                next_texture_id,
                &texture_label.clone().into(),
                GpuTextureManager::atlas_width_height() as usize,
                GpuTextureManager::atlas_width_height() as usize,
                false,
            );
            gpu_interface
                .texture_asset_manager
                .insert_engine_texture(&engine_texture)
                .unwrap();

            let texture = Texture::new_empty(
                &texture_label,
                device,
                (
                    GpuTextureManager::atlas_width_height(),
                    GpuTextureManager::atlas_width_height(),
                ),
                1,
                texture_id,
                TextureFormat::Rgba8Unorm,
                false,
            );
            self.asset_to_texture.insert(texture_id, texture);
            self.texture_atlases.push(TextureAtlas::new(texture_id));
        }

        let texture_image = image::load_from_memory(texture_bytes).unwrap();

        let dimensions = match self
            .texture_atlases
            .last_mut()
            .unwrap()
            .load_texture_into_atlas(&texture_image, texture_name, texture_id)
        {
            Ok(dimensions) => dimensions,
            Err(error_0) => {
                let next_texture_id = gpu_interface
                    .texture_asset_manager
                    .register_next_texture_id();
                let internal_texture = EngineTexture::new(
                    next_texture_id,
                    &texture_label.clone().into(),
                    GpuTextureManager::atlas_width_height() as usize,
                    GpuTextureManager::atlas_width_height() as usize,
                    false,
                );
                gpu_interface
                    .texture_asset_manager
                    .insert_engine_texture(&internal_texture)
                    .unwrap();

                let texture = Texture::new_empty(
                    &texture_label,
                    device,
                    (
                        GpuTextureManager::atlas_width_height(),
                        GpuTextureManager::atlas_width_height(),
                    ),
                    1,
                    next_texture_id,
                    TextureFormat::Rgba8Unorm,
                    false,
                );
                self.asset_to_texture.insert(texture_id, texture);
                self.texture_atlases.push(TextureAtlas::new(texture_id));
                match self
                    .texture_atlases
                    .last_mut()
                    .unwrap()
                    .load_texture_into_atlas(&texture_image, texture_name, texture_id)
                {
                    Ok(dimensions) => dimensions,
                    Err(error_1) => {
                        panic!(
                            "GpuTextureManager::load_texture_into_atlas() - Failed adding {texture_name} with errors {:?} and {:?}",
                            error_0, error_1
                        );
                    }
                }
            }
        };

        let origin = wgpu::Origin3d {
            x: dimensions.0,
            y: dimensions.1,
            z: 0,
        };
        let atlas_texture = self
            .get_texture(self.texture_atlases.last().unwrap().atlas_texture_id)
            .unwrap();
        let rgba = texture_image.to_rgba8();
        queue.write_texture(
            wgpu::ImageCopyTexture {
                aspect: wgpu::TextureAspect::All,
                texture: &atlas_texture.texture,
                mip_level: 0,
                origin,
            },
            &rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * texture_image.width()),
                rows_per_image: Some(texture_image.height()),
            },
            wgpu::Extent3d {
                width: texture_image.width(),
                height: texture_image.height(),
                depth_or_array_layers: 1,
            },
        );

        (texture_image.width(), texture_image.height())
    }

    pub fn texture_atlas(&self, index: usize) -> &TextureAtlas {
        &self.texture_atlases[index]
    }

    pub const fn atlas_width_height() -> u32 {
        2048
    }

    pub fn create_render_targets(
        &mut self,
        gpu_interface: &mut GpuInterface,
        device: &Device,
        config: &wgpu::SurfaceConfiguration,
    ) -> Vec<TextureId> {
        let screen_dimensions = (config.width, config.height);
        let screen_dimensions_downsample1x = (config.width / 2, config.height / 2);
        let screen_dimensions_downsample2x = (config.width / 4, config.height / 4);

        let output = RenderTargetType::iter()
            .map(|target_type| {
                let (label, dimensions, sample_count) = match target_type {
                    RenderTargetType::ColorMsaa => {
                        ("GpuTextureManager::ColorMsaa", screen_dimensions, 4)
                    }
                    RenderTargetType::ColorResolve => {
                        ("GpuTextureManager::ColorResolve", screen_dimensions, 1)
                    }
                    RenderTargetType::PostProcess => {
                        ("GpuTextureManager::PostProcess", screen_dimensions, 1)
                    }
                    RenderTargetType::ColorDownSample1x => (
                        "GpuTextureManager::ColorDownSample1x",
                        screen_dimensions_downsample1x,
                        1,
                    ),
                    RenderTargetType::ColorDownSample2x => (
                        "GpuTextureManager::ColorDownSample2x",
                        screen_dimensions_downsample2x,
                        1,
                    ),
                    RenderTargetType::DownSample2xScratch => (
                        "GpuTextureManager::DownSample2xcratch",
                        screen_dimensions_downsample2x,
                        1,
                    ),
                };

                let texture_id = if let Some(texture_id) = gpu_interface
                    .texture_asset_manager
                    .get_engine_texture_id_from_path(&label.into())
                {
                    gpu_interface
                        .texture_asset_manager
                        .update_engine_texture(
                            texture_id,
                            dimensions.0 as usize,
                            dimensions.1 as usize,
                        )
                        .unwrap();
                    texture_id
                } else {
                    let texture_id = gpu_interface
                        .texture_asset_manager
                        .register_next_texture_id();
                    let internal_texture = EngineTexture::new(
                        texture_id,
                        &label.into(),
                        dimensions.0 as usize,
                        dimensions.1 as usize,
                        false,
                    );
                    gpu_interface
                        .texture_asset_manager
                        .insert_engine_texture(&internal_texture)
                        .unwrap();

                    self.target_to_texture_id.push(texture_id);
                    texture_id
                };

                let texture = Texture::new_empty(
                    label,
                    device,
                    dimensions,
                    sample_count,
                    texture_id,
                    config.format,
                    true,
                );

                self.asset_to_texture.insert(texture_id, texture);

                // Clear any bind groups created using this render target's asset_id
                self.tex_bind_group_cache.retain(|k, _| {
                    for cached_texture_id in k {
                        if *cached_texture_id == texture_id {
                            return false;
                        }
                    }
                    true
                });

                texture_id
            })
            .collect();

        assert_eq!(self.target_to_texture_id.len(), RenderTargetType::COUNT);

        output
    }

    pub fn add_camera_render_target(&mut self, device: &Device, width: u32, height: u32) -> u32 {
        let resolve_target = self.get_render_target(RenderTargetType::ColorResolve);

        let camera_texture_id = self.next_camera_texture_id;
        let texture = Texture::new_empty(
            &format!("GpuTextureManager::Camera::{}", camera_texture_id),
            device,
            (width, height),
            4,
            TextureId(0),
            resolve_target.texture.format(),
            true,
        );

        self.camera_textures.push((camera_texture_id, texture));
        self.next_camera_texture_id += 1;

        camera_texture_id
    }

    pub fn remove_camera_render_target(&mut self, camera_texture_id: u32) {
        let mut index_to_remove = None;
        self.camera_textures
            .iter()
            .enumerate()
            .for_each(|(i, (texture_id, _))| {
                if *texture_id == camera_texture_id {
                    index_to_remove = Some(i);
                }
            });

        if let Some(index_to_remove) = index_to_remove {
            self.camera_textures.swap_remove(index_to_remove);
            // Do we need to clean up any device texture memory here?
        }
    }

    pub fn camera_render_target(&self, camera_texture_id: u32) -> Option<&Texture> {
        self.camera_textures
            .iter()
            .find_map(|camera_render_target| {
                if camera_render_target.0 == camera_texture_id {
                    Some(&camera_render_target.1)
                } else {
                    None
                }
            })
    }

    pub fn get_render_target(&self, target_type: RenderTargetType) -> &Texture {
        let asset_id = self.target_to_texture_id[target_type as usize];
        self.asset_to_texture.get(&asset_id).unwrap()
    }

    pub fn get_render_target_id(&self, target_type: RenderTargetType) -> TextureId {
        self.target_to_texture_id[target_type as usize]
    }

    pub fn set_tex_bind_group(
        &mut self,
        group: u32,
        render_pass: &mut RenderPass<'_>,
        texture_list: &[TextureId],
        device: &Device,
    ) {
        let bind_group_entry = self.tex_bind_group_cache.get(texture_list);
        if let Some(bind_group) = bind_group_entry {
            render_pass.set_bind_group(group, bind_group, &[]);
            return;
        }

        let mut bind_group_layouts = Vec::<BindGroupLayoutEntry>::new();
        let mut bind_group_entries = Vec::<BindGroupEntry<'_>>::new();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let mut any_textures_missing = false;
        for (i, texture_id) in texture_list.iter().enumerate() {
            bind_group_layouts.push(BindGroupLayoutEntry {
                binding: (i * 2) as u32,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            });
            bind_group_layouts.push(BindGroupLayoutEntry {
                binding: (i * 2 + 1) as u32,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });

            let texture = self.get_texture_or_missing(*texture_id);

            if texture.metadata().texture_id() == MISSING_TEXTURE_TEXTURE_ID {
                any_textures_missing = true;
            }

            bind_group_entries.push(BindGroupEntry {
                binding: (i * 2) as u32,
                resource: wgpu::BindingResource::TextureView(&texture.view),
            });
            bind_group_entries.push(BindGroupEntry {
                binding: (i * 2 + 1) as u32,
                resource: wgpu::BindingResource::Sampler(&sampler),
            });
        }

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            entries: bind_group_layouts.as_slice(),
            label: Some("GpuTextureManager::bind_group_layout"),
        });
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            layout: &bind_group_layout,
            entries: bind_group_entries.as_slice(),
            label: Some("GpuTextureManager::bind_group"),
        });

        render_pass.set_bind_group(group, &bind_group, &[]);

        if !any_textures_missing {
            self.tex_bind_group_cache
                .insert(texture_list.to_vec(), bind_group);
        }
    }
}
