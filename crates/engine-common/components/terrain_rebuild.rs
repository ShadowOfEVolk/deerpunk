/// Marker component: terrain mesh needs rebuild from heightmap data.
///
/// Set this on an entity that has a Terrain component + Mesh3D.
/// The rebuild system reads Terrain, calls build_mesh(), updates Mesh3D, clears this marker.
#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct TerrainNeedRebuild;
