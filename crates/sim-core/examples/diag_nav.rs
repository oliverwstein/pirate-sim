use sim_core::ai::ShipAI;
use sim_core::pathfind::{find_path_to_harbor, PathfindContext};
use sim_core::ship::{Ship, ShipState, ShipStats};
use sim_core::world::World;
use std::path::Path;

fn main() {
    let world = World::load(Path::new("data/"));
    let stats = ShipStats::sloop();
    let pf = PathfindContext::new(&world.map.land, &world.weather.wind, &stats, 0);

    // Try a few representative routes.
    let routes: &[(&str, &str)] = &[
        ("Bridgetown", "Tobago"),
        ("Bridgetown", "Port Royal"),
        ("Havana", "Nassau"),
        ("Petit-Goâve", "Cartagena"),
        ("Petit-Goâve", "Santo Domingo"),
        ("Petit-Goâve", "San Juan"),
        ("Petit-Goâve", "Bridgetown"),
        ("Port Royal", "Bridgetown"),
        ("Cartagena", "New York"),
        ("Cartagena", "Philadelphia"),
        ("Port Royal", "Cartagena"),
        ("Trinidad", "Bridgetown"),
    ];
    for (a, b) in routes {
        let pa = world.ports.iter().find(|p| p.name == *a).unwrap().position;
        let b_idx = world.ports.iter().position(|p| p.name == *b).unwrap();
        let harbor = match world.harbors.for_port(b_idx) {
            Some(h) => h,
            None => {
                println!("{} -> {}: NO HARBOR ZONE", a, b);
                continue;
            }
        };
        match find_path_to_harbor(&pf, pa, harbor) {
            Some(path) => {
                println!("{} -> {}: {} waypoints, last={:?}, anchor={:?}, zone_cells={}",
                    a, b, path.len(), path.last(), harbor.anchor, harbor.cells.len());
            }
            None => println!("{} -> {}: NO PATH FOUND  (anchor={:?}, zone_cells={})",
                a, b, harbor.anchor, harbor.cells.len()),
        }
    }

    // Routing test from a non-port runtime position (where a ship might
    // arrive inside Petit-Goâve's harbor zone): mimics post-arrival replan.
    {
        use sim_core::types::Position;
        let from = Position::new(-182.8, 134.5);
        let bri_idx = world.ports.iter().position(|p| p.name == "Bridgetown").unwrap();
        let harbor = world.harbors.for_port(bri_idx).unwrap();
        match find_path_to_harbor(&pf, from, harbor) {
            Some(path) => println!("(harbor-zone) {:?} -> Bridgetown: {} waypoints, first={:?}, last={:?}",
                from, path.len(), path.first(), path.last()),
            None => println!("(harbor-zone) {:?} -> Bridgetown: NO PATH FOUND", from),
        }
    }

    // Now simulate one ship over 30 days, printing position at each midnight.
    let mut world = World::load(Path::new("data/"));
    let bridgetown = world.ports.iter().find(|p| p.name == "Bridgetown").unwrap().position;
    let ship = Ship::new(bridgetown, ShipState::Docked);
    let ai = ShipAI::with_seed(7);
    world.add_ship(ship, ai);

    let mut prev = world.ships[0].position;
    let mut consecutive_stuck = 0;
    for t in 0..(24 * 30) {
        world.tick();
        let s = &world.ships[0];
        if s.position.distance(prev) < 0.05 && s.state == ShipState::Sailing {
            consecutive_stuck += 1;
        } else {
            consecutive_stuck = 0;
        }
        if t % 24 == 0 || consecutive_stuck == 6 {
            let nav = &world.ship_ais[0].nav;
            println!("h{:4} state={:?} pos=({:6.1},{:6.1}) hd={:5.1} sp={:4.1} dest={:?} wps={}",
                t, s.state, s.position.x, s.position.y, s.heading, s.speed,
                nav.destination.map(|p| (p.x as i32, p.y as i32)), nav.waypoints.len());
        }
        prev = s.position;
    }
}
