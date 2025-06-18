use std::{
    f32::consts::PI,
    mem::size_of,
    ops::{Add, Deref, Mul},
};

use void_public::{
    Vec2, Vec3,
    colors::{Color, palette},
};
use wgpu::{BufferAddress, VertexAttribute, VertexBufferLayout, VertexStepMode};

#[derive(Clone, Debug)]
pub struct Shape2d {
    pub num_sides: usize,
    pub position: Vec3,
    pub color: Color,
}

impl Shape2d {
    pub fn generate_default_quad(vertices: &mut Vec<Vertex2d>) {
        vertices.push(Vertex2d::new(
            Vec3::new(1., 1., 1.),
            Vec2::new(1., 0.),
            palette::WHITE,
        ));
        vertices.push(Vertex2d::new(
            Vec3::new(-1., 1., 1.),
            Vec2::new(0., 0.),
            palette::WHITE,
        ));
        vertices.push(Vertex2d::new(
            Vec3::new(-1., -1., 1.),
            Vec2::new(0., 1.),
            palette::WHITE,
        ));
        vertices.push(Vertex2d::new(
            Vec3::new(1., 1., 1.),
            Vec2::new(1., 0.),
            palette::WHITE,
        ));
        vertices.push(Vertex2d::new(
            Vec3::new(-1., -1., 1.),
            Vec2::new(0., 1.),
            palette::WHITE,
        ));
        vertices.push(Vertex2d::new(
            Vec3::new(1., -1., 1.),
            Vec2::new(1., 1.),
            palette::WHITE,
        ));
    }

    pub fn generate_line(
        vertices: &mut Vec<Vertex2d>,
        from: &Vec3,
        to: &Vec3,
        thickness: f32,
        color: Color,
    ) {
        let from_to = (to - from).normalize();
        let cross_vec = Vec3::new(from_to.y, -from_to.x, 0.0) * (thickness * 0.5);

        vertices.push(Vertex2d::new(to + cross_vec, Vec2::new(0.0, 0.0), color));
        vertices.push(Vertex2d::new(from + cross_vec, Vec2::new(0.0, 1.0), color));
        vertices.push(Vertex2d::new(from - cross_vec, Vec2::new(1.0, 1.0), color));

        vertices.push(Vertex2d::new(to + cross_vec, Vec2::new(0.0, 0.0), color));
        vertices.push(Vertex2d::new(from - cross_vec, Vec2::new(1.0, 1.0), color));
        vertices.push(Vertex2d::new(to - cross_vec, Vec2::new(1.0, 0.0), color));
    }

    pub fn generate_circle(&self) -> impl Iterator<Item = Vertex2d> {
        if self.num_sides == 0 || self.num_sides > 128 {
            panic!(
                "Shape2d::calculate_vertices() - num_sides [{}] is either 0 or > 128",
                self.num_sides
            );
        }

        let vertex_rotation = Shape2dRotation(2.0 * PI / self.num_sides as f32);

        let center_vert = Vertex2d::new(
            Vec3::new(0.0, 0.0, self.position.z),
            Vec2::new(0.5, 0.5),
            self.color,
        );

        let mut last_vert_local = {
            if self.num_sides == 4 {
                Vertex2d::new(
                    Vec3::new(-1.0, 1.0, self.position.z),
                    Vec2::new(0.0, 1.0),
                    self.color,
                )
            } else {
                Vertex2d::new(
                    Vec3::new(0.0, 1.0, self.position.z),
                    Vec2::new(0.5, 1.0),
                    self.color,
                )
            }
        };

        (0..self.num_sides).flat_map(move |_| {
            let mut cur_vert_local = last_vert_local * vertex_rotation;
            cur_vert_local.tex_coords[0] = cur_vert_local.position[0] * 0.5 + 0.5;
            cur_vert_local.tex_coords[1] = cur_vert_local.position[1] * 0.5 + 0.5;

            let res = [center_vert, last_vert_local, cur_vert_local];

            last_vert_local = cur_vert_local;

            res
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Shape2dRotation(f32);

impl Deref for Shape2dRotation {
    type Target = f32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Mul<Shape2dRotation> for Vertex2d {
    type Output = Vertex2d;

    fn mul(self, rhs: Shape2dRotation) -> Vertex2d {
        let cos_angle = rhs.cos();
        let sin_angle = rhs.sin();
        let position = Vec3::new(
            self.position.x * cos_angle + self.position.y * -sin_angle,
            self.position.x * sin_angle + self.position.y * cos_angle,
            self.position.z,
        );

        Vertex2d::new(position, self.tex_coords, self.color)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Shape2dTranslation(Vec3);

impl Deref for Shape2dTranslation {
    type Target = Vec3;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Add<Shape2dTranslation> for Vertex2d {
    type Output = Vertex2d;

    fn add(self, rhs: Shape2dTranslation) -> Vertex2d {
        let position = Vec3::new(
            self.position[0] + rhs.0.x,
            self.position[1] + rhs.0.y,
            self.position[2],
        );
        Vertex2d::new(position, self.tex_coords, self.color)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Shape2dScale(Vec2);

impl Deref for Shape2dScale {
    type Target = Vec2;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Mul<Shape2dScale> for Vertex2d {
    type Output = Vertex2d;

    fn mul(self, rhs: Shape2dScale) -> Vertex2d {
        let position = Vec3::new(
            self.position[0] * rhs.x,
            self.position[1] * rhs.y,
            self.position[2],
        );
        Vertex2d::new(position, self.tex_coords, self.color)
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex2d {
    pub color: Color,
    pub position: Vec3,
    pub tex_coords: Vec2,
    _padding: [f32; 3],
}

impl Vertex2d {
    pub fn new(position: Vec3, tex_coords: Vec2, color: Color) -> Self {
        Self {
            color,
            position,
            tex_coords,
            _padding: [0.0f32; 3],
        }
    }
}

impl Vertex2d {
    pub fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: size_of::<Vertex2d>() as BufferAddress,
            step_mode: VertexStepMode::Vertex,
            attributes: &[
                VertexAttribute {
                    // color
                    offset: 0 as BufferAddress,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x4,
                },
                VertexAttribute {
                    // position
                    offset: size_of::<[f32; 4]>() as BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                VertexAttribute {
                    // tex_coords
                    offset: size_of::<[f32; 4 + 3]>() as BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}
