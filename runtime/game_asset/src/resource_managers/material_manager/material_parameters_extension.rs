use std::iter::once;

use anyhow::{Result, anyhow, bail};
use void_public::{
    graphics::TextureId,
    material::{MaterialParameters, TEXTURE_LIMIT, UNIFORM_LIMIT},
};

use super::{
    textures::MaterialTextures,
    uniforms::{MaterialUniforms, UniformType, UniformValue},
};
use crate::ecs_module::MaterialManager;

/// This is an extension trait for adding Rust only functionality to
/// [`MaterialParameters`]. It is focused on adding means of correctly
/// manipulating it's uniforms and texture buffers. There are internal rules
/// around uniform and texture order, and these helpers allow a user to focus on
/// updating uniforms and textures correctly without directly interacting with
/// the buffers.
pub trait MaterialParametersExt {
    fn as_material_uniforms(&self, material_manager: &MaterialManager) -> Result<MaterialUniforms>;

    fn update_uniforms_from_iter<'a, S, I>(
        &mut self,
        material_manager: &MaterialManager,
        iterator: I,
    ) -> Result<&mut MaterialParameters>
    where
        I: IntoIterator<Item = &'a (S, &'a UniformValue)>,
        S: AsRef<str> + 'a;

    fn update_from_material_uniforms(&mut self, material_uniforms: &MaterialUniforms)
    -> Result<()>;

    fn update_uniforms<S>(
        &mut self,
        material_manager: &MaterialManager,
        uniform_name_to_uniform_value_slice: &[(S, &UniformValue)],
    ) -> Result<&mut MaterialParameters>
    where
        S: AsRef<str>;

    fn update_uniform<S>(
        &mut self,
        material_manager: &MaterialManager,
        uniform_name_to_uniform_value: &(S, &UniformValue),
    ) -> Result<&mut MaterialParameters>
    where
        S: AsRef<str>;

    fn as_material_textures(&self, material_manager: &MaterialManager) -> Result<MaterialTextures>;

    fn update_textures_from_iter<'a, S, I>(
        &mut self,
        material_manager: &MaterialManager,
        iterator: I,
    ) -> Result<&mut MaterialParameters>
    where
        I: IntoIterator<Item = &'a (S, &'a TextureId)>,
        S: AsRef<str> + 'a;

    fn update_from_material_textures(&mut self, material_textures: &MaterialTextures)
    -> Result<()>;

    fn update_textures<S>(
        &mut self,
        material_manager: &MaterialManager,
        material_spec_to_texture_id_slice: &[(S, &TextureId)],
    ) -> Result<&mut MaterialParameters>
    where
        S: AsRef<str>;

    fn update_texture<S>(
        &mut self,
        material_manager: &MaterialManager,
        material_spec_to_texture_id: &(S, &TextureId),
    ) -> Result<&mut MaterialParameters>
    where
        S: AsRef<str>;

    fn end_chain(&mut self) -> MaterialParameters;
}

impl MaterialParametersExt for MaterialParameters {
    /// Converts [`MaterialParameters`] data field into [`MaterialUniforms`]
    ///
    /// # Errors
    ///
    /// Returns an error if the [`MaterialParameters`]'s [`MaterialId`] is not
    /// found. This can only occur if a user manually creates a
    /// [`MaterialParameters`] outside of the [`Material`] API
    fn as_material_uniforms(&self, material_manager: &MaterialManager) -> Result<MaterialUniforms> {
        let material = material_manager
            .get_material(self.material_id())
            .ok_or(anyhow!(
                "Material {} not found in Material Manager",
                self.material_id()
            ))?;
        material.get_current_uniforms(self.data.as_slice())
    }

