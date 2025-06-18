use game_module_macro::Component;
use void_public::{Component, ComponentId, EcsType};

#[repr(C)]
#[derive(Component, Debug, serde::Deserialize)]
pub struct BoxCollider;

#[repr(C)]
#[derive(Component, Debug, serde::Deserialize)]
pub struct CircleCollider;
