use macroquad::prelude::*;
use sim_core::ai::ShipAI;
use sim_core::ship::{Ship, ShipState};
use sim_core::spatial::SPATIAL_CELL_NM;
use sim_core::types::{Position, ShipId};
use sim_core::world::World;
use std::path::Path;

const SEA_COLOR: Color = Color::new(0.08, 0.20, 0.35, 1.0);
const LAND_COLOR: Color = Color::new(0.15, 0.40, 0.12, 1.0);
const COAST_COLOR: Color = Color::new(0.05, 0.15, 0.05, 0.9);
/// Visual sighting range (NM) used to draw faint lines between
/// differing-faction ships near each other. Matches the 17C horizon
/// from a quarterdeck on a clear day; equals one spatial-hash cell.
const SHIP_SIGHT_RANGE_NM: f32 = SPATIAL_CELL_NM;
/// Stroke color/alpha for "I see a foreign sail" sight-lines.
const SIGHT_LINE_COLOR: Color = Color::new(0.85, 0.85, 0.9, 0.18);
const WIND_COLOR: Color = Color::new(0.5, 0.7, 1.0, 0.6);
const PATH_COLOR: Color = Color::new(1.0, 0.9, 0.2, 0.5);
const SELECT_COLOR: Color = Color::new(0.4, 0.9, 1.0, 1.0);

/// Discrete level-of-detail zoom steps in pixels-per-NM. Mouse wheel
/// snaps between adjacent levels; the camera additionally clamps zoom
/// up so that the entire map always fills the viewport.
const LOD_LEVELS: &[f32] = &[0.1, 0.2, 0.4, 0.8, 1.6, 3.2];

struct Camera {
    offset: Vec2,      // world-space center of view
    lod_index: usize,  // index into LOD_LEVELS (requested zoom step)
    fit_zoom: f32,     // computed each frame: zoom at which the whole map fits
    scroll_accum: f32, // accumulator for mouse wheel deltas
}

impl Camera {
    fn new() -> Self {
        Self {
            offset: Vec2::ZERO,
            // 0.4 px/NM is a reasonable default-fit for typical screens.
            lod_index: LOD_LEVELS.iter().position(|&z| z >= 0.4).unwrap_or(2),
            fit_zoom: 0.0,
            scroll_accum: 0.0,
        }
    }

    /// Effective zoom: the requested LoD level, but never below `fit_zoom`
    /// (so we don't waste viewport area when the entire map already fits).
    fn zoom(&self) -> f32 {
        LOD_LEVELS[self.lod_index].max(self.fit_zoom)
    }

    fn world_to_screen(&self, pos: Position) -> Vec2 {
        let sw = screen_width();
        let sh = screen_height();
        let dx = pos.x - self.offset.x;
        let dy = pos.y - self.offset.y;
        let z = self.zoom();
        Vec2::new(sw / 2.0 + dx * z, sh / 2.0 - dy * z)
    }

    /// Inverse of `world_to_screen`: convert a screen-space pixel
    /// (e.g., a mouse click) back to world coordinates.
    fn screen_to_world(&self, screen: Vec2) -> Position {
        let sw = screen_width();
        let sh = screen_height();
        let z = self.zoom();
        Position::new(
            (screen.x - sw / 2.0) / z + self.offset.x,
            -(screen.y - sh / 2.0) / z + self.offset.y,
        )
    }

    /// Update `fit_zoom` for the current viewport given the world extent.
    fn update_fit(&mut self, world: &World) {
        let land = &world.map.land;
        let world_w = land.width as f32 * land.cell_size_nm;
        let world_h = land.height as f32 * land.cell_size_nm;
        let fit_x = screen_width() / world_w;
        let fit_y = screen_height() / world_h;
        self.fit_zoom = fit_x.min(fit_y);
    }

