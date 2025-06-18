use std::{cmp::Ordering, collections::HashMap};

use anyhow::{Result, anyhow, bail};
use glam::Vec4;
use indexmap::IndexMap;
use lazy_regex::{Lazy, Regex, lazy_regex};
use serde::{
    Deserialize,
    de::{Error, Visitor},
};
use toml::Value;
use void_public::material::FilterMode;

use super::{
    ShaderInsertionPoint, ShaderSnippet,
    fixed_size_vec::FixedSizeVec,
    uniforms::{UniformType, UniformValue, UniformVar, sort_uniforms_by_name_and_type},
};

// see regex101.com for a better visualization, but this matches array<vec4f, X> where X is a number, and also a regex capture group
static ARRAY_OF_VEC4_IN_WGSL_REGEX: Lazy<Regex> = lazy_regex!(r"array<vec4f,[ ]*(\d*)>");

/// This is just our `serde` struct for reading materials from .toml files
/// * [`Self::uniform_types`] : map of uniform names to their uniform types or a set of [`UniformOptions`]
/// * [`Self::texture_descs`] : map of textures to descriptions about the textures
/// * [`Self::get_world_offset`] : body of vertex shader function
/// * [`Self::get_fragment_color`] : body of fragment color shader function
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TomlMaterial {
    uniform_types: Option<HashMap<String, UniformTypeOrUniformOptions>>,
    texture_descs: Option<HashMap<String, String>>,
    get_world_offset: String,
    get_fragment_color: String,
}

#[derive(Debug)]
struct UniformOptions {
    uniform_type: UniformType,
    default: Option<UniformValue>,
}

/// This is a helper for parsing the uniforms from serde. Currently a user can pass in either a [`String`],
/// which gets converted to a [`UniformType`], or a map that aligns with [`UniformOptions`]. It is worth noting
/// we don't use `type` on [`UniformOptions`] because it's a reserved word in Rust and it's a bit ugly to use r#type
#[derive(Debug)]
enum UniformTypeOrUniformOptions {
    UniformType(UniformType),
    UniformOptions(UniformOptions),
}

/// [`Visitor`] is a [`serde`] pattern for describing a relationship between various types and how they are deserialized.
/// So in this case, this type relates to how [`UniformTypeOrUniformOptions`] is deserialized
struct UniformTypeOrUniformOptionsVisitor;

impl<'de> Deserialize<'de> for UniformTypeOrUniformOptions {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(UniformTypeOrUniformOptionsVisitor)
    }
}

const VALID_UNIFORM_OPTIONS_KEYS: [&str; 2] = ["type", "default"];

impl<'de> Visitor<'de> for UniformTypeOrUniformOptionsVisitor {
    type Value = UniformTypeOrUniformOptions;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a string or an array with a string matched with either a f32, array of 4 f32s, or an array of arrays of 4 f32s")
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let uniform_type = parse_uniform_type_strings(value).map_err(|err| {
            Error::custom(format!(
                "Could not parse type into a known uniform type: {err}"
            ))
        })?;
        Ok(UniformTypeOrUniformOptions::UniformType(uniform_type))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(value.as_str())
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut map_entries = vec![];
        while let Some((key, value)) = map.next_entry::<Value, Value>()? {
            let Value::String(key_as_string) = key else {
                return Err(Error::custom(format!(
                    "Key must be a string, instead found {:?}",
                    key
                )));
            };
            if !VALID_UNIFORM_OPTIONS_KEYS.contains(&key_as_string.as_str()) {
                return Err(Error::custom(format!(
                    "key {} found, only {} keys allowed",
                    key_as_string,
                    VALID_UNIFORM_OPTIONS_KEYS.join(",")
                )));
            }
            map_entries.push((key_as_string, value));
        }

