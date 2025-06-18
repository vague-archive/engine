use std::{collections::HashMap, error::Error, num::NonZero};

use void_public::{
    EventWriter,
    event::graphics::NewPipeline,
    material::MaterialId,
    pipeline::{PipelineId, PipelineType},
};

type FfiPendingPipeline = void_public::pipeline::PendingPipeline;

pub type PipelineFailure = Box<dyn Error + Send + Sync>;

#[derive(Debug, Default)]
#[cfg_attr(not(feature = "internal_features"), allow(dead_code))]
pub struct PipelineAssetManager {
    /// This is used in situation where potentially large numbers of pipelines
    /// can be queued up, for example when loading a game.
    pub(crate) batched_pipelines: HashMap<PipelineId, Pipeline>,
    pub(crate) batched_material_id_to_id: HashMap<MaterialId, PipelineId>,
    pub(crate) next_pipeline_id: PipelineId,
    pub(crate) pipelines: HashMap<PipelineId, Pipeline>,
    pub(crate) material_id_to_id: HashMap<MaterialId, PipelineId>,
}

impl PipelineAssetManager {
    /// Gives the caller the next available [`PipelineId`]. Currently
    /// [`PipelineId`]'s should not be relied upon as consistently applied to
    /// the same pipeline from run to run. The associated [`MaterialId`] should
    /// be consistent though.
    pub fn register_next_pipeline_id(&mut self) -> PipelineId {
        let next_pipeline_id = self.next_pipeline_id;
        self.next_pipeline_id =
            PipelineId(unsafe { NonZero::new_unchecked((*next_pipeline_id).get() + 1) });
        next_pipeline_id
    }

    pub fn get_pipeline_by_id(&self, pipeline_id: PipelineId) -> Option<&Pipeline> {
        self.pipelines
            .get(&pipeline_id)
            .or_else(|| self.batched_pipelines.get(&pipeline_id))
    }

    pub fn get_pipeline_id_from_material_id(&self, material_id: MaterialId) -> Option<PipelineId> {
        self.material_id_to_id
            .get(&material_id)
            .or_else(|| self.batched_material_id_to_id.get(&material_id))
            .copied()
    }

    pub fn are_all_ids_loaded<'a, I>(&self, ids: I) -> bool
    where
        I: IntoIterator<Item = &'a PipelineId>,
    {
        ids.into_iter().all(|id| {
            let Some(pipeline) = self.pipelines.get(id) else {
                return false;
            };

            matches!(pipeline.pipeline_type(), PipelineType::Loaded)
        })
    }

    /// Sends the message to the platform side to construct a render pipeline
    /// for a given material. Users likely won't manually call this, as we
    /// automatically handle this with material creation, however it is
    /// available for a potential power user.
    pub fn load_pipeline<'a>(
        &'a mut self,
        material_id: MaterialId,
        new_pipeline_event_writer: &EventWriter<NewPipeline>,
    ) -> &'a PendingPipeline {
        let pending_pipeline_id = self.register_next_pipeline_id();
        let pending_pipeline = PendingPipeline::new(pending_pipeline_id, material_id);

        self.load_pipeline_by_pending_pipeline(&pending_pipeline, new_pipeline_event_writer);

        self.get_pipeline_by_id(pending_pipeline_id)
            .unwrap()
            .as_pending_pipeline()
            .unwrap()
    }

    pub fn load_pipeline_by_pending_pipeline(
        &mut self,
        pending_pipeline: &PendingPipeline,
        new_pipeline_event_writer: &EventWriter<NewPipeline>,
    ) {
        self.pipelines.remove(&pending_pipeline.id());
        new_pipeline_event_writer.write(NewPipeline::new(
            pending_pipeline.id().get(),
            *pending_pipeline.material_id(),
        ));
        self.pipelines
            .insert(pending_pipeline.id(), (*pending_pipeline).into());
        self.material_id_to_id
            .insert(pending_pipeline.material_id(), pending_pipeline.id());
    }
}

#[cfg(feature = "internal_features")]
impl PipelineAssetManager {
    pub fn add_to_batched_pipelines(&mut self, pending_pipeline: PendingPipeline) {
        self.batched_material_id_to_id
            .insert(pending_pipeline.material_id(), pending_pipeline.id());
        self.batched_pipelines
            .insert(pending_pipeline.id(), pending_pipeline.into());
    }