    fn handle_input(&mut self) {
        let z = self.zoom();
        let pan_speed = 10.0 / z;
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

        // Mouse wheel: accumulate and step LoD on threshold so a single
        // scroll click moves exactly one level (avoids zoom-flicker on
        // high-resolution trackpads which deliver many tiny deltas).
        let (_sx, sy) = mouse_wheel();
        self.scroll_accum += sy;
        while self.scroll_accum >= 1.0 {
            if self.lod_index + 1 < LOD_LEVELS.len() {
                self.lod_index += 1;
            }
            self.scroll_accum -= 1.0;
        }
        while self.scroll_accum <= -1.0 {
            if self.lod_index > 0 {
                self.lod_index -= 1;
            }
            self.scroll_accum += 1.0;
        }

        // Keyboard zoom (Z = in, X = out).
        if is_key_pressed(KeyCode::Z) && self.lod_index + 1 < LOD_LEVELS.len() {
            self.lod_index += 1;
        }
        if is_key_pressed(KeyCode::X) && self.lod_index > 0 {
            self.lod_index -= 1;
        }
    }
}

/// Render the land mesh as filled triangles. Falls back to drawing
/// nothing if the mesh wasn't loaded (`data/grids/land_polys.bin`
/// missing).
///
/// Triangles whose AABB is fully off-screen are skipped to avoid
/// shipping every triangle in the dataset to the GPU each frame; for
/// the eastern-seaboard + Caribbean view this typically prunes 80–95%
/// of the ~80k triangles depending on zoom.
fn draw_land(world: &World, camera: &Camera) {
    let mesh = &world.land_mesh;
    if mesh.indices.is_empty() {
        return;
    }
    let sw = screen_width();
    let sh = screen_height();

    // Project all vertices once per frame.
    let projected: Vec<Vec2> = mesh
        .vertices
        .iter()
        .map(|v| camera.world_to_screen(*v))
        .collect();

    let tri_count = mesh.indices.len() / 3;
    for t in 0..tri_count {
        let i0 = mesh.indices[t * 3] as usize;
        let i1 = mesh.indices[t * 3 + 1] as usize;
        let i2 = mesh.indices[t * 3 + 2] as usize;
        let a = projected[i0];
        let b = projected[i1];
        let c = projected[i2];

        // Off-screen reject: all three vertices on the same outside.
        let off = (a.x < 0.0 && b.x < 0.0 && c.x < 0.0)
            || (a.x > sw && b.x > sw && c.x > sw)
            || (a.y < 0.0 && b.y < 0.0 && c.y < 0.0)
            || (a.y > sh && b.y > sh && c.y > sh);
        if off {
            continue;
        }
        draw_triangle(a, b, c, LAND_COLOR);
    }
}

fn draw_coastline(world: &World, camera: &Camera) {
    if world.coastline.lines.is_empty() {
        return;
    }
    // Line thickness in pixels — keep constant at high zoom, slimmer when
    // far out so the coast doesn't smear into a halo.
    let thickness = (camera.zoom() * 0.7).clamp(0.6, 1.6);

    let sw = screen_width();
    let sh = screen_height();
    for line in &world.coastline.lines {
        if line.len() < 2 {
            continue;
        }
        let mut prev = camera.world_to_screen(line[0]);
        for p in &line[1..] {
            let cur = camera.world_to_screen(*p);
            // Cheap viewport reject: skip segments fully off-screen.
            let off = (prev.x < 0.0 && cur.x < 0.0)
                || (prev.x > sw && cur.x > sw)
                || (prev.y < 0.0 && cur.y < 0.0)
                || (prev.y > sh && cur.y > sh);
            if !off {
                draw_line(prev.x, prev.y, cur.x, cur.y, thickness, COAST_COLOR);
            }
            prev = cur;
        }
    }
}