        if map_entries.is_empty() {
            return Err(Error::custom(
                "map is empty, it must contain at least type with a valid uniform_type",
            ));
        }
        // Sort so type is first, this makes processing easier
        map_entries.sort_by(|(a_name, _), (b_name, _)| {
            if a_name == "type" {
                Ordering::Less
            } else if b_name == "type" {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        });

        let (_, type_value) = map_entries.remove(0);

        let uniform_type = if let Value::String(uniform_type_as_string) = type_value {
            parse_uniform_type_strings(uniform_type_as_string.as_str())
                .map_err(|err| Error::custom(format!("{err}")))?
        } else {
            return Err(Error::custom(
                "Value for key type is not a string, expected a string".to_string(),
            ));
        };

        // Right now there can only be one more element. If we add more options for uniform_types, it will probably make sense to
        // iterate over the map_entries but right now I'm just going to attempt to pull the only remaining member

        let default = if let Some((_, default_value)) = map_entries.first() {
            Some(
                parse_uniform_default_from_serde_value(&uniform_type, default_value)
                    .map_err(|err| Error::custom(format!("{err}")))?,
            )
        } else {
            None
        };

        Ok(Self::Value::UniformOptions(UniformOptions {
            uniform_type,
            default,
        }))
    }
}

fn parse_uniform_default_from_serde_value(
    uniform_type: &UniformType,
    value: &Value,
) -> Result<UniformValue> {
    Ok(match uniform_type {
        // There is an additional constraint on arrays that the default matches the expected length from the array definition
        // ie, if a user says array<vec4f, 3>, then the default array must have 3 members
        UniformType::Array(size) => {
            let array_of_vec4s = match value {
                Value::Array(inner_vec4s) => {
                    if inner_vec4s.len() != *size {
                        bail!(
                            "The size designated from the array of vec4 name does not match the supplied array of vec4s"
                        );
                    }
                    inner_vec4s.iter().try_fold(
                        vec![],
                        |mut accumulator, possible_vec4_array| {
                            let vec4 = match parse_vec4_from_serde_value(possible_vec4_array) {
                                Ok(vec4) => vec4,
                                Err(err) => bail!("{err}"),
                            };
                            accumulator.push(vec4);
                            Ok(accumulator)
                        },
                    )?
                }
                _ => {
                    bail!("array of vec4 types must have a single outer array")
                }
            };
            UniformValue::Array(UniformVar::new(None, FixedSizeVec::new(&array_of_vec4s)))
        }
        UniformType::F32 => {
            let float_value = match value {
                Value::Float(float_value) => *float_value as f32,
                _ => {
                    bail!("f32 types must have an f32 default value".to_string(),)
                }
            };
            UniformValue::F32(UniformVar::new(None, float_value))
        }
        UniformType::Vec4 => {
            let vec4 = parse_vec4_from_serde_value(value).map_err(|err| anyhow!("{err}"))?;
            UniformValue::Vec4(UniformVar::new(None, vec4))
        }
    })
}

fn parse_vec4_from_serde_value(vec4_value: &Value) -> Result<Vec4> {
    Ok(match vec4_value {
        Value::Array(array) => {
            if array.len() != 4 {
                bail!("the vec of floats must have 4 f32s");
            }
            array.iter().enumerate().try_fold(
                Vec4::default(),
                |mut accumulator, (index, value)| {
                    let float_value = match value {
                        Value::Float(float_value) => (*float_value) as f32,
                        _ => return Err(anyhow!("f32 types must have an f32 default value")),
                    };
                    accumulator[index] = float_value;
                    Ok(accumulator)
                },
            )?
        }
        _ => bail!(format!(
            "vec4 types must have an array of 4 f32 for the default value"
        )),
    })
}