    pub fn drain_batched_pipelines(
        &mut self,
    ) -> std::collections::hash_map::Drain<'_, PipelineId, Pipeline> {
        self.batched_material_id_to_id.clear();
        self.batched_pipelines.drain()
    }

    pub fn insert_engine_pipeline(
        &mut self,
        engine_pipeline: &EnginePipeline,
    ) -> Result<(), PipelineFailure> {
        if self.pipelines.contains_key(&engine_pipeline.id) {
            return Err(format!(
                "Pipeline id {} already exists, cannot insert internal pipeline",
                engine_pipeline.id()
            )
            .into());
        }

        if let Some(existent_pipeline_id) = self.material_id_to_id.get(&engine_pipeline.material_id)
        {
            return Err(format!("Material {} already already has a pipeline {existent_pipeline_id}, cannot insert internal pipeline", engine_pipeline.material_id()).into());
        }

        self.pipelines
            .insert(engine_pipeline.id(), (*engine_pipeline).into());
        self.material_id_to_id
            .insert(engine_pipeline.material_id(), engine_pipeline.id());

        Ok(())
    }

    /// This should only be used on events from the platform side to indicate a
    /// given [`Pipeline`] failed to load.
    pub fn replace_failed_pipeline(&mut self, failed_pipeline: &FailedPipeline) {
        self.pipelines.remove(&failed_pipeline.id());

        self.pipelines
            .insert(failed_pipeline.id(), failed_pipeline.clone().into());
        self.material_id_to_id
            .insert(failed_pipeline.material_id(), failed_pipeline.id());
    }

    /// This should only be used on events from the platform side to indicate a
    /// given [`Pipeline`] successfully loaded.
    pub fn replace_loaded_pipeline(&mut self, loaded_pipeline: &LoadedPipeline) {
        self.pipelines.remove(&loaded_pipeline.id());

        self.pipelines
            .insert(loaded_pipeline.id(), (*loaded_pipeline).into());
        self.material_id_to_id
            .insert(loaded_pipeline.material_id(), loaded_pipeline.id());
    }

    pub fn insert_loaded_pipeline(
        &mut self,
        loaded_pipeline: &LoadedPipeline,
    ) -> Result<(), PipelineFailure> {
        if self.pipelines.contains_key(&loaded_pipeline.id()) {
            return Err(format!(
                "Id {} already exists, cannot insert loaded pipeline",
                loaded_pipeline.id()
            )
            .into());
        }

        if let Some(existent_pipeline_id) = self.material_id_to_id.get(&loaded_pipeline.material_id)
        {
            return Err(format!("Material {} already already has a pipeline {existent_pipeline_id}, cannot insert pipeline", loaded_pipeline.material_id()).into());
        }

        self.pipelines
            .insert(loaded_pipeline.id(), (*loaded_pipeline).into());
        self.material_id_to_id
            .insert(loaded_pipeline.material_id(), loaded_pipeline.id());

        Ok(())
    }
}

/// Enum representing all possible states of a pipeline creation request.
#[derive(Clone, Debug)]
pub enum Pipeline {
    Pending(PendingPipeline),
    Loaded(LoadedPipeline),
    Engine(EnginePipeline),
    Failed(FailedPipeline),
}

impl Pipeline {
    pub fn id(&self) -> PipelineId {
        match self {
            Self::Pending(pending_pipeline) => pending_pipeline.id(),
            Self::Loaded(loaded_pipeline) => loaded_pipeline.id(),
            Self::Engine(engine_pipeline) => engine_pipeline.id(),
            Self::Failed(failed_pipeline) => failed_pipeline.id(),
        }
    }

    pub fn material_id(&self) -> MaterialId {
        match self {
            Self::Pending(pending_pipeline) => pending_pipeline.material_id(),
            Self::Loaded(loaded_pipeline) => loaded_pipeline.material_id(),
            Self::Engine(engine_pipeline) => engine_pipeline.material_id(),
            Self::Failed(failed_pipeline) => failed_pipeline.material_id(),
        }
    }

    pub const fn pipeline_type(&self) -> PipelineType {
        match self {
            Self::Pending(_) => PendingPipeline::pipeline_type(),
            Self::Loaded(_) => LoadedPipeline::pipeline_type(),
            Self::Engine(_) => EnginePipeline::pipeline_type(),
            Self::Failed(_) => FailedPipeline::pipeline_type(),
        }
    }

