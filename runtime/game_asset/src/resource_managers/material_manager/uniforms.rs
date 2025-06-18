use std::{cmp::Ordering, iter::once, ops::Deref};

use anyhow::{Result, bail};
use glam::Vec4;
use indexmap::IndexMap;
use strum::Display;
use void_public::material::MaterialId;

use super::{fixed_size_vec::FixedSizeVec, materials::MaterialType};

pub type FfiUniformValue = void_public::material::UniformValue;

fn uniform_comparer(a: &(&str, &UniformType), b: &(&str, &UniformType)) -> Ordering {
    let uniform_type_comparison = a.1.cmp(b.1);
    if uniform_type_comparison == Ordering::Equal {
        a.0.cmp(b.0)
    } else {
        uniform_type_comparison
    }
}

pub fn sort_uniforms_by_name_and_type<'a, 'b>(
    uniforms: &'a IndexMap<String, UniformValue>,
) -> Vec<(&'b str, &'b UniformValue)>
where
    'a: 'b,
{
    let mut uniforms_as_vec: Vec<(&'b str, &'b UniformValue)> = Vec::from_iter(uniforms)
        .into_iter()
        .map(|(name, uniform)| (name.as_str(), uniform))
        .collect();
    uniforms_as_vec
        .sort_by(|a, b| uniform_comparer(&(a.0, &a.1.uniform_type()), &(b.0, &b.1.uniform_type())));
    uniforms_as_vec
}

/// The actual values of uniform, to eventually flow into a uniform buffer
#[derive(Debug, Clone, PartialEq)]
pub enum UniformValue {
    Array(UniformVar<FixedSizeVec<Vec4>>),
    F32(UniformVar<f32>),
    Vec4(UniformVar<Vec4>),
}

/// Possible types of uniforms. The [`UniformType::Array`] variant contains the
/// size of the array
#[derive(Clone, Copy, Debug, Display, PartialEq, Eq)]
pub enum UniformType {
    Array(usize),
    F32,
    Vec4,
}

impl PartialOrd for UniformType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// This orders the [`UniformType`] by alignment. Currently the order for alignment is
/// vec4 >> Array >> f32
impl Ord for UniformType {
    fn cmp(&self, other: &Self) -> Ordering {
        match self {
            UniformType::Array(first_array_size) => match other {
                UniformType::Array(second_array_size) => first_array_size.cmp(second_array_size),
                UniformType::F32 => Ordering::Less,
                UniformType::Vec4 => Ordering::Greater,
            },
            UniformType::F32 => match other {
                UniformType::F32 => Ordering::Equal,
                UniformType::Vec4 | UniformType::Array(_) => Ordering::Greater,
            },
            UniformType::Vec4 => match other {
                UniformType::Array(_) | UniformType::F32 => Ordering::Less,
                UniformType::Vec4 => Ordering::Equal,
            },
        }
    }
}

impl From<&UniformValue> for UniformValue {
    fn from(value: &UniformValue) -> Self {
        value.clone()
    }
}

impl From<f32> for UniformValue {
    fn from(value: f32) -> Self {
        UniformValue::F32(value.into())
    }
}

impl From<Vec4> for UniformValue {
    fn from(value: Vec4) -> Self {
        UniformValue::Vec4(value.into())
    }
}

impl From<FixedSizeVec<Vec4>> for UniformValue {
    fn from(value: FixedSizeVec<Vec4>) -> Self {
        UniformValue::Array(value.into())
    }
}

impl UniformValue {
    pub fn matches_uniform_type(&self, uniform_type: &UniformType) -> bool {
        match &self {
            UniformValue::Array(real_time_array) => match uniform_type {
                UniformType::Array(size) => real_time_array.current_value().len() == *size,
                UniformType::F32 | UniformType::Vec4 => false,
            },
            UniformValue::F32(_) => matches!(uniform_type, UniformType::F32),
            UniformValue::Vec4(_) => matches!(uniform_type, UniformType::Vec4),
        }
    }