fn draw_wind_arrows(world: &World, camera: &Camera) {
    let wind = &world.weather.wind;
    let month = world.date.month();

    // Draw wind arrows every N grid cells
    let spacing = (20.0 / camera.zoom()).max(3.0) as u32;

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
            let arrow_len = w.speed() * camera.zoom() * 0.8;
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
        if sp.x < -50.0
            || sp.x > screen_width() + 50.0
            || sp.y < -50.0
            || sp.y > screen_height() + 50.0
        {
            continue;
        }

        let (r, g, b) = port.faction.color_rgb();
        let color = Color::from_rgba(r, g, b, 255);

        draw_circle(sp.x, sp.y, 4.0, color);
        draw_circle_lines(sp.x, sp.y, 4.0, 1.0, WHITE);

        // Draw name if zoomed in enough
        if camera.zoom() > 0.15 {
            draw_text(&port.name, sp.x + 6.0, sp.y - 4.0, 14.0, color);
        }
    }
}

fn draw_ships(world: &World, camera: &Camera, selected_ship: Option<ShipId>) {
    // Planned paths first, so ship triangles draw on top.
    for (id, ship) in &world.ships {
        let nav = &ship.nav;
        if world.ship_ais.get(id).is_none() {
            continue;
        }
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

    // Inter-faction sight lines: for each Sailing ship, draw a faint
    // line to any *Sailing* neighbor of a different faction within
    // visual range. Uses the world's dynamic spatial hash; each pair
    // is drawn twice (overlapping exactly), which is visually
    // indistinguishable from one stroke and avoids the cost of an
    // ordered-key dedupe.
    for (id, ship) in &world.ships {
        if ship.state != ShipState::Sailing {
            continue;
        }
        let mine = ship.faction;
        let others = world
            .spatial
            .neighbors(ship.position, SHIP_SIGHT_RANGE_NM, |other_id| {
                other_id != id
                    && world
                        .ships
                        .get(other_id)
                        .map(|o| o.faction != mine)
                        .unwrap_or(false)
            });
        if others.is_empty() {
            continue;
        }
        let from = camera.world_to_screen(ship.position);
        for other_id in others {
            let to = camera.world_to_screen(world.ships[other_id].position);
            draw_line(from.x, from.y, to.x, to.y, 1.0, SIGHT_LINE_COLOR);
        }
    }

    for (id, ship) in &world.ships {
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

        let (r, g, b) = ship.faction.color_rgb();
        let color = Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0);
        draw_triangle(tip, left, right, color);

        if selected_ship == Some(id) {
            draw_circle_lines(sp.x, sp.y, size * 2.2, 2.0, SELECT_COLOR);
        }
    }
}

fn draw_hud(world: &World, camera: &Camera, paused: bool, ticks_per_frame: u32) {
    let date_str = format!(
        "Year {} Day {} {:02}:00",
        world.date.year, world.date.day_of_year, world.date.hour
    );
    let status = if paused { "PAUSED" } else { "RUNNING" };

    // Compact fleet summary: counts by state + economic totals.
    let mut n_sail = 0;
    let mut n_dock = 0;
    let mut n_anchor = 0;
    let mut total_silver = 0.0_f32;
    let mut total_dividends = 0.0_f32;
    let mut total_debt = 0.0_f32;
    let mut total_pnl = 0.0_f32;
    for (_, ship) in &world.ships {
        match ship.state {
            ShipState::Sailing => n_sail += 1,
            ShipState::Docked => n_dock += 1,
            ShipState::Anchored => n_anchor += 1,
            ShipState::Hiring => n_dock += 1,
        }
        total_silver += ship.silver;
        total_dividends += ship.lifetime_dividends;
        total_debt += ship.debt;
        total_pnl += (ship.silver - ship.starting_silver) + ship.lifetime_dividends - ship.debt;
    }

    let line1 = format!(
        "{} | {} | {}h/frame | Zoom {}/{} ({:.2}px/NM)",
        date_str,
        status,
        ticks_per_frame,
        camera.lod_index + 1,
        LOD_LEVELS.len(),
        camera.zoom(),
    );
    let line2 = format!(
        "Fleet: {} ships [{} sail, {} dock, {} anc] | silver ${:.0} | debt ${:.0} | div ${:.0} | P/L ${:+.0}",
        world.ships.len(),
        n_sail,
        n_dock,
        n_anchor,
        total_silver,
        total_debt,
        total_dividends,
        total_pnl,
    );
    draw_text(&line1, 10.0, 20.0, 20.0, WHITE);
    draw_text(&line2, 10.0, 40.0, 16.0, LIGHTGRAY);
    draw_text(
        "Click a port or ship to inspect.  Esc clears selection.",
        10.0,
        screen_height() - 10.0,
        14.0,
        Color::new(0.6, 0.6, 0.6, 1.0),
    );
}

