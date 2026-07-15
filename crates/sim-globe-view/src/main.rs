//! Standalone viewer for `sim_core::geo::GeoMesh` — the Tier-1 global
//! geodesic mesh (see `planning/development-log.md`, "Global map
//! expansion"). This is a development/inspection tool, not part of the
//! game: it renders the mesh's Goldberg-polyhedron dual (each tile's
//! actual hex/pentagon face, via `GeoMesh::tile_polygon`) as a real 3D
//! globe, colored by land/sea classification baked by
//! `tools/preprocess/preprocess_geo_tiles.py` into
//! `data/grids/geo_tiles.bin`. It does not know about ports, rivers, or
//! the shipping-lane graph yet — those land on top of this mesh in later
//! milestones.
//!
//! Run from the workspace root (it reads `data/grids/geo_tiles.bin`
//! relative to the current directory, same convention as `sim-viz`).
//!
//! Controls:
//!   drag         orbit the globe
//!   wheel        zoom
//!   E            toggle tile-edge wireframe overlay
//!   P            toggle pentagon markers
//!   R            reset camera

use macroquad::models::{draw_mesh, Mesh, Vertex};
use macroquad::prelude::*;
use sim_core::geo::{GeoMesh, LandSea, TileId, DEFAULT_SUBDIVISIONS};
use std::path::Path;

/// `sim_core::geo` positions come from sim-core's glam (0.29); macroquad
/// vendors its own older glam (0.27) and re-exports it as `glam` via
/// `macroquad::prelude`, so the two `Vec3`s are distinct types despite
/// the identical name (see the `core_glam` comment in Cargo.toml).
/// Convert at the boundary.
fn mq(v: core_glam::Vec3) -> macroquad::math::Vec3 {
    macroquad::math::Vec3::new(v.x, v.y, v.z)
}

const OCEAN_COLOR: Color = Color::new(0.06, 0.14, 0.26, 1.0);
const LAND_COLOR: Color = Color::new(0.22, 0.42, 0.19, 1.0);
const BG_COLOR: Color = Color::new(0.02, 0.02, 0.035, 1.0);
const EDGE_COLOR: Color = Color::new(0.0, 0.0, 0.0, 0.35);
const PENTAGON_COLOR: Color = Color::new(1.0, 0.75, 0.25, 1.0);
const HUD_COLOR: Color = Color::new(0.92, 0.94, 0.98, 1.0);

const GLOBE_RADIUS: f32 = 1.0;
const MIN_DISTANCE: f32 = 1.4;
const MAX_DISTANCE: f32 = 6.0;

/// Orbit camera: position tracked as spherical angles around a fixed
/// target at the origin (the globe's center).
struct OrbitCamera {
    yaw: f32,
    pitch: f32,
    distance: f32,
    dragging: bool,
    last_mouse: Vec2,
}

impl OrbitCamera {
    fn new() -> Self {
        Self {
            yaw: 0.6,
            pitch: 0.35,
            distance: 3.0,
            dragging: false,
            last_mouse: Vec2::ZERO,
        }
    }

    fn reset(&mut self) {
        self.yaw = 0.6;
        self.pitch = 0.35;
        self.distance = 3.0;
    }

    fn position(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * cy, sp, cp * sy) * self.distance
    }

    fn to_camera3d(&self) -> Camera3D {
        Camera3D {
            position: self.position(),
            target: Vec3::ZERO,
            up: Vec3::Y,
            ..Default::default()
        }
    }

    fn handle_input(&mut self) {
        let wheel = mouse_wheel().1;
        if wheel != 0.0 {
            let factor = if wheel > 0.0 { 0.9 } else { 1.0 / 0.9 };
            self.distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
        }

        let (mx, my) = mouse_position();
        let mouse = Vec2::new(mx, my);
        if is_mouse_button_pressed(MouseButton::Left) {
            self.dragging = true;
            self.last_mouse = mouse;
        }
        if is_mouse_button_released(MouseButton::Left) {
            self.dragging = false;
        }
        if self.dragging {
            let delta = mouse - self.last_mouse;
            self.yaw += delta.x * 0.008;
            // Clamp pitch shy of the poles: at exactly +/- PI/2 `up` and
            // the view direction become parallel and the orbit flips.
            self.pitch = (self.pitch + delta.y * 0.008).clamp(-1.5, 1.5);
            self.last_mouse = mouse;
        }
    }
}

/// Tiles per mesh chunk. `Mesh::indices` is `u16`, so a single mesh can
/// address at most 65,536 vertices; at up to 7 vertices/tile (1 hub + 6
/// corners), 9,000 tiles/chunk stays safely under that (63,000). At
/// `DEFAULT_SUBDIVISIONS` (40,962 tiles) this yields 5 chunks.
const TILES_PER_CHUNK: usize = 9_000;