    pub fn uniform_type(&self) -> UniformType {
        match &self {
            UniformValue::Array(real_time_array) => {
                UniformType::Array(real_time_array.current_value().len())
            }
            UniformValue::F32(_) => UniformType::F32,
            UniformValue::Vec4(_) => UniformType::Vec4,
        }
    }

    pub fn generate_offset(&self) -> u32 {
        match &self {
            UniformValue::Array(uniform_var) => uniform_var.current_value().generate_offset(),
            UniformValue::F32(uniform_var) => uniform_var.current_value().generate_offset(),
            UniformValue::Vec4(uniform_var) => uniform_var.current_value().generate_offset(),
        }
    }

    pub fn size_of(&self) -> u32 {
        (match &self {
            UniformValue::Array(uniform) => size_of::<Vec4>() * uniform.current_value().len(),
            UniformValue::F32(_) => size_of::<f32>(),
            UniformValue::Vec4(_) => size_of::<Vec4>(),
        }) as u32
    }

    pub fn as_f32_buffer(&self) -> Vec<f32> {
        match &self {
            UniformValue::Array(uniform_var) => uniform_var.current_value().as_f32_buffer(),
            UniformValue::F32(uniform_var) => uniform_var.current_value().as_f32_buffer(),
            UniformValue::Vec4(uniform_var) => uniform_var.current_value().as_f32_buffer(),
        }
    }
}

impl UniformType {
    pub fn shader_string(&self) -> String {
        match &self {
            UniformType::Array(size) => FixedSizeVec::new_empty(*size).shader_string(),
            UniformType::F32 => f32::default().shader_string(),
            UniformType::Vec4 => Vec4::default().shader_string(),
        }
    }

    pub fn default_value(&self) -> UniformValue {
        match &self {
            UniformType::Array(size) => UniformValue::Array(UniformVar::<FixedSizeVec<Vec4>>::new(
                None,
                FixedSizeVec::<Vec4>::new_empty(*size),
            )),
            UniformType::F32 => UniformValue::F32(UniformVar::<f32>::default()),
            UniformType::Vec4 => UniformValue::Vec4(UniformVar::<Vec4>::default()),
        }
    }

    pub fn doubleword_size<UT, U>(uniform_types: U) -> usize
    where
        UT: Into<UniformType>,
        U: IntoIterator<Item = UT>,
    {
        uniform_types
            .into_iter()
            .map(|uniform_type| match uniform_type.into() {
                UniformType::Array(size) => size * 4,
                UniformType::F32 => 1,
                UniformType::Vec4 => 4,
            })
            .sum()
    }
}

/// A trait to be implemented on any [`Uniform`] type to ensure it has certain API contracts
pub trait Uniform: std::fmt::Debug + Clone + PartialEq {
    fn default_value() -> Self;
    fn shader_string(&self) -> String;
    fn generate_offset(&self) -> u32;
    fn as_f32_iter(&self) -> impl Iterator<Item = f32>;
    fn as_f32_buffer(&self) -> Vec<f32>;
}

impl Uniform for FixedSizeVec<Vec4> {
    fn default_value() -> Self {
        FixedSizeVec::<Vec4>::default()
    }

    fn shader_string(&self) -> String {
        format!("array<vec4f, {}>", &self.len())
    }

    fn generate_offset(&self) -> u32 {
        (self.len() * 4) as u32
    }

    fn as_f32_iter(&self) -> impl Iterator<Item = f32> {
        self.iter().flat_map(|value| value.to_array())
    }

    fn as_f32_buffer(&self) -> Vec<f32> {
        self.as_f32_iter().collect()
    }
}

impl Uniform for f32 {
    fn default_value() -> Self {
        0.
    }

    fn shader_string(&self) -> String {
        "f32".to_string()
    }

    fn generate_offset(&self) -> u32 {
        1
    }

    fn as_f32_iter(&self) -> impl Iterator<Item = f32> {
        // To stay consistent with most of the other types, we need to be able to expose
        // this single value as an iterator, `once` allows us to do that
        once(*self)
    }