/// Left-side panel showing a selected ship's full state. Mirrors the
/// market panel on the right.
fn draw_ship_panel(world: &World, ship_id: ShipId) {
    let ship = match world.ships.get(ship_id) {
        Some(s) => s,
        None => return,
    };
    let ai = match world.ship_ais.get(ship_id) {
        Some(a) => a,
        None => return,
    };
    let stype = world.ship_types.get(ship.ship_type);
    let stats = &stype.stats;

    let cargo_lines = ship.cargo.iter().count().max(1);
    let panel_w: f32 = 320.0;
    let panel_h: f32 = 28.0 + 18.0 * (10.0 + cargo_lines as f32);
    let x = 10.0;
    let y = 70.0;

    draw_rectangle(x, y, panel_w, panel_h, Color::new(0.0, 0.0, 0.0, 0.7));
    draw_rectangle_lines(x, y, panel_w, panel_h, 2.0, SELECT_COLOR);

    let header = format!("Ship {:?} — {}", ship_id, stype.name);
    draw_text(&header, x + 10.0, y + 22.0, 20.0, YELLOW);

    let state_line = match ship.state {
        ShipState::Sailing => {
            let dest = ai
                .goal
                .dest_port
                .and_then(|p| world.ports.get(p))
                .map(|p| p.name.as_str())
                .unwrap_or("(open sea)");
            let dist_nm = ai
                .goal
                .destination
                .map(|d| ship.position.distance(d))
                .unwrap_or(0.0);
            let eta_h = if ship.speed > 0.1 {
                dist_nm / ship.speed
            } else {
                0.0
            };
            format!("SAILING → {}  ({:.0} NM, ETA {:.0}h)", dest, dist_nm, eta_h)
        }
        ShipState::Docked => {
            let port = ship
                .nav
                .docked_at_port
                .and_then(|p| world.ports.get(p))
                .map(|p| p.name.as_str())
                .unwrap_or("?");
            let act = match ship.dock_action {
                sim_core::ship::DockAction::Idle => "idle",
                sim_core::ship::DockAction::Resupplying => "resupplying",
                sim_core::ship::DockAction::Careening => "careening",
            };
            format!("DOCKED at {} ({})", port, act)
        }
        ShipState::Anchored => "ANCHORED".to_string(),
        ShipState::Hiring => format!("HIRING (crew {}/{})", ship.crew_alive, stats.crew_typical()),
    };

    let prov_pct = (ship.provisions / stats.provision_capacity * 100.0) as i32;
    let pnl = (ship.silver - ship.starting_silver) + ship.lifetime_dividends - ship.debt;

    let lines: [(String, Color); 9] = [
        (state_line, LIGHTGRAY),
        (
            format!(
                "speed {:.1} kt   heading {:>3.0}°",
                ship.speed, ship.heading
            ),
            LIGHTGRAY,
        ),
        (
            format!(
                "provisions {} %  ({:.0}/{:.0} t)",
                prov_pct, ship.provisions, stats.provision_capacity
            ),
            LIGHTGRAY,
        ),
        (format!("hull fouling {:.0}", ship.hull_fouling), LIGHTGRAY),
        (String::new(), LIGHTGRAY),
        (format!("silver     ${:>9.0}", ship.silver), WHITE),
        (
            format!("debt       ${:>9.0}", ship.debt),
            if ship.debt > 0.0 { ORANGE } else { LIGHTGRAY },
        ),
        (
            format!(
                "dividends  ${:>9.0}   (start ${:.0})",
                ship.lifetime_dividends, ship.starting_silver
            ),
            LIGHTGRAY,
        ),
        (
            format!("P/L        ${:>+9.0}", pnl),
            if pnl >= 0.0 { GREEN } else { RED },
        ),
    ];
    for (i, (text, color)) in lines.iter().enumerate() {
        draw_text(text, x + 10.0, y + 44.0 + i as f32 * 18.0, 14.0, *color);
    }

    let cargo_y = y + 44.0 + lines.len() as f32 * 18.0 + 6.0;
    let header = format!(
        "cargo  {:.0}/{:.0} t",
        ship.cargo.total_tons(),
        stats.cargo_capacity_tons
    );
    draw_text(&header, x + 10.0, cargo_y, 14.0, YELLOW);
    let mut row = 0;
    for (gid, tons) in ship.cargo.iter() {
        if tons <= 0.0 {
            continue;
        }
        let name = &world.goods.get(gid).name;
        let line = format!("  {:<16} {:>5.1} t", name, tons);
        draw_text(
            &line,
            x + 10.0,
            cargo_y + 18.0 + row as f32 * 16.0,
            14.0,
            WHITE,
        );
        row += 1;
    }
    if row == 0 {
        draw_text("  (empty)", x + 10.0, cargo_y + 18.0, 14.0, LIGHTGRAY);
    }
}