    /// Update's a [`MaterialParameters`] data buffer to reflect an input
    /// [`MaterialUniforms`]
    ///
    /// # Errors
    ///
    /// * An error will occur if the input [`MaterialUniforms`] has a different
    ///   [`MaterialId`] than the [`MaterialParameters`]
    /// * NO ERROR will occur if the [`MaterialUniforms`] layout does not match
    ///   what the given [`Material`] expects. If one does not get
    ///   [`Material::get_current_uniforms`] or
    ///   [`MaterialParameters::as_material_uniforms`], then they run the risk of
    ///   having a malformed buffer
    fn update_from_material_uniforms(
        &mut self,
        material_uniforms: &MaterialUniforms,
    ) -> Result<()> {
        if material_uniforms.material_id() != self.material_id() {
            bail!(
                "Attempted to update MaterialParameters of material {} from MaterialUniforms of material {}",
                self.material_id(),
                material_uniforms.material_id()
            );
        }
        let mut uniform_buffer_slice = self.data.as_mut_slice();

        for (_, uniform) in material_uniforms.uniforms_map.iter() {
            let uniform_size = UniformType::doubleword_size([uniform.uniform_type()]);
            let uniform_buffer_slice_replace_slice = &mut uniform_buffer_slice[..uniform_size];
            let uniform_value_as_buffer = uniform.as_f32_buffer();
            if uniform_buffer_slice_replace_slice.len() != uniform_value_as_buffer.len() {
                // This should never happen, it means we have allowed a material with uniforms larger than UNIFORM_LIMIT
                bail!("MaterialUniforms overflowed {UNIFORM_LIMIT} limit");
            }
            uniform_buffer_slice_replace_slice.copy_from_slice(&uniform.as_f32_buffer());
            uniform_buffer_slice = &mut uniform_buffer_slice[uniform_size..];
        }
        Ok(())
    }

