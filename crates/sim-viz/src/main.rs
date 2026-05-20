use macroquad::prelude::*;
use sim_core::ai::ShipAI;
use sim_core::ship::{Ship, ShipState};
use sim_core::types::Position;
use sim_core::world::World;
use std::path::Path;

const SEA_COLOR: Color = Color::new(0.08, 0.20, 0.35, 1.0);
const LAND_COLOR: Color = Color::new(0.15, 0.40, 0.12, 1.0);
const SHIP_COLOR: Color = Color::new(1.0, 0.9, 0.2, 1.0);
const WIND_COLOR: Color = Color::new(0.5, 0.7, 1.0, 0.6);
const PATH_COLOR: Color = Color::new(1.0, 0.9, 0.2, 0.5);

struct Camera {
    offset: Vec2,   // world-space center of view
    zoom: f32,      // pixels per nautical mile
}

impl Camera {
    fn new() -> Self {
        Self {
            offset: Vec2::ZERO,
            zoom: 0.4,
        }
    }

    fn world_to_screen(&self, pos: Position) -> Vec2 {
        let sw = screen_width();
        let sh = screen_height();
        let dx = pos.x - self.offset.x;
        let dy = pos.y - self.offset.y;
        Vec2::new(sw / 2.0 + dx * self.zoom, sh / 2.0 - dy * self.zoom)
    }

    fn handle_input(&mut self) {
        let pan_speed = 10.0 / self.zoom;
        if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
            self.offset.y += pan_speed;
        }
        if is_key_down(KeyCode::S) || is_key_down(KeyCode::Down) {
            self.offset.y -= pan_speed;
        }
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
            self.offset.x -= pan_speed;
        }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
            self.offset.x += pan_speed;
        }

        let (_scroll_x, scroll_y) = mouse_wheel();
        if scroll_y != 0.0 {
            self.zoom *= 1.0 + scroll_y * 0.1;
            self.zoom = self.zoom.clamp(0.05, 5.0);
        }
    }
}

/// Build a single texture of the land mask. Cells are downsampled by
/// `STRIDE` (block-any-land) to keep GPU memory + upload time reasonable
/// for a 6300×3900 source: at STRIDE=2 the texture is ~3150×1950 ≈ 24 MB,
/// which uploads near-instantly and renders as a single textured quad. The
/// coastline visually still reads as ~2 NM resolution which is more than
/// enough for the strategic-scale viz.
fn build_land_texture(world: &World) -> Texture2D {
    const STRIDE: u32 = 2;
    let land = &world.map.land;
    let (sw, sh) = (land.width, land.height);
    let tw = (sw + STRIDE - 1) / STRIDE;
    let th = (sh + STRIDE - 1) / STRIDE;
    let mut bytes = vec![0u8; (tw * th * 4) as usize];
    for r in 0..th {
        for c in 0..tw {
            // Block-any: any source cell in the STRIDE×STRIDE block is land.
            let r0 = r * STRIDE;
            let c0 = c * STRIDE;
            let r1 = (r0 + STRIDE).min(sh);
            let c1 = (c0 + STRIDE).min(sw);
            let mut is_land = false;
            'outer: for rr in r0..r1 {
                let base = (rr * sw) as usize;
                for cc in c0..c1 {
                    if land.data[base + cc as usize] == 255 {
                        is_land = true;
                        break 'outer;
                    }
                }
            }
            if is_land {
                let idx = ((r * tw + c) * 4) as usize;
                bytes[idx] = (LAND_COLOR.r * 255.0) as u8;
                bytes[idx + 1] = (LAND_COLOR.g * 255.0) as u8;
                bytes[idx + 2] = (LAND_COLOR.b * 255.0) as u8;
                bytes[idx + 3] = 255;
            }
        }
    }
    let img = Image { bytes, width: tw as u16, height: th as u16 };
    let tex = Texture2D::from_image(&img);
    tex.set_filter(FilterMode::Nearest);
    tex
}