    pub fn as_pending_pipeline(&self) -> Option<&PendingPipeline> {
        if let Self::Pending(pending_pipeline) = self {
            Some(pending_pipeline)
        } else {
            None
        }
    }

    pub fn as_engine_pipeline(&self) -> Option<&EnginePipeline> {
        if let Self::Engine(engine_pipeline) = self {
            Some(engine_pipeline)
        } else {
            None
        }
    }

    pub fn as_loaded_pipeline(&self) -> Option<&LoadedPipeline> {
        if let Self::Loaded(laoded_pipeline) = self {
            Some(laoded_pipeline)
        } else {
            None
        }
    }

    pub fn as_failed_pipeline(&self) -> Option<&FailedPipeline> {
        if let Self::Failed(failed_pipeline) = self {
            Some(failed_pipeline)
        } else {
            None
        }
    }
}

/// This is a [`Pipeline`] that is being loaded on the platform.
#[derive(Clone, Copy, Debug)]
pub struct PendingPipeline {
    id: PipelineId,
    material_id: MaterialId,
}

impl PendingPipeline {
    pub fn new(id: PipelineId, material_id: MaterialId) -> Self {
        Self { id, material_id }
    }

    pub fn id(&self) -> PipelineId {
        self.id
    }

    pub fn material_id(&self) -> MaterialId {
        self.material_id
    }

    pub const fn pipeline_type() -> PipelineType {
        PipelineType::Pending
    }
}

impl From<PendingPipeline> for Pipeline {
    fn from(value: PendingPipeline) -> Self {
        Self::Pending(value)
    }
}

impl From<&FfiPendingPipeline> for PendingPipeline {
    fn from(value: &FfiPendingPipeline) -> Self {
        Self::new(value.id, value.material_id)
    }
}

impl From<FfiPendingPipeline> for PendingPipeline {
    fn from(value: FfiPendingPipeline) -> Self {
        (&value).into()
    }
}

/// An internal, engine only pipeline.
#[derive(Clone, Copy, Debug)]
pub struct EnginePipeline {
    id: PipelineId,
    material_id: MaterialId,
}

impl EnginePipeline {
    pub fn new(id: PipelineId, material_id: MaterialId) -> Self {
        Self { id, material_id }
    }

    pub fn id(&self) -> PipelineId {
        self.id
    }

    pub fn material_id(&self) -> MaterialId {
        self.material_id
    }

    pub const fn pipeline_type() -> PipelineType {
        PipelineType::Engine
    }
}

impl From<EnginePipeline> for Pipeline {
    fn from(value: EnginePipeline) -> Self {
        Self::Engine(value)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LoadedPipeline {
    id: PipelineId,
    material_id: MaterialId,
}

impl LoadedPipeline {
    pub fn new(id: PipelineId, material_id: MaterialId) -> Self {
        Self { id, material_id }
    }

    pub fn id(&self) -> PipelineId {
        self.id
    }

    pub fn material_id(&self) -> MaterialId {
        self.material_id
    }

    pub const fn pipeline_type() -> PipelineType {
        PipelineType::Loaded
    }
}

impl From<LoadedPipeline> for Pipeline {
    fn from(value: LoadedPipeline) -> Self {
        Self::Loaded(value)
    }
}

/// This represents a [`Pipeline`] that has failed to load from the platform.
#[derive(Clone, Debug)]
pub struct FailedPipeline {
    id: PipelineId,
    material_id: MaterialId,
    failure_reason: String,
}

impl FailedPipeline {
    pub fn new(id: PipelineId, material_id: MaterialId, failure_reason: &str) -> Self {
        Self {
            id,
            material_id,
            failure_reason: failure_reason.to_string(),
        }
    }

    pub fn id(&self) -> PipelineId {
        self.id
    }

    pub fn material_id(&self) -> MaterialId {
        self.material_id
    }

    pub fn failure_reason(&self) -> &str {
        &self.failure_reason
    }

    pub const fn pipeline_type() -> PipelineType {
        PipelineType::Failed
    }
}

impl From<FailedPipeline> for Pipeline {
    fn from(value: FailedPipeline) -> Self {
        Self::Failed(value)
    }
}
