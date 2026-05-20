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

fn draw_land(world: &World, camera: &Camera) {
    let land = &world.map.land;
    // Only draw visible cells for performance
    let sw = screen_width();
    let sh = screen_height();

    // Determine visible world bounds
    let half_w = sw / (2.0 * camera.zoom);
    let half_h = sh / (2.0 * camera.zoom);
    let min_x = camera.offset.x - half_w;
    let max_x = camera.offset.x + half_w;
    let min_y = camera.offset.y - half_h;
    let max_y = camera.offset.y + half_h;

    // Convert to grid cells
    let col_start = ((min_x - land.origin.x) / land.cell_size_nm).floor().max(0.0) as u32;
    let col_end = ((max_x - land.origin.x) / land.cell_size_nm).ceil().min(land.width as f32) as u32;
    let row_start = ((land.origin.y - max_y) / land.cell_size_nm).floor().max(0.0) as u32;
    let row_end = ((land.origin.y - min_y) / land.cell_size_nm).ceil().min(land.height as f32) as u32;

    let cell_screen_size = land.cell_size_nm * camera.zoom;

    // Skip if cells are too small to see
    if cell_screen_size < 1.0 {
        return;
    }

    for row in row_start..row_end {
        for col in col_start..col_end {
            let idx = (row * land.width + col) as usize;
            if land.data[idx] == 255 {
                // This cell is land
                let world_x = land.origin.x + col as f32 * land.cell_size_nm;
                let world_y = land.origin.y - row as f32 * land.cell_size_nm;
                let screen_pos = camera.world_to_screen(Position::new(world_x, world_y));
                draw_rectangle(
                    screen_pos.x,
                    screen_pos.y - cell_screen_size,
                    cell_screen_size,
                    cell_screen_size,
                    LAND_COLOR,
                );
            }
        }
    }
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
        draw_land(&world, &camera);
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