    /// Updates the data buffer from human readable uniform names paired with
    /// their [`UniformValue`]s. The available names and their [`UniformType`]s
    /// will be listed in the shader's TOML file. You might want to use
    /// [`MaterialParameters::update_uniform`] or
    /// [`MaterialParameters::update_uniforms`] unless you are inputing an
    /// iterator
    ///
    /// # Example
    ///
    /// ```
    /// use game_asset::{ecs_module::MaterialManager, resource_managers::material_manager::{fixed_size_vec::FixedSizeVec, material_parameters_extension::MaterialParametersExt}};
    /// use void_public::{material::{MaterialId, MaterialParameters}, Vec4};
    ///
    /// let material_manager = MaterialManager::default(); // &MaterialManager should be accessed as a resource from a system
    /// let mut material_parameters = MaterialParameters::new(MaterialId(3));
    /// let _ = material_parameters.update_uniforms_from_iter(
    ///     &material_manager,
    ///     &[
    ///         ("uniform_1", &8.0.into()),
    ///         ("uniform_2", &Vec4::new(1.0, 2.0, 3.0, 4.0).into()),
    ///         ("uniform_3", &FixedSizeVec::new(
    ///             &[
    ///                 Vec4::new(0.1, 0.2, 0.3, 0.4),
    ///                 Vec4::new(0.5, 0.6, 0.7, 0.8),
    ///                 Vec4::new(0.9, 1.0, 1.1, 1.2),
    ///             ])
    ///             .into())
    ///     ]
    /// );
    /// ```
    /// # Errors
    ///
    /// * Same errors as [`MaterialParameters::as_material_uniforms`]
    /// * Same errors as [`MaterialParameters::update_from_material_uniforms`]
    fn update_uniforms_from_iter<'a, S, I>(
        &mut self,
        material_manager: &MaterialManager,
        iterator: I,
    ) -> Result<&mut MaterialParameters>
    where
        I: IntoIterator<Item = &'a (S, &'a UniformValue)>,
        S: AsRef<str> + 'a,
    {
        let mut material_uniforms = self.as_material_uniforms(material_manager)?;
        for (name, value) in iterator {
            let name = name.as_ref();
            let value = (*value).clone();
            material_uniforms.update(name, value)?;
        }
        self.update_from_material_uniforms(&material_uniforms)?;
        Ok(self)
    }

    /// Updates the data buffer based human readable names paired with their
    /// [`UniformValue`] in a slice.
    ///
    ///
    /// # Example
    ///
    /// ```
    /// use game_asset::{ecs_module::MaterialManager, resource_managers::material_manager::{fixed_size_vec::FixedSizeVec, material_parameters_extension::MaterialParametersExt}};
    /// use void_public::{material::{MaterialId, MaterialParameters}, Vec4};
    ///
    /// let material_manager = MaterialManager::default(); // &MaterialManager should be accessed as a resource from a system
    /// let mut material_parameters = MaterialParameters::new(MaterialId(3));
    /// let _ = material_parameters.update_uniforms(
    ///     &material_manager,
    ///     &[
    ///         ("uniform_1", &8.0.into()),
    ///         ("uniform_2", &Vec4::new(1.0, 2.0, 3.0, 4.0).into()),
    ///         ("uniform_3", &FixedSizeVec::new(
    ///             &[
    ///                 Vec4::new(0.1, 0.2, 0.3, 0.4),
    ///                 Vec4::new(0.5, 0.6, 0.7, 0.8),
    ///                 Vec4::new(0.9, 1.0, 1.1, 1.2),
    ///             ])
    ///             .into())
    ///     ]
    /// );
    /// ```
    /// # Errors
    ///
    /// * Same errors as [`MaterialParameters::as_material_uniforms`]
    /// * Same errors as [`MaterialParameters::update_from_material_uniforms`]
    fn update_uniforms<S>(
        &mut self,
        material_manager: &MaterialManager,
        uniform_name_to_uniform_value_slice: &[(S, &UniformValue)],
    ) -> Result<&mut MaterialParameters>
    where
        S: AsRef<str>,
    {
        self.update_uniforms_from_iter(material_manager, uniform_name_to_uniform_value_slice)?;
        Ok(self)
    }

    /// Updates the data buffer based on a single human readable names paired
    /// with a [`UniformValue`]
    ///
    ///
    /// # Example
    ///
    /// ```
    /// use game_asset::{ecs_module::MaterialManager, resource_managers::material_manager::{fixed_size_vec::FixedSizeVec, material_parameters_extension::MaterialParametersExt}};
    /// use void_public::{material::{MaterialId, MaterialParameters}, Vec4};
    ///
    /// let material_manager = MaterialManager::default(); // &MaterialManager should be accessed as a resource from a system
    /// let mut material_parameters = MaterialParameters::new(MaterialId(3));
    /// let _ = material_parameters.update_uniform(
    ///     &material_manager,
    ///     &("uniform_1", &Vec4::new(1.0, 2.0, 3.0, 4.0).into()),
    /// );
    /// ```
    /// # Errors
    ///
    /// * Same errors as [`MaterialParameters::as_material_uniforms`]
    /// * Same errors as [`MaterialParameters::update_from_material_uniforms`]
    fn update_uniform<S>(
        &mut self,
        material_manager: &MaterialManager,
        uniform_name_to_uniform_value: &(S, &UniformValue),
    ) -> Result<&mut MaterialParameters>
    where
        S: AsRef<str>,
    {
        self.update_uniforms_from_iter(material_manager, once(uniform_name_to_uniform_value))?;
        Ok(self)
    }

    /// Converts [`MaterialParameters`] data field into [`MaterialTextures`]
    ///
    /// # Errors
    ///
    /// Returns an error if the [`MaterialParameters`]'s [`MaterialId`] is not
    /// found. This can only occur if a user manually creates a
    /// [`MaterialParameters`] outside of the [`Material`] API
    fn as_material_textures(&self, material_manager: &MaterialManager) -> Result<MaterialTextures> {
        let material = material_manager
            .get_material(self.material_id())
            .ok_or(anyhow!(
                "Material {} not found in Material Manager",
                self.material_id()
            ))?;
        material.get_current_textures(&self.textures)
    }

    /// Updates the data buffer from human readable texture names paired with
    /// their [`TextureId`]s. The available names will be listed in the shader's
    /// TOML file. You might want to use [`MaterialParameters::update_texture`]
    /// or [`MaterialParameters::update_textures`] unless you are inputing an
    /// iterator
    ///
    /// # Example
    ///
    /// ```
    /// use game_asset::{ecs_module::MaterialManager, resource_managers::material_manager::material_parameters_extension::MaterialParametersExt};
    /// use void_public::{graphics::TextureId, material::{MaterialId, MaterialParameters}};
    ///
    /// let material_manager = MaterialManager::default(); // &MaterialManager should be accessed as a resource from a system
    /// let mut material_parameters = MaterialParameters::new(MaterialId(3));
    /// material_parameters.update_textures_from_iter(
    ///     &material_manager,
    ///     &[
    ///         ("texture_1", &TextureId(3)), // TextureIds will probably be from TextureAssetManager
    ///         ("texture_2", &TextureId(4)),
    ///     ]
    /// );
    /// ```
    /// # Errors
    ///
    /// * Same errors as [`MaterialParameters::as_material_textures`]
    /// * Same errors as [`MaterialParameters::update_from_material_textures`]
    fn update_textures_from_iter<'a, S, I>(
        &mut self,
        material_manager: &MaterialManager,
        iterator: I,
    ) -> Result<&mut MaterialParameters>
    where
        I: IntoIterator<Item = &'a (S, &'a TextureId)>,
        S: AsRef<str> + 'a,
    {
        let mut material_textures = self.as_material_textures(material_manager)?;
        for (name, texture_id) in iterator {
            let name = name.as_ref();
            material_textures.update(name, **texture_id)?;
        }
        self.update_from_material_textures(&material_textures)?;

        Ok(self)
    }

    /// Update's a [`MaterialParameters`] data buffer to reflect an input
    /// [`MaterialTextures`]
    ///
    /// # Errors
    ///
    /// * An error will occur if the input [`MaterialTextures`] has a different
    ///   [`MaterialId`] than the [`MaterialParameters`]
    /// * NO ERROR will occur if the [`MaterialTextures`] layout does not match
    ///   what the given [`Material`] expects. If one does not get
    ///   [`Material::get_current_textures`] or
    ///   [`MaterialParameters::as_material_textures`], then they run the risk of
    ///   having a malformed buffer
    fn update_from_material_textures(
        &mut self,
        material_textures: &MaterialTextures,
    ) -> Result<()> {
        if material_textures.material_id() != self.material_id() {
            bail!(
                "Attempted to update MaterialParameters of material {} from MaterialUniforms of material {}",
                self.material_id(),
                material_textures.material_id()
            );
        }
        for (index, (_, updated_texture_id)) in
            material_textures.iter().take(TEXTURE_LIMIT).enumerate()
        {
            self.textures[index] = *updated_texture_id;
        }
        Ok(())
    }

    /// Updates the data buffer from human readable texture names paired with
    /// their [`TextureId`]s in a slice
    ///
    /// # Example
    ///
    /// ```
    /// use game_asset::{ecs_module::MaterialManager, resource_managers::material_manager::material_parameters_extension::MaterialParametersExt};
    /// use void_public::{graphics::TextureId, material::{MaterialId, MaterialParameters}};
    ///
    /// let material_manager = MaterialManager::default(); // &MaterialManager should be accessed as a resource from a system
    /// let mut material_parameters = MaterialParameters::new(MaterialId(3));
    /// material_parameters.update_textures(
    ///     &material_manager,
    ///     &[
    ///         ("texture_1", &TextureId(3)), // TextureIds will probably be from TextureAssetManager
    ///         ("texture_2", &TextureId(4)),
    ///     ]
    /// );
    /// ```
    /// # Errors
    ///
    /// * Same errors as [`MaterialParameters::as_material_textures`]
    /// * Same errors as [`MaterialParameters::update_from_material_textures`]
    fn update_textures<S>(
        &mut self,
        material_manager: &MaterialManager,
        material_spec_to_texture_id_slice: &[(S, &TextureId)],
    ) -> Result<&mut MaterialParameters>
    where
        S: AsRef<str>,
    {
        self.update_textures_from_iter(material_manager, material_spec_to_texture_id_slice)?;
        Ok(self)
    }

    /// Updates the data buffer from human readable texture name paired with a
    /// [`TextureId`]
    ///
    /// # Example
    ///
    /// ```
    /// use game_asset::{ecs_module::MaterialManager, resource_managers::material_manager::material_parameters_extension::MaterialParametersExt};
    /// use void_public::{graphics::TextureId, material::{MaterialId, MaterialParameters}};
    ///
    /// let material_manager = MaterialManager::default(); // &MaterialManager should be accessed as a resource from a system
    /// let mut material_parameters = MaterialParameters::new(MaterialId(3));
    /// material_parameters.update_texture(
    ///     &material_manager,
    ///     &("texture_1", &TextureId(3)), // TextureIds will probably be from TextureAssetManager
    /// );
    /// ```
    /// # Errors
    ///
    /// * Same errors as [`MaterialParameters::as_material_textures`]
    /// * Same errors as [`MaterialParameters::update_from_material_textures`]
    fn update_texture<S>(
        &mut self,
        material_manager: &MaterialManager,
        material_spec_to_texture_id: &(S, &TextureId),
    ) -> Result<&mut MaterialParameters>
    where
        S: AsRef<str>,
    {
        self.update_textures_from_iter(material_manager, once(material_spec_to_texture_id))?;
        Ok(self)
    }

    /// Helper function so that you can initialize a struct, call some chaining
    /// functions, and then return self for convenience
    fn end_chain(&mut self) -> MaterialParameters {
        *self
    }
}