fn parse_uniform_type_strings(possible_uniform_type_as_str: &str) -> Result<UniformType> {
    Ok(match possible_uniform_type_as_str {
        "vec4f" => UniformType::Vec4,
        "f32" => UniformType::F32,
        array_type if ARRAY_OF_VEC4_IN_WGSL_REGEX.is_match(possible_uniform_type_as_str) => {
            let array_size_as_str = ARRAY_OF_VEC4_IN_WGSL_REGEX
                .captures(array_type)
                .unwrap()
                .get(1)
                .unwrap()
                .as_str();
            let array_size = array_size_as_str.parse()?;
            UniformType::Array(array_size)
        }
        unknown_uniform_type => bail!(
            "TomlMaterial::generate_shader_snippets() Uniform type {unknown_uniform_type} supplied from TomlMaterial. Should be vec4f , f32 , or array<vec4f, 8>"
        ),
    })
}

impl TomlMaterial {
    /// Converts the TOML file data to a [`Vec`] of ([`ShaderInsertionPoint`], [`ShaderSnippet`])
    ///
    /// # Errors
    ///
    /// - Creates an error if an unknown uniform type is found (currently vec4f, f32, array<vec4f, 8>)
    /// - Creates an error if the uniform cannot be converted into [`Uniform`]
    /// - Creates an error if a texture filter mode cannot be converted to an expected [`FilterMode`]
    /// - Creates an error if a texture filter cannot be converted to [`ShaderSnippet::Textures`]
    pub fn generate_shader_snippets(&self) -> Result<Vec<(ShaderInsertionPoint, ShaderSnippet)>> {
        let mut output = vec![];
        output.push((
            ShaderInsertionPoint::world_offset(),
            ShaderSnippet::FunctionBody(self.get_world_offset.clone()),
        ));
        output.push((
            ShaderInsertionPoint::fragment_color(),
            ShaderSnippet::FunctionBody(self.get_fragment_color.clone()),
        ));
        if let Some(uniform_types_or_defaults) = &self.uniform_types {
            let transformed_uniforms: Result<IndexMap<String, UniformValue>> = uniform_types_or_defaults.iter().try_fold(
                IndexMap::new(),
                |mut accumulator, (uniform_name_string, uniform_type_string_or_string_array)| {
                    let uniform = match uniform_type_string_or_string_array {
                        UniformTypeOrUniformOptions::UniformType(uniform_type) => {
                            uniform_type.default_value()
                        },
                        UniformTypeOrUniformOptions::UniformOptions(uniform_options) => {
                            if let Some(default) = &uniform_options.default {
                                default.clone()
                            } else {
                                uniform_options.uniform_type.default_value()
                            }
                        },
                    };
                    accumulator.insert(uniform_name_string.clone(), uniform);
                    Ok(accumulator)
                },
            );
            let transformed_uniforms = match transformed_uniforms {
                Ok(transformed_uniforms) => transformed_uniforms,
                Err(err) => bail!(
                    "TomlMaterial::generate_shader_snippets() Error converting uniforms: {err}"
                ),
            };
            let transformed_uniforms = sort_uniforms_by_name_and_type(&transformed_uniforms)
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.clone()))
                .collect::<IndexMap<_, _>>();
            output.push((
                ShaderInsertionPoint::uniform(),
                ShaderSnippet::Uniforms(transformed_uniforms),
            ));
        }
        if let Some(textures) = &self.texture_descs {
            let transformed_textures = textures.iter().try_fold(IndexMap::new(), |mut accumulator, (texture_name, sampler_filter_mode)| {
                let filter_mode = match sampler_filter_mode.as_str() {
                    "nearest" => FilterMode::Nearest,
                    "linear" => FilterMode::Linear,
                    unknown_filter_mode => bail!("TomlMaterial::generate_shader_snippets() Filter mode type {unknown_filter_mode} supplied from TOMLMaterial. Should be nearest or linear"),
                };
                accumulator.insert(texture_name.clone(), filter_mode);
                Ok(accumulator)
            });
            let transformed_textures = match transformed_textures {
                Ok(transformed_textures) => transformed_textures,
                Err(err) => bail!(
                    "TomlMaterial::generate_shader_snippets() Error converting textures: {err}"
                ),
            };
            output.push((
                ShaderInsertionPoint::texture(),
                ShaderSnippet::Textures(transformed_textures),
            ));
        }
        Ok(output)
    }
}
