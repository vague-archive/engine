use std::path::PathBuf;

use game_asset::{
    ecs_module::GpuInterface,
    resource_managers::material_manager::material_parameters_extension::MaterialParametersExt,
};
use game_module_macro::system_once;
use void_public::{
    EcsType, Engine, EventWriter, Transform, Vec2, bundle, colors::palette,
    event::graphics::NewTexture, graphics::TextureRender, linalg, material::DefaultMaterials,
};

#[system_once]
#[allow(clippy::needless_pass_by_value)]
fn load_texture(
    gpu_interface: &mut GpuInterface,
    new_texture_event_writer: EventWriter<NewTexture<'_>>,
) {
    let pending_texture = gpu_interface
        .texture_asset_manager
        .load_texture(
            &PathBuf::from("scared.png").into(),
            false,
            &new_texture_event_writer,
        )
        .unwrap();

    let texture_id = pending_texture.id();
    let texture_render = TextureRender::new(texture_id);

    let transform = Transform {
        scale: linalg::Vec2::new(Vec2::splat(100.)),
        ..Default::default()
    };

    let default_material = gpu_interface
        .material_manager
        .get_material(DefaultMaterials::Sprite.material_id())
        .unwrap();

    let white_color = palette::WHITE;

    let material_parameters = default_material
        .generate_default_material_parameters()
        .update_uniform(
            &gpu_interface.material_manager,
            &("color_param_1", &(**white_color).into()),
        )
        .unwrap()
        .update_texture(&gpu_interface.material_manager, &("color_tex", &texture_id))
        .unwrap()
        .end_chain();

    Engine::spawn(bundle!(
        &texture_render,
        &transform,
        &white_color,
        &material_parameters
    ));
}

// This includes auto-generated C FFI code (saves you from writing it manually).
include!(concat!(env!("OUT_DIR"), "/ffi.rs"));
