//! Steam Audio scene built from the client BSP world mesh + probe batch.
//! Ported from game-lib client-audio/scene_builder.rs (data source differs:
//! we already have a resolved Mesh + BspFaceDebug with texture names).

use audionimbus::geometry::{
    Point, Scene, SceneSettings, StaticMesh, StaticMeshSettings, Triangle,
};
use audionimbus::{Material, ProbeBatch, SteamAudioError};
use deerpunk_engine_common::resource::mesh::Mesh;
use glam::Vec3;

use crate::steam_audio::METERS_IN_UNIT;

/// Xash: x right, y forward, z up. Steam Audio: x right, y up, -z ahead.
#[inline]
fn xash_to_steam(v: Vec3) -> Point {
    audionimbus::Vector3::new(
        v.x * METERS_IN_UNIT,
        v.z * METERS_IN_UNIT,
        -v.y * METERS_IN_UNIT,
    )
}

fn compute_aabb(vertices: &[Point]) -> Option<(Point, Point)> {
    let mut iter = vertices.iter();
    let first = *iter.next()?;
    let (mut min_x, mut min_y, mut min_z) = (first.x, first.y, first.z);
    let (mut max_x, mut max_y, mut max_z) = (first.x, first.y, first.z);
    for p in iter {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        min_z = min_z.min(p.z);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
        max_z = max_z.max(p.z);
    }
    Some((
        Point::new(min_x, min_y, min_z),
        Point::new(max_x, max_y, max_z),
    ))
}

pub struct WorldScene {
    pub scene: Scene,
    /// Must outlive simulator use of `scene` (explicit retain).
    pub static_meshes: Vec<StaticMesh>,
    /// Scene AABB in Steam Audio coords (meters) — used for probe generation.
    pub aabb: (Point, Point),
}

/// BISCT: one non-degenerate triangle in Steam coords (meters).
pub fn build_tiny_scene(context: &audionimbus::Context) -> Result<WorldScene, SteamAudioError> {
    let vertices = [
        Point::new(0.0, 0.0, 0.0),
        Point::new(1.0, 0.0, 0.0),
        Point::new(0.0, 0.0, 1.0),
    ];
    let triangles = [Triangle::new(0, 1, 2)];
    let materials = [Material::CONCRETE];
    let material_indices = [0usize];
    let aabb = (Point::new(0.0, 0.0, 0.0), Point::new(1.0, 0.0, 1.0));

    let mut scene = Scene::try_new(context, &SceneSettings::Default)?;
    let mesh_settings = StaticMeshSettings {
        vertices: &vertices,
        triangles: &triangles,
        material_indices: &material_indices,
        materials: &materials,
    };
    let static_mesh = StaticMesh::try_new(&scene, &mesh_settings)?;
    let retained = static_mesh.clone();
    scene.add_static_mesh(static_mesh);
    scene.commit();

    Ok(WorldScene {
        scene,
        static_meshes: vec![retained],
        aabb,
    })
}

