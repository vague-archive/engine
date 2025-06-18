use game_module_macro::ResourceWithoutSerialize;
use void_public::{ComponentId, EcsType, Resource, material::MaterialId};

use crate::resource_managers::material_manager::{materials::Material, uniforms::MaterialUniforms};

#[derive(Debug, Default, ResourceWithoutSerialize)]
pub struct WorldRenderManager {
    pub should_generate_down_samples: bool,
    postprocesses: Vec<PostProcess>,
}

impl WorldRenderManager {
    pub fn postprocesses(&self) -> &[PostProcess] {
        &self.postprocesses
    }

    pub fn get_postprocess_by_material_id(&self, material_id: MaterialId) -> Option<&PostProcess> {
        self.postprocesses
            .iter()
            .find(|post_process| post_process.material_id == material_id)
    }

    pub fn get_postprocess_by_material_id_mut(
        &mut self,
        material_id: MaterialId,
    ) -> Option<&mut PostProcess> {
        self.postprocesses
            .iter_mut()
            .find(|post_process| post_process.material_id == material_id)
    }

    pub fn add_or_update_postprocess(
        &mut self,
        material: &Material,
        material_uniforms: &MaterialUniforms,
    ) {
        let validated_material_uniforms = material.validate_material_uniforms(material_uniforms);
        let material_uniforms = match validated_material_uniforms {
            Ok(_) => material_uniforms.clone(),
            Err(validated_material_uniforms) => validated_material_uniforms,
        };

        self.postprocesses
            .push(PostProcess::new(material.material_id(), material_uniforms));
    }

    pub fn remove_postprocess(&mut self, material_id: MaterialId) {
        self.postprocesses
            .retain(|post_process| post_process.material_id() != &material_id);
    }

    pub fn remove_postprocesses(&mut self, material_ids: &[MaterialId]) {
        self.postprocesses
            .retain(|post_process| !material_ids.contains(&post_process.material_id));
    }
}

#[derive(Debug)]
pub struct PostProcess {
    material_id: MaterialId,
    pub material_uniforms: MaterialUniforms,
}

impl PostProcess {
    pub fn new(material_id: MaterialId, material_uniforms: MaterialUniforms) -> Self {
        Self {
            material_id,
            material_uniforms,
        }
    }

    pub fn material_id(&self) -> &MaterialId {
        &self.material_id
    }
}
