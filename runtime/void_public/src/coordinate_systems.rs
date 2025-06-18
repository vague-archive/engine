use glam::{Mat4, Quat, Vec2, Vec3, Vec4, Vec4Swizzles};

use crate::{Camera, LocalToWorld, Transform};

/// Convert the given world-space `position` into the local coordinate system of the given `camera`.
pub fn world_to_view(position: &Vec3, camera: &Camera) -> Vec3 {
    (*camera.view_matrix * position.extend(1f32)).truncate()
}

/// Convert the given world-space `position` into homogenous clip space of the given `camera`.
pub fn world_to_clip(position: &Vec3, camera: &Camera) -> Vec3 {
    let world_to_clip = get_world_to_clip_matrix(camera);
    (world_to_clip * position.extend(1f32)).truncate()
}

/// Convert the given world-space `position` into the screen space of the given `camera` at the given
/// `screen_dimensions` in pixels.
pub fn world_to_screen(position: &Vec3, screen_dimensions: &Vec2, camera: &Camera) -> Vec2 {
    let world_position = position.extend(1f32);
    let world_to_clip = get_world_to_clip_matrix(camera);

    let clip_to_screen = get_clip_to_screen_matrix(screen_dimensions.x, screen_dimensions.y);

    let world_to_screen = clip_to_screen * world_to_clip;

    (world_to_screen * world_position).xy()
}

/// Convert the given screen-space `screen_position` using the given `camera` and `screen_dimensions`
/// into a world-space position.
pub fn screen_to_world(screen_position: &Vec2, screen_dimensions: &Vec2, camera: &Camera) -> Vec3 {
    let screen_pos = Vec4::new(screen_position.x, screen_position.y, 0.0, 1.0);
    let screen_to_view =
        get_screen_to_view_matrix(camera, screen_dimensions.x, screen_dimensions.y);
    let screen_to_world = camera.view_matrix.inverse() * screen_to_view;

    // convert into world coordinates
    let world_coordinates = screen_to_world * screen_pos;

    world_coordinates.truncate()
}

/// Convert the given screen-space `screen_position` using the given `screen_dimensions` into its
/// homogenous clip space equivalent.
pub fn screen_to_clip(screen_position: &Vec2, screen_dimensions: &Vec2) -> Vec3 {
    let screen_position = Vec4::new(screen_position.x, screen_position.y, 0.0, 1.0);
    let clip_coordinates =
        get_screen_to_clip_matrix(screen_dimensions.x, screen_dimensions.y) * screen_position;

    clip_coordinates.truncate()
}

/// Convert the given screen-space `screen_position` using the `screen_dimensions`
/// into the local coordinate system of `camera`.
pub fn screen_to_view(screen_position: &Vec2, screen_dimensions: &Vec2, camera: &Camera) -> Vec3 {
    let screen_position = Vec4::new(screen_position.x, screen_position.y, 0.0, 1.0);
    let view_coordinates =
        get_screen_to_view_matrix(camera, screen_dimensions.x, screen_dimensions.y)
            * screen_position;

    view_coordinates.truncate()
}

/// Convert the given `world-position` into the local coordinate system represented by `local_to_world`
pub fn world_to_local(world_position: &Vec2, local_to_world: &LocalToWorld) -> Vec2 {
    local_to_world
        .inverse()
        .mul_vec4(Vec4::new(world_position.x, world_position.y, 0.0, 1.0))
        .xy()
}

/// Convert the given `local_position` in the coordinate system of `local_to_world` into world-space
#[inline]
pub fn local_to_world(local_position: &Vec2, local_to_world: &LocalToWorld) -> Vec2 {
    local_to_world
        .mul_vec4(Vec4::new(local_position.x, local_position.y, 0.0, 1.0))
        .xy()
}

/// Returns the world-space position of the entity associated with the given `LocalToWorld` component
#[inline]
pub fn world_position(local_to_world: &LocalToWorld) -> Vec2 {
    local_to_world.w_axis.xy()
}

/// Set the position of the given `transform` to world-space relative position using the associated
/// `local_to_world` matrix
pub fn set_world_position(
    transform: &mut Transform,
    local_to_world: &LocalToWorld,
    world_position: &Vec2,
) {
    let local_to_parent = Mat4::from_scale_rotation_translation(
        transform.scale.extend(0f32),
        Quat::from_rotation_z(transform.rotation),
        *transform.position,
    );

    let world_to_parent = local_to_parent * local_to_world.inverse();

    let new_local_pos = world_to_parent * Vec4::new(world_position.x, world_position.y, 0.0, 1.0);
    transform.position.x = new_local_pos.x;
    transform.position.y = new_local_pos.y;
}