/// Right-side panel showing a selected port's market state: prices,
/// stockpile, treasury, gateway flag.
fn draw_market_panel(world: &World, port_idx: usize) {
    if port_idx >= world.ports.len() || port_idx >= world.markets.len() {
        return;
    }
    let port = &world.ports[port_idx];
    let market = &world.markets[port_idx];

    let panel_w: f32 = 320.0;
    let panel_h: f32 = 28.0 + 18.0 * (world.goods.len() as f32 + 3.0);
    let x = screen_width() - panel_w - 10.0;
    let y = 10.0;

    draw_rectangle(x, y, panel_w, panel_h, Color::new(0.0, 0.0, 0.0, 0.7));
    draw_rectangle_lines(x, y, panel_w, panel_h, 2.0, WHITE);

    draw_text(&port.name, x + 10.0, y + 22.0, 20.0, YELLOW);
    draw_text(
        &format!("Treasury: ${:.0}", market.silver),
        x + 10.0,
        y + 42.0,
        16.0,
        LIGHTGRAY,
    );
    draw_text(
        "Good          stk(t)   buy   sell",
        x + 10.0,
        y + 62.0,
        14.0,
        Color::new(0.7, 0.7, 0.7, 1.0),
    );

    for (i, good) in world.goods.iter().enumerate() {
        let stk = market.stockpile.get(good.id);
        let buy = market.buy_price(good.id, &world.goods);
        let sell = market.sell_price(good.id, &world.goods);
        let line = format!("{:<14}{:>7.0} {:>5.1} {:>5.1}", good.name, stk, buy, sell);
        draw_text(&line, x + 10.0, y + 80.0 + i as f32 * 18.0, 14.0, WHITE);
    }
}

/// Find the ship closest to the given world point within a screen-radius
/// hit zone (converted to NM at the current zoom). Returns None if no
/// ship is within range.
fn pick_ship_at(world: &World, world_pos: Position, zoom_px_per_nm: f32) -> Option<ShipId> {
    // ~12 pixels of click slop, regardless of zoom.
    let click_r_nm = 12.0 / zoom_px_per_nm.max(0.05);
    let mut best: Option<(ShipId, f32)> = None;
    for (id, ship) in &world.ships {
        let d = world_pos.distance(ship.position);
        if d <= click_r_nm && best.is_none_or(|(_, db)| d < db) {
            best = Some((id, d));
        }
    }
    best.map(|(id, _)| id)
}