    fn as_f32_buffer(&self) -> Vec<f32> {
        vec![*self]
    }
}

impl Uniform for Vec4 {
    fn default_value() -> Self {
        Vec4::default()
    }

    fn shader_string(&self) -> String {
        "vec4f".to_string()
    }

    fn generate_offset(&self) -> u32 {
        4
    }

    fn as_f32_iter(&self) -> impl Iterator<Item = f32> {
        self.to_array().into_iter()
    }

    fn as_f32_buffer(&self) -> Vec<f32> {
        self.to_array().into()
    }
}

/// An actual value of a [`Uniform`] trait applied value. Useful because it can hold the default value
/// and return it to the user if necessary
#[derive(Debug, Clone)]
pub struct UniformVar<U: Uniform> {
    value: Option<U>,
    default: U,
    offset: u32,
}

impl<U: Uniform> UniformVar<U> {
    pub fn new(value: Option<U>, default: U) -> Self {
        let mut new_uniform_var = Self {
            value,
            default,
            ..Default::default()
        };
        new_uniform_var.offset = new_uniform_var.current_value().generate_offset();
        new_uniform_var
    }
    pub fn current_value(&self) -> &U {
        if let Some(value) = &self.value {
            value
        } else {
            &self.default
        }
    }

    pub fn update_value(&mut self, new_value: &U) {
        self.value = Some(new_value.clone());
        self.offset = new_value.generate_offset();
    }

    pub fn is_default(&self) -> bool {
        self.value.is_none() || *self.value.as_ref().unwrap() == self.default
    }

    pub fn shader_string(&self) -> String {
        self.current_value().shader_string()
    }
}

impl<U: Uniform> PartialEq for UniformVar<U> {
    fn eq(&self, other: &Self) -> bool {
        self.current_value() == other.current_value()
    }
}

impl<U: Uniform> Default for UniformVar<U> {
    fn default() -> Self {
        let mut new_uniform_var = Self {
            value: None,
            default: U::default_value(),
            offset: 0,
        };
        new_uniform_var.offset = new_uniform_var.default.generate_offset();
        new_uniform_var
    }
}

impl<U: Uniform> From<U> for UniformVar<U> {
    fn from(value: U) -> Self {
        Self {
            value: Some(value),
            ..Default::default()
        }
    }
}

/// These represents one specific mapping of [`UniformValue`] with specified values to a [`Material`]'s names.
/// For example, if we have a `wiggle` [`Material`], we could have a "fast wiggle" with a low value specified
/// with a name of `wiggle` and a [`UniformValue::F32`], and a "slow wiggle" with a high value. These would be two
/// seperate [`MaterialUniforms`]
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialUniforms {
    material_id: MaterialId,
    pub(crate) uniforms_map: IndexMap<String, UniformValue>,
}

impl MaterialUniforms {
    pub fn new(material_id: MaterialId, uniforms_map: IndexMap<String, UniformValue>) -> Self {
        Self {
            material_id,
            uniforms_map,
        }
    }

    pub fn new_from_iter<S, U, I>(material_id: MaterialId, iter: I) -> Self
    where
        S: ToString,
        U: Into<UniformValue>,
        I: IntoIterator<Item = (S, U)>,
    {
        let uniforms_map = iter
            .into_iter()
            .map(|(name, uniform)| (name.to_string(), uniform.into()))
            .collect::<IndexMap<_, _>>();

        Self::new(material_id, uniforms_map)
    }

    pub fn new_from_failable_iter<S, I>(material_id: MaterialId, iter: I) -> Result<Self>
    where
        S: ToString,
        I: IntoIterator<Item = (S, Result<UniformValue>)>,
    {
        let uniforms_map =
            iter.into_iter()
                .try_fold(IndexMap::new(), |mut accumulator, (name, uniform)| {
                    let uniform = match uniform {
                        Ok(uniform) => uniform,
                        Err(err) => bail! {"{err}"},
                    };
                    let name = name.to_string();
                    accumulator.insert(name, uniform);
                    Ok(accumulator)
                });

        match uniforms_map {
            Ok(uniforms_map) => Ok(Self::new(material_id, uniforms_map)),
            Err(err) => bail!("{err}"),
        }
    }