fn get_screen_to_view_matrix(cam: &Camera, screen_width: f32, screen_height: f32) -> Mat4 {
    cam.projection_matrix.inverse() * get_screen_to_clip_matrix(screen_width, screen_height)
}

fn get_screen_to_clip_matrix(width: f32, height: f32) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(2.0 / width, 0.0, 0.0, 0.0),
        Vec4::new(0.0, -2.0 / height, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 0.0),
        Vec4::new(-1.0, 1.0, 0.0, 1.0),
    )
}

fn get_clip_to_screen_matrix(width: f32, height: f32) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(width * 0.5, 0.0, 0.0, 0.0),
        Vec4::new(0.0, -height * 0.5, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 0.0),
        Vec4::new(width * 0.5, height * 0.5, 0.0, 1.0),
    )
}

fn get_world_to_clip_matrix(cam: &Camera) -> Mat4 {
    *(cam.projection_matrix * cam.view_matrix)
}

#[cfg(test)]
mod tests {
    use assert_approx_eq::assert_approx_eq;
    use glam::{Mat4, Quat, Vec2, Vec3};

    use super::{
        screen_to_clip, screen_to_view, screen_to_world, set_world_position, world_to_clip,
        world_to_local, world_to_screen, world_to_view,
    };
    use crate::{Camera, Transform, coordinate_systems, linalg::create_orthographic_matrix};

    const DEFAULT_SCREEN_DIMENSIONS: Vec2 = Vec2::new(800.0, 600.0);

    #[test]
    fn test_screen_to_clip() {
        let screen_pos = Vec2::ZERO;

        let clip_coordinates = screen_to_clip(&screen_pos, &DEFAULT_SCREEN_DIMENSIONS);
        assert_eq!(clip_coordinates, Vec3::new(-1.0, 1.0, 0.0));

        let screen_pos = Vec2::new(400.0, 300.0);
        let clip_coordinates = screen_to_clip(&screen_pos, &DEFAULT_SCREEN_DIMENSIONS);
        assert_eq!(clip_coordinates, Vec3::new(0.0, 0.0, 0.0));
    }

    #[test]
    fn test_screen_to_view() {
        let camera = create_test_camera(
            &Transform::from_rotation_translation(45f32.to_radians(), &Vec2::new(100., 100.)),
            1f32,
            &DEFAULT_SCREEN_DIMENSIONS,
        );

        let screen_pos = DEFAULT_SCREEN_DIMENSIONS * 0.5;

        let cam_coordinates = screen_to_view(&screen_pos, &DEFAULT_SCREEN_DIMENSIONS, &camera);
        assert_approx_eq!(0.0f32, cam_coordinates.x);
        assert_approx_eq!(0.0f32, cam_coordinates.y);
    }

    #[test]
    fn test_screen_to_world() {
        let screen_pos = DEFAULT_SCREEN_DIMENSIONS * 0.5f32;

        let cam = create_test_camera(
            &Transform::from_rotation_translation(45f32.to_radians(), &Vec2::new(10., 10.)),
            1f32,
            &DEFAULT_SCREEN_DIMENSIONS,
        );

        let pos_in_world_space = screen_to_world(
            &screen_pos,
            &Vec2::new(DEFAULT_SCREEN_DIMENSIONS.x, DEFAULT_SCREEN_DIMENSIONS.y),
            &cam,
        );

        assert_approx_eq!(10.0f32, pos_in_world_space.x);
        assert_approx_eq!(10.0f32, pos_in_world_space.y);
    }

    #[test]
    fn test_world_to_view() {
        let world_position = Vec3::ZERO;

        let transform =
            &Transform::from_rotation_translation(90.0f32.to_radians(), &Vec2::new(10.0, 0.0));

        let cam = create_test_camera(transform, 1f32, &DEFAULT_SCREEN_DIMENSIONS);

        let pos_in_camera_space = world_to_view(&world_position, &cam);

        assert_approx_eq!(0.0f32, pos_in_camera_space.x);
        assert_approx_eq!(10.0f32, pos_in_camera_space.y);
    }

    #[test]
    fn test_world_to_clip() {
        let world_position = Vec3::ZERO;

        let transform =
            Transform::from_rotation_translation(45f32.to_radians(), &Vec2::new(10.0, 10.0));

        let cam = create_test_camera(&transform, 1f32, &DEFAULT_SCREEN_DIMENSIONS);

        let pos_in_clip_space = world_to_clip(&world_position, &cam);
        assert_approx_eq!(
            f32::sqrt(200.0) / (-DEFAULT_SCREEN_DIMENSIONS.x * 0.5),
            pos_in_clip_space.x
        );
        assert_approx_eq!(0.0f32, pos_in_clip_space.y);
    }