/// Builds a Steam Audio scene from the world mesh (uniform concrete material).
pub fn build_scene_from_mesh(
    context: &audionimbus::Context,
    mesh: &Mesh,
) -> Result<WorldScene, SteamAudioError> {
    let mut vertices: Vec<Point> = Vec::with_capacity(mesh.positions.len());
    for p in &mesh.positions {
        vertices.push(xash_to_steam(Vec3::from_array(*p)));
    }

    let tri_count = mesh.indices.len() / 3;
    let mut triangles: Vec<Triangle> = Vec::with_capacity(tri_count);
    let mut material_indices: Vec<usize> = Vec::with_capacity(tri_count);
    let mut materials: Vec<Material> = Vec::new();

    let nverts = vertices.len() as i32;
    for (t, tri) in mesh.indices.chunks_exact(3).enumerate() {
        let (a, b, c) = (tri[0] as i32, tri[1] as i32, tri[2] as i32);
        // Skip degenerate / OOB — phonon BVH heap-corrupts on bad tris.
        if a == b
            || b == c
            || a == c
            || a < 0
            || b < 0
            || c < 0
            || a >= nverts
            || b >= nverts
            || c >= nverts
        {
            continue;
        }
        let va = vertices[a as usize];
        let vb = vertices[b as usize];
        let vc = vertices[c as usize];
        let ab = (vb.x - va.x, vb.y - va.y, vb.z - va.z);
        let ac = (vc.x - va.x, vc.y - va.y, vc.z - va.z);
        let cross = (
            ab.1 * ac.2 - ab.2 * ac.1,
            ab.2 * ac.0 - ab.0 * ac.2,
            ab.0 * ac.1 - ab.1 * ac.0,
        );
        if cross.0 * cross.0 + cross.1 * cross.1 + cross.2 * cross.2 < 1e-12 {
            continue;
        }

        triangles.push(Triangle::new(a, b, c));
        let mat = Material::CONCRETE;
        let idx = materials.iter().position(|m| *m == mat).unwrap_or_else(|| {
            materials.push(mat);
            materials.len() - 1
        });
        material_indices.push(idx);
    }

    if vertices.is_empty() || triangles.is_empty() {
        return Err(SteamAudioError::Initialization);
    }

    let aabb = compute_aabb(&vertices).ok_or(SteamAudioError::Initialization)?;

    let mut scene = Scene::try_new(context, &SceneSettings::Default)?;
    let mesh_settings = StaticMeshSettings {
        vertices: &vertices,
        triangles: &triangles,
        material_indices: &material_indices,
        materials: &materials,
    };
    let static_mesh = StaticMesh::try_new(&scene, &mesh_settings)?;
    // Retain: keep Rust owner; scene Add also retains.
    let retained = static_mesh.clone();
    scene.add_static_mesh(static_mesh);
    scene.commit();

    Ok(WorldScene {
        scene,
        static_meshes: vec![retained],
        aabb,
    })
}

/// Generates a floor probe batch inside the scene AABB (for pathing).
/// Reference culls probes against traces; we keep all — fine for pathing.
pub fn build_probe_batch(
    context: &audionimbus::Context,
    scene: &Scene,
    aabb: (Point, Point),
) -> Result<ProbeBatch, SteamAudioError> {
    const PROBE_MARGIN: f32 = 0.1;
    const PROBE_SPACING: f32 = 1.5;
    const PROBE_HEIGHT: f32 = 1.5;

    let mut probe_batch = ProbeBatch::try_new(context)?;

    let (min, max) = aabb;
    let min_x = min.x - PROBE_MARGIN;
    let min_y = min.y - PROBE_MARGIN;
    let min_z = min.z - PROBE_MARGIN;
    let max_x = max.x + PROBE_MARGIN;
    let max_y = max.y + PROBE_MARGIN;
    let max_z = max.z + PROBE_MARGIN;
    let (sx, sy, sz) = (max_x - min_x, max_y - min_y, max_z - min_z);
    let (cx, cy, cz) = (
        (min_x + max_x) * 0.5,
        (min_y + max_y) * 0.5,
        (min_z + max_z) * 0.5,
    );

    let floor_transform = audionimbus::Matrix::new([
        [sx, 0.0, 0.0, cx],
        [0.0, sy, 0.0, cy],
        [0.0, 0.0, sz, cz],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    let mut array = audionimbus::ProbeArray::try_new(context)?;
    array.generate_probes(
        scene,
        &audionimbus::ProbeGenerationParams::UniformFloor {
            spacing: PROBE_SPACING,
            height: PROBE_HEIGHT,
            transform: floor_transform,
        },
    );
    for idx in 0..array.num_probes() {
        probe_batch.add_probe(&array.probe(idx));
    }

    probe_batch.commit();
    Ok(probe_batch)
}