/// Builds the globe as several static triangle-mesh chunks (see
/// `TILES_PER_CHUNK`): for each tile, a fan of `(center, corner_i,
/// corner_i+1)` triangles colored by land/sea. Corners come from
/// `GeoMesh::tile_polygon`, which is watertight across tile boundaries
/// (see the crate's unit tests), so each chunk — and the union of all of
/// them — is a sealed surface with no cracks; depth testing alone hides
/// the far hemisphere correctly.
fn build_globe_mesh(mesh: &GeoMesh, land_sea: &LandSea) -> Vec<Mesh> {
    let mut chunks = Vec::new();
    let mut vertices = Vec::with_capacity(TILES_PER_CHUNK * 7);
    let mut indices = Vec::with_capacity(TILES_PER_CHUNK * 18);

    for tile in 0..mesh.tile_count() as TileId {
        let color = if land_sea.is_land(tile) {
            LAND_COLOR
        } else {
            OCEAN_COLOR
        };
        let center = mesh.centers[tile as usize] * GLOBE_RADIUS;
        let corners = mesh.tile_polygon(tile);

        let hub_index = vertices.len() as u16;
        vertices.push(vertex_at(mq(center), color));
        let first_corner_index = vertices.len() as u16;
        for c in &corners {
            vertices.push(vertex_at(mq(*c * GLOBE_RADIUS), color));
        }
        let deg = corners.len() as u16;
        for i in 0..deg {
            indices.push(hub_index);
            indices.push(first_corner_index + i);
            indices.push(first_corner_index + (i + 1) % deg);
        }

        if (tile as usize + 1).is_multiple_of(TILES_PER_CHUNK) {
            chunks.push(Mesh {
                vertices: std::mem::take(&mut vertices),
                indices: std::mem::take(&mut indices),
                texture: None,
            });
        }
    }
    if !vertices.is_empty() {
        chunks.push(Mesh {
            vertices,
            indices,
            texture: None,
        });
    }
    chunks
}

fn vertex_at(position: Vec3, color: Color) -> Vertex {
    Vertex::new(position.x, position.y, position.z, 0.0, 0.0, color)
}

fn draw_edges(mesh: &GeoMesh) {
    let lift = 1.001; // nudge slightly off the surface to avoid z-fighting
    for tile in 0..mesh.tile_count() as u32 {
        let a = mesh.centers[tile as usize];
        for &neighbor in &mesh.neighbors[tile as usize] {
            if neighbor <= tile {
                continue;
            }
            let b = mesh.centers[neighbor as usize];
            draw_line_3d(mq(a * lift), mq(b * lift), EDGE_COLOR);
        }
    }
}

fn draw_pentagon_markers(mesh: &GeoMesh) {
    for tile in 0..mesh.tile_count() as TileId {
        if mesh.is_pentagon(tile) {
            let p = mesh.centers[tile as usize] * 1.004;
            draw_sphere(mq(p), 0.012, None, PENTAGON_COLOR);
        }
    }
}

fn draw_hud(
    subdivisions: u32,
    tile_count: usize,
    land_count: usize,
    show_edges: bool,
    show_pentagons: bool,
) {
    let sea_count = tile_count - land_count;
    let land_pct = 100.0 * land_count as f32 / tile_count as f32;
    let lines = [
        format!("Tier-1 geodesic mesh  (n={subdivisions})"),
        format!("{tile_count} tiles — {land_count} land ({land_pct:.1}%), {sea_count} sea"),
        String::new(),
        format!(
            "E  edges (currently {})",
            if show_edges { "on" } else { "off" }
        ),
        format!(
            "P  pentagon markers (currently {})",
            if show_pentagons { "on" } else { "off" }
        ),
        "drag / wheel  orbit / zoom".to_string(),
        "R  reset camera".to_string(),
    ];
    for (i, line) in lines.iter().enumerate() {
        draw_text(line, 14.0, 24.0 + i as f32 * 18.0, 18.0, HUD_COLOR);
    }
}

#[macroquad::main("Sim Globe View - Tier-1 Geodesic Mesh")]
async fn main() {
    let mesh = GeoMesh::build(DEFAULT_SUBDIVISIONS);
    let land_sea = LandSea::load(Path::new("data/grids/geo_tiles.bin"));
    assert_eq!(
        land_sea.subdivisions, mesh.subdivisions,
        "geo_tiles.bin was baked for a different subdivision level than DEFAULT_SUBDIVISIONS \
         — regenerate it with tools/preprocess/preprocess_geo_tiles.py"
    );
    assert_eq!(
        land_sea.is_land.len(),
        mesh.tile_count(),
        "geo_tiles.bin tile count doesn't match the mesh"
    );
    let land_count = land_sea.is_land.iter().filter(|&&l| l).count();
    let globe_mesh_chunks = build_globe_mesh(&mesh, &land_sea);

    // macroquad's default per-draw-call buffer (10k vertices / 5k
    // indices) is far smaller than one of our mesh chunks (up to 63,000
    // vertices / 162,000 indices) — raise it to fit, or `draw_mesh`
    // silently clamps and most of the globe goes missing.
    macroquad::window::gl_set_drawcall_buffer_capacity(70_000, 200_000);

    let mut camera = OrbitCamera::new();
    let mut show_edges = false;
    let mut show_pentagons = true;

    loop {
        if is_key_pressed(KeyCode::E) {
            show_edges = !show_edges;
        }
        if is_key_pressed(KeyCode::P) {
            show_pentagons = !show_pentagons;
        }
        if is_key_pressed(KeyCode::R) {
            camera.reset();
        }
        camera.handle_input();

        clear_background(BG_COLOR);

        set_camera(&camera.to_camera3d());
        for chunk in &globe_mesh_chunks {
            draw_mesh(chunk);
        }
        if show_edges {
            draw_edges(&mesh);
        }
        if show_pentagons {
            draw_pentagon_markers(&mesh);
        }

        set_default_camera();
        draw_hud(
            mesh.subdivisions,
            mesh.tile_count(),
            land_count,
            show_edges,
            show_pentagons,
        );

        next_frame().await;
    }
}