    #[test]
    fn test_world_to_screen() {
        let world_position = Vec3::ZERO;

        let transform =
            Transform::from_rotation_translation(45f32.to_radians(), &Vec2::new(10.0, 10.0));

        let cam = create_test_camera(&transform, 1f32, &DEFAULT_SCREEN_DIMENSIONS);

        let pos_in_screen_space =
            world_to_screen(&world_position, &DEFAULT_SCREEN_DIMENSIONS, &cam);
        assert_approx_eq!(
            (0.5f32 * DEFAULT_SCREEN_DIMENSIONS.x) - f32::sqrt(200.0),
            pos_in_screen_space.x,
            1e-4
        );
        assert_approx_eq!(
            (0.5f32 * DEFAULT_SCREEN_DIMENSIONS.y),
            pos_in_screen_space.y
        );
    }

    #[test]
    fn test_local_to_world() {
        let local_position = Vec2::new(1.0, 1.0);
        let local_to_world = Mat4::from_scale_rotation_translation(
            Vec3::new(10.0, 10.0, 10.0),
            Quat::IDENTITY,
            Vec3::new(5.0, -6.0, 0.0),
        )
        .into();

        let world_position = coordinate_systems::local_to_world(&local_position, &local_to_world);
        assert_eq!(15.0f32, world_position.x);
        assert_eq!(4.0f32, world_position.y);

        let local_position = Vec2::new(7.0, -13.0);
        let local_to_world = Mat4::from_scale_rotation_translation(
            Vec3::new(10.0, 10.0, 10.0),
            Quat::from_rotation_z(45f32.to_radians()),
            Vec3::new(5.0, -2.0, 0.0),
        )
        .into();

        let world_position = coordinate_systems::local_to_world(&local_position, &local_to_world);
        assert_approx_eq!(146.4213f32, world_position.x, 1e-4);
        assert_approx_eq!(-44.4264f32, world_position.y, 1e-4);
    }

    #[test]
    fn test_world_to_local() {
        let world_position = Vec2::ZERO;
        let local_to_world = Mat4::from_scale_rotation_translation(
            Vec3::new(10.0, 10.0, 10.0),
            Quat::IDENTITY,
            Vec3::new(5.0, -6.0, 0.0),
        )
        .into();

        let local_position = world_to_local(&world_position, &local_to_world);
        assert_eq!(-0.5f32, local_position.x);
        assert_eq!(0.6f32, local_position.y);

        let world_position = Vec2::new(7.0, -13.0);
        let local_to_world = Mat4::from_scale_rotation_translation(
            Vec3::new(3.0, 3.0, 3.0),
            Quat::from_rotation_z(30f32.to_radians()),
            Vec3::new(5.0, -2.0, 0.0),
        )
        .into();

        let world_position = coordinate_systems::world_to_local(&world_position, &local_to_world);
        assert_approx_eq!(-1.25598f32, world_position.x, 1e-3);
        assert_approx_eq!(-3.50849f32, world_position.y, 1e-3);
    }

    #[test]
    fn test_set_world_position() {
        let scale = 100f32;
        let rotation = 90.0f32.to_radians();
        let translation = Vec2::new(10.0, 0.0);
        let mut transform =
            Transform::from_scale_rotation_translation(&Vec2::splat(scale), rotation, &translation);
        let local_to_world = Mat4::from_scale_rotation_translation(
            Vec3::splat(3.0),
            Quat::from_rotation_z(0.0f32.to_radians()),
            Vec3::new(5.0, -2.0, 0.0),
        )
        .into();

        let world_position = Vec2::new(16.0, 5.0);

        set_world_position(&mut transform, &local_to_world, &world_position);

        assert_approx_eq!(-670.0f32 / 3.0f32, transform.position.x, 1e-3);
        assert_approx_eq!(1100.0f32 / 3.0f32, transform.position.y, 1e-3);
    }

    fn create_test_camera(
        transform: &Transform,
        orthographic_size: f32,
        screen_dimensions: &Vec2,
    ) -> Camera {
        let local_to_world = Mat4::from_scale_rotation_translation(
            transform.scale.extend(1.0f32),
            Quat::from_rotation_z(transform.rotation),
            *transform.position,
        );

        let view_matrix =
            Mat4::from_scale(Vec3::splat(orthographic_size)) * local_to_world.inverse();
        Camera {
            view_matrix: view_matrix.into(),
            projection_matrix: create_orthographic_matrix(
                -screen_dimensions.x * 0.5,
                screen_dimensions.x * 0.5,
                -screen_dimensions.y * 0.5,
                screen_dimensions.y * 0.5,
                -4096.0,
                4096.0,
            )
            .into(),
            ..Default::default()
        }
    }
}