/// Find the port whose harbor radius contains the given world point,
/// preferring the closer one if multiple match.
fn pick_port_at(world: &World, world_pos: Position) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, port) in world.ports.iter().enumerate() {
        let d = world_pos.distance(port.position);
        // Use a generous click radius so small ports are still hittable.
        let click_r = port.harbor_radius_nm.max(15.0);
        if d <= click_r && best.is_none_or(|(_, db)| d < db) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

#[macroquad::main("Pirate Sim - Phase 2")]
async fn main() {
    let mut world = World::load(Path::new("data/"));
    spawn_demo_ships(&mut world);

    let mut camera = Camera::new();
    let mut paused = false;
    let mut ticks_per_frame: u32 = 1;
    let mut selected_port: Option<usize> = None;
    let mut selected_ship: Option<ShipId> = None;

    loop {
        // Input
        camera.update_fit(&world);
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
            spawn_demo_ships(&mut world);
            ticks_per_frame = 1;
            selected_port = None;
            selected_ship = None;
        }

        // Click selects (or deselects) a ship or port. Ships take priority
        // since their hit zone is small; an empty-water click keeps the
        // current selection so the inspector doesn't flicker.
        if is_mouse_button_pressed(MouseButton::Left) {
            let (mx, my) = mouse_position();
            let world_pos = camera.screen_to_world(Vec2::new(mx, my));
            if let Some(id) = pick_ship_at(&world, world_pos, camera.zoom()) {
                selected_ship = if selected_ship == Some(id) {
                    None
                } else {
                    Some(id)
                };
                selected_port = None;
            } else if let Some(i) = pick_port_at(&world, world_pos) {
                selected_port = if selected_port == Some(i) {
                    None
                } else {
                    Some(i)
                };
                selected_ship = None;
            }
        }
        if is_key_pressed(KeyCode::Escape) {
            selected_port = None;
            selected_ship = None;
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
        draw_coastline(&world, &camera);
        draw_wind_arrows(&world, &camera);
        draw_ports(&world, &camera);
        draw_ships(&world, &camera, selected_ship);
        draw_hud(&world, &camera, paused, ticks_per_frame);
        if let Some(idx) = selected_port {
            draw_market_panel(&world, idx);
        }
        if let Some(id) = selected_ship {
            draw_ship_panel(&world, id);
        }

        next_frame().await;
    }
}

fn spawn_demo_ships(world: &mut World) {
    // A starter fleet spread across Caribbean, North America, and
    // Europe — gives the trader AI all three legs of the triangular
    // trade visible from tick zero.
    let starts: &[(&str, u64)] = &[
        ("Bridgetown", 7),
        ("Port Royal", 13),
        ("Boston", 21),
        ("Charleston", 33),
        ("Cartagena", 41),
        ("Havana", 53),
        ("Fort-Royal", 67),
        ("London", 79),
        ("Amsterdam", 89),
        ("Nantes", 97),
    ];
    for (port_name, seed) in starts {
        if let Some(idx) = world.ports.iter().position(|p| p.name == *port_name) {
            let port_pos = world.ports[idx].position;
            let mut ship = Ship::new(port_pos, ShipState::Docked);
            let ai = ShipAI::with_seed(*seed);
            ship.nav.docked_at_port = Some(idx);
            world.add_ship(ship, ai);
        }
    }

    // Step 6: pirate sloops at the major Caribbean havens — gives the
    // viz at least one visibly hostile encounter during a normal
    // session. See `bench_trade.rs` for the same set + rationale.
    for (name, seed) in &[
        ("Tortuga", 1009u64),
        ("Petit-Goâve", 1031),
        ("Nassau", 1051),
    ] {
        let _ = world.spawn_pirate_sloop_at(name, *seed);
    }
}