fn draw_land(world: &World, camera: &Camera, tex: &Texture2D) {
    let land = &world.map.land;
    // Top-left corner of the texture in world space.
    let world_x = land.origin.x;
    let world_y = land.origin.y;
    let top_left = camera.world_to_screen(Position::new(world_x, world_y));
    let pixel_size = land.cell_size_nm * camera.zoom;
    let dest_w = land.width as f32 * pixel_size;
    let dest_h = land.height as f32 * pixel_size;
    draw_texture_ex(
        tex,
        top_left.x,
        top_left.y,
        WHITE,
        DrawTextureParams {
            dest_size: Some(Vec2::new(dest_w, dest_h)),
            ..Default::default()
        },
    );
}

fn draw_wind_arrows(world: &World, camera: &Camera) {
    let wind = &world.weather.wind;
    let month = world.date.month();

    // Draw wind arrows every N grid cells
    let spacing = (20.0 / camera.zoom).max(3.0) as u32;

    for row in (0..wind.height).step_by(spacing as usize) {
        for col in (0..wind.width).step_by(spacing as usize) {
            let world_x = wind.origin.x + col as f32 * wind.cell_size_nm;
            let world_y = wind.origin.y - row as f32 * wind.cell_size_nm;
            let pos = Position::new(world_x, world_y);

            let w = wind.wind_at(pos, month);
            let screen_pos = camera.world_to_screen(pos);

            // Only draw if on screen
            if screen_pos.x < -20.0
                || screen_pos.x > screen_width() + 20.0
                || screen_pos.y < -20.0
                || screen_pos.y > screen_height() + 20.0
            {
                continue;
            }

            // Arrow length proportional to wind speed
            let arrow_len = w.speed() * camera.zoom * 0.8;
            if arrow_len < 2.0 {
                continue;
            }

            // Wind vector direction (TO direction)
            let dir_rad = w.direction_to().to_radians();
            let end = Vec2::new(
                screen_pos.x + arrow_len * dir_rad.sin(),
                screen_pos.y - arrow_len * dir_rad.cos(),
            );

            draw_line(screen_pos.x, screen_pos.y, end.x, end.y, 1.0, WIND_COLOR);
        }
    }
}

fn draw_ports(world: &World, camera: &Camera) {
    for port in &world.ports {
        let sp = camera.world_to_screen(port.position);

        // Only draw if on screen
        if sp.x < -50.0 || sp.x > screen_width() + 50.0
            || sp.y < -50.0 || sp.y > screen_height() + 50.0
        {
            continue;
        }

        let (r, g, b) = port.faction.color_rgb();
        let color = Color::from_rgba(r, g, b, 255);

        draw_circle(sp.x, sp.y, 4.0, color);
        draw_circle_lines(sp.x, sp.y, 4.0, 1.0, WHITE);

        // Draw name if zoomed in enough
        if camera.zoom > 0.15 {
            draw_text(port.name, sp.x + 6.0, sp.y - 4.0, 14.0, color);
        }
    }
}

fn draw_ships(world: &World, camera: &Camera) {
    // Planned paths first, so ship triangles draw on top.
    for (i, ship) in world.ships.iter().enumerate() {
        let nav = &world.ship_ais[i].nav;
        if nav.waypoints.is_empty() {
            continue;
        }
        let mut prev = camera.world_to_screen(ship.position);
        for wp in nav.waypoints.iter() {
            let p = camera.world_to_screen(*wp);
            draw_line(prev.x, prev.y, p.x, p.y, 1.5, PATH_COLOR);
            draw_circle(p.x, p.y, 2.0, PATH_COLOR);
            prev = p;
        }
    }

    for ship in &world.ships {
        let sp = camera.world_to_screen(ship.position);
        let size = 6.0;
        let rad = ship.heading.to_radians();

        // Triangle pointing in heading direction
        let tip = Vec2::new(sp.x + size * rad.sin(), sp.y - size * rad.cos());
        let left = Vec2::new(
            sp.x + size * 0.5 * (rad + 2.5).sin(),
            sp.y - size * 0.5 * (rad + 2.5).cos(),
        );
        let right = Vec2::new(
            sp.x + size * 0.5 * (rad - 2.5).sin(),
            sp.y - size * 0.5 * (rad - 2.5).cos(),
        );

        draw_triangle(tip, left, right, SHIP_COLOR);
    }
}