    pub fn empty(material_id: MaterialId) -> Self {
        Self {
            material_id,
            uniforms_map: IndexMap::new(),
        }
    }

    pub fn material_id(&self) -> MaterialId {
        self.material_id
    }

    pub fn update<S: AsRef<str>>(&mut self, name: S, value: UniformValue) -> Result<&mut Self> {
        let name = name.as_ref();

        if !self.uniforms_map.contains_key(name) {
            bail!("Name {name} does not exist in MaterialUniforms, can update value to {value:?}");
        }

        if self.uniforms_map.get(name).unwrap().uniform_type() != value.uniform_type() {
            bail!(
                "Attempted to update MaterialUniforms at key {name} to type {} when type is actually {}",
                value.uniform_type(),
                self.uniforms_map.get(name).unwrap().uniform_type()
            )
        }

        self.uniforms_map.insert(name.to_string(), value);

        Ok(self)
    }

    const fn sprite_material_reserved_uniforms() -> &'static str {
        "  local_to_world: mat4x4f,
  color: vec4f,
  uv_scale_offset: vec4f,\n"
    }

    pub fn calculate_reserved_uniform_buffer_size() -> usize {
        Self::sprite_material_reserved_uniforms().trim().split(',').filter_map(|segment| {
            if segment.is_empty() {
                return None;
            }
            Some(segment.split(":").nth(1).expect("There should always be a : in the sprite_material_reserved uniforms"))
        }).fold(0, |accumulator, type_as_string| {
            accumulator + match type_as_string.trim() {
                "mat4x4f" => 16,
                "vec4f" => 4,
                unexpected_uniform_type => panic!("Encountered {unexpected_uniform_type} when parsing reserved uniforms, which is currently not accounted for"),
            }
        })
    }

    pub fn generate_dynamic_uniforms_shader_string(
        &self,
        material_type: &MaterialType,
        padding_length: usize,
    ) -> String {
        let mut uniforms_string = "struct SceneInstance {\n".to_string();
        let uniform_types_with_defaults = &self.uniforms_map;
        if material_type == &MaterialType::Sprite {
            uniforms_string.push_str(Self::sprite_material_reserved_uniforms());
        }
        if !uniform_types_with_defaults.is_empty() {
            for (variable_name, uniform) in
                sort_uniforms_by_name_and_type(uniform_types_with_defaults)
            {
                uniforms_string.push_str(
                    format!(
                        "  {variable_name} : {},\n",
                        uniform.uniform_type().shader_string()
                    )
                    .as_str(),
                );
            }
        }
        let vec4s = padding_length / 4;
        let remaining_f32s = padding_length % 4;

        for index in 0..remaining_f32s {
            uniforms_string.push_str(format!("  f32_{index}_padding: f32,\n").as_str());
        }
        uniforms_string.push_str(format!("  padding: array<vec4f, {vec4s}>\n").as_str());
        uniforms_string.push_str("};");
        uniforms_string
    }

    pub fn params_size(&self) -> u32 {
        self.uniforms_map
            .iter()
            .fold(0, |accumulator, (_, uniform)| {
                accumulator + uniform.size_of()
            })
    }

    /// The sort order matters, and details can be found in the [`Ord`] implementation of [`UniformType`]
    pub fn sort_into_vec(&self) -> Vec<(&str, &UniformValue)> {
        sort_uniforms_by_name_and_type(&self.uniforms_map)
    }

    pub fn sort(&mut self) {
        let sorted_uniforms = self.sort_into_vec();
        *self = Self::new_from_iter(self.material_id, sorted_uniforms);
    }
}

impl Deref for MaterialUniforms {
    type Target = IndexMap<String, UniformValue>;

    fn deref(&self) -> &Self::Target {
        &self.uniforms_map
    }
}
