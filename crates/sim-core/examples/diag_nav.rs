use sim_core::ai::ShipAI;
use sim_core::pathfind::{find_path, PathfindContext};
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
        ("Cartagena", "New York"),
        ("Port Royal", "Cartagena"),
        ("Trinidad", "Bridgetown"),
    ];
    for (a, b) in routes {
        let pa = world.ports.iter().find(|p| p.name == *a).unwrap().position;
        let pb = world.ports.iter().find(|p| p.name == *b).unwrap().position;
        match find_path(&pf, pa, pb) {
            Some(path) => {
                println!("{} -> {}: {} waypoints, last={:?}, port={:?}", a, b, path.len(), path.last(), pb);
            }
            None => println!("{} -> {}: NO PATH FOUND", a, b),
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