fn draw_hud(world: &World, paused: bool, ticks_per_frame: u32) {
    let date_str = format!(
        "Year {} Day {} {:02}:00",
        world.date.year, world.date.day_of_year, world.date.hour
    );
    let status = if paused { "PAUSED" } else { "RUNNING" };
    let info = format!(
        "{} | Speed: {}h/frame | {} | Ships: {}",
        date_str,
        ticks_per_frame,
        status,
        world.ships.len()
    );
    draw_text(&info, 10.0, 20.0, 20.0, WHITE);

    // Ship info
    for (i, ship) in world.ships.iter().enumerate() {
        let dist = world.ship_ais[i]
            .nav
            .destination
            .map(|d| ship.position.distance(d))
            .unwrap_or(0.0);
        let state_str = match ship.state {
            ShipState::Sailing => "SAILING".to_string(),
            ShipState::Docked => {
                match world.ship_ais[i].dock_action {
                    sim_core::ai::DockAction::Idle => "DOCKED (idle)".to_string(),
                    sim_core::ai::DockAction::Resupplying => "DOCKED (resupply)".to_string(),
                    sim_core::ai::DockAction::Careening => "DOCKED (careen)".to_string(),
                }
            }
            ShipState::Anchored => "ANCHORED".to_string(),
        };
        let stats = sim_core::ship::ShipStats::sloop();
        let prov_pct = (ship.provisions / stats.provision_capacity * 100.0) as i32;
        let ship_info = format!(
            "Ship {}: {} | spd={:.1}kt dist={:.0}nm | food={}% foul={:.0}",
            i, state_str, ship.speed, dist, prov_pct, ship.hull_fouling
        );
        draw_text(&ship_info, 10.0, 40.0 + i as f32 * 18.0, 16.0, LIGHTGRAY);
    }
}

#[macroquad::main("Pirate Sim - Phase 1")]
async fn main() {
    let mut world = World::load(Path::new("data/"));
    spawn_demo_ship(&mut world);

    let land_texture = build_land_texture(&world);
    let mut camera = Camera::new();
    let mut paused = false;
    let mut ticks_per_frame: u32 = 1;

    loop {
        // Input
        camera.handle_input();
        if is_key_pressed(KeyCode::Space) {
            paused = !paused;
        }
        if is_key_pressed(KeyCode::Equal) {
            ticks_per_frame = (ticks_per_frame * 2).min(96);
        }
        if is_key_pressed(KeyCode::Minus) {
            ticks_per_frame = (ticks_per_frame / 2).max(1);
        }
        if is_key_pressed(KeyCode::R) {
            world = World::load(Path::new("data/"));
            spawn_demo_ship(&mut world);
            ticks_per_frame = 1;
        }

        // Tick
        if !paused {
            for _ in 0..ticks_per_frame {
                world.tick();
            }
        }

        // Render
        clear_background(SEA_COLOR);
        draw_land(&world, &camera, &land_texture);
        draw_wind_arrows(&world, &camera);
        draw_ports(&world, &camera);
        draw_ships(&world, &camera);
        draw_hud(&world, paused, ticks_per_frame);

        next_frame().await;
    }
}

fn spawn_demo_ship(world: &mut World) {
    let barbados_pos = world.ports.iter().find(|p| p.name == "Bridgetown").unwrap().position;
    let ship = Ship::new(barbados_pos, ShipState::Docked);
    let ai = ShipAI::with_seed(7); // will choose a random destination on first tick
    world.add_ship(ship, ai);
}
