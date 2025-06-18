use game_ecs::{
    ArchetypeStorageMap, CpuFrameData, FrameDataBufferBorrowRef, FrameDataBufferBorrowRefMut,
};
use game_entity::EntityId;
use game_world::World;
use void_public::{EcsType, LocalToWorld, Mat4, Quat, Transform};

pub fn update_world_transforms(
    world: &World,
    archetype_storage_map: &ArchetypeStorageMap,
    cpu_data: &mut CpuFrameData,
) {
    for entity_id in world.entities() {
        if let Some(entity_data) = world.get(entity_id) {
            if entity_data.parent_id.is_none() {
                // if it's a root level entity
                update_world_transform_recursive(
                    world,
                    archetype_storage_map,
                    entity_id,
                    cpu_data,
                    &Mat4::IDENTITY,
                );
            }
        }
    }
}

fn update_world_transform_recursive(
    world: &World,
    archetype_storage_map: &ArchetypeStorageMap,
    entity_id: EntityId,
    cpu_data: &mut CpuFrameData,
    parent_local_world: &Mat4,
) {
    let Some(entity_data) = world.get(entity_id) else {
        return;
    };

    let Some(storage) = archetype_storage_map.get(&entity_data.archetype_key) else {
        return;
    };
    let Some(transform_comp_offset) = storage.get_component_byte_offset(&Transform::id()) else {
        return;
    };

    let Some(world_transform_comp_offset) = storage.get_component_byte_offset(&LocalToWorld::id())
    else {
        return;
    };

    let mut buffer = cpu_data.get_buffer_mut(storage.cpu.buffer_index);
    let transform_state = unsafe {
        buffer
            .get_with_offset_as::<Transform>(entity_data.archetype_index, transform_comp_offset)
            .unwrap()
    };

    // convert local transform data into a local->parent affine matrix
    let mat_local_to_parent = Mat4::from_scale_rotation_translation(
        transform_state.scale.extend(1.),
        Quat::from_rotation_z(transform_state.rotation),
        *transform_state.position,
    );

    // multiply local->parent matrix by parent's local->world
    let local_to_world = parent_local_world.mul_mat4(&mat_local_to_parent);

    // update the local -> world matrix
    let world_transform_state_mut = unsafe {
        buffer
            .get_mut_with_offset_as::<LocalToWorld>(
                entity_data.archetype_index,
                world_transform_comp_offset,
            )
            .unwrap()
    };
    *world_transform_state_mut = local_to_world.into();

    for child_id in &entity_data.child_ids[..] {
        update_world_transform_recursive(
            world,
            archetype_storage_map,
            *child_id,
            cpu_data,
            &local_to_world,
        );
    }
}
