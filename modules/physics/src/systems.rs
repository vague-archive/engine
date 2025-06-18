use std::{collections::HashSet, num::NonZero};

use game_module_macro::system;
use void_public::{
    event::physics::{BoxCollision, CircleCollision},
    EcsType, EntityId, EventWriter, Query, Transform, Vec3Swizzles,
};

use crate::components::{self, BoxCollider, CircleCollider};

pub mod ffi {
    #![allow(clippy::all, clippy::pedantic, warnings, unused, unused_imports)]
    use super::*;
    use crate::systems;

    include!(concat!(env!("OUT_DIR"), "/ffi.rs"));
}

#[allow(clippy::needless_pass_by_value)]
#[system]
fn box_collisions(
    box_colliders: Query<(&Transform, &EntityId, &BoxCollider)>,
    box_collisions: EventWriter<BoxCollision>,
) {
    let mut event_ids_sent: HashSet<(EntityId, EntityId)> = HashSet::new();

    // TODO: Replace with par_for_each.
    // https://github.com/vaguevoid/engine/issues/425
    box_colliders.iter().for_each(|components| {
        let (transform1, entity_id1, _) = components.unpack();

        box_colliders.iter().for_each(|components2| {
            let (transform2, entity_id2, _) = components2.unpack();

            if entity_id1 != entity_id2
                && box_overlap(transform1, transform2)
                && !event_ids_sent.contains(&(**entity_id1, **entity_id2))
                && !event_ids_sent.contains(&(**entity_id2, **entity_id1))
            {
                event_ids_sent.insert((**entity_id1, **entity_id2));

                box_collisions.write(BoxCollision::new(&[
                    NonZero::from(game_entity::EntityId::from(**entity_id1)).get(),
                    NonZero::from(game_entity::EntityId::from(**entity_id2)).get(),
                ]));
            }
        });
    });
}

#[allow(clippy::needless_pass_by_value)]
#[system]
fn circle_collisions(
    circle_colliders: Query<(&Transform, &EntityId, &CircleCollider)>,
    circle_collisions: EventWriter<CircleCollision>,
) {
    let mut event_ids_sent: HashSet<(EntityId, EntityId)> = HashSet::new();

    // TODO: Replace with par_for_each.
    // https://github.com/vaguevoid/engine/issues/425
    circle_colliders.iter().for_each(|components| {
        let (transform1, entity_id1, _) = components.unpack();

        circle_colliders.iter().for_each(|components2| {
            let (transform2, entity_id2, _) = components2.unpack();

            if entity_id1 != entity_id2
                && circle_overlap(transform1, transform2)
                && !event_ids_sent.contains(&(**entity_id1, **entity_id2))
                && !event_ids_sent.contains(&(**entity_id2, **entity_id1))
            {
                event_ids_sent.insert((**entity_id1, **entity_id2));

                circle_collisions.write(CircleCollision::new(&[
                    NonZero::from(game_entity::EntityId::from(**entity_id1)).get(),
                    NonZero::from(game_entity::EntityId::from(**entity_id2)).get(),
                ]));
            }
        });
    });
}

fn box_overlap(t1: &Transform, t2: &Transform) -> bool {
    let rect1_min_x = t1.position.x - t1.scale.x / 2.0;
    let rect1_max_x = t1.position.x + t1.scale.x / 2.0;
    let rect1_min_y = t1.position.y - t1.scale.y / 2.0;
    let rect1_max_y = t1.position.y + t1.scale.y / 2.0;

    let rect2_min_x = t2.position.x - t2.scale.x / 2.0;
    let rect2_max_x = t2.position.x + t2.scale.x / 2.0;
    let rect2_min_y = t2.position.y - t2.scale.y / 2.0;
    let rect2_max_y = t2.position.y + t2.scale.y / 2.0;

    rect1_min_x < rect2_max_x
        && rect1_max_x > rect2_min_x
        && rect1_min_y < rect2_max_y
        && rect1_max_y > rect2_min_y
}

fn circle_overlap(t1: &Transform, t2: &Transform) -> bool {
    let r1 = t1.scale.x.max(t1.scale.y) / 2.0;
    let r2 = t2.scale.x.max(t2.scale.y) / 2.0;

    let distance = t2.position.xy().distance(t1.position.xy());
    let radii_sum = r1 + r2;

    distance <= radii_sum
}
