//! Run a calibration demo over a configurable horizon: spawn a small
//! trader fleet, tick the simulation hour-by-hour, and report each
//! ship's profit-and-loss together with a sample of port stockpiles
//! and prices.
//!
//! Used during Phase 2/3 calibration to tune prosperity, monthly
//! outputs, pricing constants, sailor-pool dynamics, and morale.
//! The 60-day run is the quick smoke; 365 / 730-day runs surface
//! slow-burn pathologies (pool collapse, runaway wages, equilibrium
//! drift). Look for: most ships in the black, 1–2 in the red (some
//! volatility), prices that oscillate but don't explode, sailor pools
//! that stay within a 2× band of seed values.
//!
//! Usage:
//!   cargo run --release --example bench_trade            # 60-day default
//!   cargo run --release --example bench_trade -- 365     # 1-year sweep
//!   cargo run --release --example bench_trade -- 730     # 2-year sweep

use sim_core::equilibrium::{self, EquilibriumScenario, FreightCostModel, PortSpec};
use sim_core::goods::{ids, GoodId};
use sim_core::market::archetype_for;
use sim_core::ship::ShipStats;
use sim_core::types::ShipId;
use sim_core::world::World;
use std::collections::BTreeMap;
use std::path::Path;

const DEFAULT_SIM_DAYS: u32 = 60;

/// Snapshot total stockpile (summed across all ports) and total
/// in-transit cargo (summed across all ships) for every good. Used
/// by the system-wide accounting table.
fn snapshot_system(
    world: &World,
    stockpile_out: &mut Vec<Vec<(GoodId, f32)>>,
    in_transit_out: &mut Vec<Vec<(GoodId, f32)>>,
) {
    let mut stk: BTreeMap<u8, f32> = BTreeMap::new();
    for m in &world.markets {
        for (gid, tons) in m.stockpile.iter() {
            *stk.entry(gid.0).or_insert(0.0) += tons;
        }
    }
    let mut tr: BTreeMap<u8, f32> = BTreeMap::new();
    for (_, s) in &world.ships {
        for (gid, tons) in s.cargo.iter() {
            *tr.entry(gid.0).or_insert(0.0) += tons;
        }
    }
    stockpile_out.push(stk.into_iter().map(|(k, v)| (GoodId(k), v)).collect());
    in_transit_out.push(tr.into_iter().map(|(k, v)| (GoodId(k), v)).collect());
}

/// Compute deterministic per-month production and consumption totals
/// from each port's recipe and prosperity. (We don't measure actual
/// post-`tick_month` deltas because debt-settlement and trade muddy
/// the picture; the recipe rates are the cleanest signal of the
/// system's underlying flow capacity.)
fn record_recipe_flow(
    world: &World,
    prod_out: &mut Vec<Vec<(GoodId, f32)>>,
    cons_out: &mut Vec<Vec<(GoodId, f32)>>,
) {
    let mut prod: BTreeMap<u8, f32> = BTreeMap::new();
    let mut cons: BTreeMap<u8, f32> = BTreeMap::new();
    for m in &world.markets {
        let p = m.recipe.prosperity.max(0.0);
        for (gid, tons) in &m.recipe.monthly_outputs {
            *prod.entry(gid.0).or_insert(0.0) += tons * p;
        }
        for (gid, tons) in &m.recipe.monthly_inputs {
            *cons.entry(gid.0).or_insert(0.0) += tons * p;
        }
    }
    prod_out.push(prod.into_iter().map(|(k, v)| (GoodId(k), v)).collect());
    cons_out.push(cons.into_iter().map(|(k, v)| (GoodId(k), v)).collect());
}

fn main() {
    let sim_days: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SIM_DAYS);
    let sim_hours: u32 = sim_days * 24;

    let mut world = World::load(Path::new("data/"));

    // Step 10: seed a historically-scaled starter fleet (~480 ships
    // across all 38 ports — see `planning/research/atlantic-fleet-
    // numbers-1650-1720.md`). Replaces the hand-picked 10-merchant +
    // 3-pirate starter from Steps 6–9; that fleet was 1–2 orders of
    // magnitude under the historical baseline of ~400–800 active
    // hulls in the Caribbean basin c. 1680.
    let ship_ids: Vec<ShipId> = world.seed_historical_fleet(0xCAFE_1680);
    let mut ship_ids = ship_ids;
    let mut origin_names: Vec<String> = ship_ids
        .iter()
        .map(|&id| {
            let port_idx = world.ships[id].owner_port.unwrap_or(0);
            world.ports[port_idx].name.clone()
        })
        .collect();
    let n_seeded_pirates = ship_ids
        .iter()
        .filter(|&&id| world.ships[id].policy == sim_core::ship::ShipPolicy::Pirate)
        .count();

    let n_ships = world.ships.len();
    println!("Calibration run: {} days, {} ships", sim_days, n_ships);
    println!();

    // Sample a few ports each week so we can see prices oscillate.
    let sample_port_names: Vec<&str> = vec!["Bridgetown", "Boston", "Cartagena", "Havana"];
    let sample_idxs: Vec<usize> = sample_port_names
        .iter()
        .filter_map(|n| world.ports.iter().position(|p| p.name == *n))
        .collect();

    println!(
        "{:>5}  {:<28}  {:>10}  {:>10}",
        "day", "port", "sugar buy", "sugar sell"
    );
    for idx in &sample_idxs {
        let m = &world.markets[*idx];
        println!(
            "{:>5}  {:<28}  {:>10.2}  {:>10.2}",
            0,
            world.ports[*idx].name,
            m.buy_price(ids::SUGAR, &world.goods),
            m.sell_price(ids::SUGAR, &world.goods),
        );
    }

    // Per-month, per-good system-wide accounting. Captured at each
    // month boundary so we can see real production / consumption /
    // stockpile flow across the simulation.
    //
    // production_per_month[m][good] = Σ over ports of recipe.outputs * prosperity
    // consumed_per_month  same shape, from recipe.inputs.
    // stockpile_per_month[m][good]  = Σ over ports of stockpile.get(good) at month end
    // in_transit_per_month[m][good] = Σ over ships of ship.cargo.get(good) at month end
    let mut last_observed_month = world.date.month();
    let mut production_per_month: Vec<Vec<(GoodId, f32)>> = Vec::new();
    let mut consumption_per_month: Vec<Vec<(GoodId, f32)>> = Vec::new();
    let mut stockpile_per_month: Vec<Vec<(GoodId, f32)>> = Vec::new();
    let mut in_transit_per_month: Vec<Vec<(GoodId, f32)>> = Vec::new();
    // Take an initial snapshot at month 0 (before any production).
    snapshot_system(&world, &mut stockpile_per_month, &mut in_transit_per_month);
    record_recipe_flow(
        &world,
        &mut production_per_month,
        &mut consumption_per_month,
    );

    // Simulate.
    for h in 1..=sim_hours {
        world.tick();
        // Track origin names for any newly-built ships so the report
        // can label them. Scan for ShipIds we haven't seen yet (SlotMap
        // iteration order is not specified, but newly-inserted keys
        // simply aren't in our `ship_ids` Vec yet).
        if world.ships.len() > ship_ids.len() {
            let known: std::collections::HashSet<ShipId> = ship_ids.iter().copied().collect();
            for (id, ship) in &world.ships {
                if !known.contains(&id) {
                    let owner_name = ship
                        .owner_port
                        .and_then(|idx| world.ports.get(idx).map(|p| p.name.clone()))
                        .unwrap_or_else(|| "?".to_string());
                    origin_names.push(owner_name);
                    ship_ids.push(id);
                }
            }
        }
        // Detect month transition and snapshot at end of month.
        if world.date.month() != last_observed_month {
            snapshot_system(&world, &mut stockpile_per_month, &mut in_transit_per_month);
            record_recipe_flow(
                &world,
                &mut production_per_month,
                &mut consumption_per_month,
            );
            last_observed_month = world.date.month();
        }
        if h % (24 * 7) == 0 {
            let day = h / 24;
            for idx in &sample_idxs {
                let m = &world.markets[*idx];
                println!(
                    "{:>5}  {:<28}  {:>10.2}  {:>10.2}",
                    day,
                    world.ports[*idx].name,
                    m.buy_price(ids::SUGAR, &world.goods),
                    m.sell_price(ids::SUGAR, &world.goods),
                );
            }
        }
    }

    println!();
    println!("Fleet aggregate after {} days:", sim_days);
    let mut total_pl = 0.0f32;
    let mut total_debt = 0.0f32;
    let mut bankrupt = 0;
    // Aggregate by (faction, ship-type) → (count, total_pl, total_debt,
    // total_hull_pct, total_rig_pct, sailing/docked/anchored/hiring/sunk).
    use sim_core::port::Faction;
    use sim_core::ship::ShipPolicy;
    #[derive(Default)]
    struct Bucket {
        n: u32,
        pl: f32,
        debt: f32,
        hull_pct: f32,
        rig_pct: f32,
        n_pirate: u32,
        n_bankrupt: u32,
    }
    let mut buckets: BTreeMap<(String, String), Bucket> = BTreeMap::new();
    let mut top: Vec<(i64, ShipId)> = Vec::new();
    for &id in &ship_ids {
        let Some(ship) = world.ships.get(id) else {
            continue;
        };
        let pl = (ship.silver - ship.starting_silver) + ship.lifetime_dividends - ship.debt;
        total_pl += pl;
        total_debt += ship.debt;
        let is_bankrupt = ship.silver < 50.0 && ship.lifetime_dividends < 1.0;
        if is_bankrupt {
            bankrupt += 1;
        }
        let stats = &world.ship_types.get(ship.ship_type).stats;
        let hull_pct = if stats.hull_integrity_max > 0.0 {
            100.0 * ship.hull_integrity / stats.hull_integrity_max
        } else {
            100.0
        };
        let rig_pct = if stats.rigging_integrity_max > 0.0 {
            100.0 * ship.rigging_integrity / stats.rigging_integrity_max
        } else {
            100.0
        };
        let faction_key = match ship.faction {
            Faction::Spain => "Spain",
            Faction::England => "England",
            Faction::France => "France",
            Faction::Netherlands => "Netherlands",
            Faction::Free => "Free",
        }
        .to_string();
        let type_key = world.ship_types.get(ship.ship_type).name.clone();
        let b = buckets.entry((faction_key, type_key)).or_default();
        b.n += 1;
        b.pl += pl;
        b.debt += ship.debt;
        b.hull_pct += hull_pct;
        b.rig_pct += rig_pct;
        if ship.policy == ShipPolicy::Pirate {
            b.n_pirate += 1;
        }
        if is_bankrupt {
            b.n_bankrupt += 1;
        }
        top.push((pl as i64, id));
    }
    println!(
        "{:<12}  {:<10}  {:>5}  {:>+12}  {:>10}  {:>6}  {:>6}  {:>6}  {:>6}",
        "faction", "type", "n", "P/L total", "debt tot", "hull%", "rig%", "pirate", "broke"
    );
    for ((fac, ty), b) in &buckets {
        if b.n == 0 {
            continue;
        }
        println!(
            "{:<12}  {:<10}  {:>5}  {:>+12.0}  {:>10.0}  {:>6.0}  {:>6.0}  {:>6}  {:>6}",
            fac,
            ty,
            b.n,
            b.pl,
            b.debt,
            b.hull_pct / b.n as f32,
            b.rig_pct / b.n as f32,
            b.n_pirate,
            b.n_bankrupt,
        );
    }
    // Outliers: top 5 and bottom 5 by P/L.
    top.sort_by(|a, b| b.0.cmp(&a.0));
    println!();
    println!("Top 5 earners:");
    for (pl, id) in top.iter().take(5) {
        let s = &world.ships[*id];
        let port = s
            .owner_port
            .and_then(|i| world.ports.get(i).map(|p| p.name.as_str()))
            .unwrap_or("?");
        println!(
            "  {:>+10}  {:<10} from {}",
            pl,
            world.ship_types.get(s.ship_type).name,
            port
        );
    }
    println!("Bottom 5:");
    for (pl, id) in top.iter().rev().take(5) {
        let s = &world.ships[*id];
        let port = s
            .owner_port
            .and_then(|i| world.ports.get(i).map(|p| p.name.as_str()))
            .unwrap_or("?");
        println!(
            "  {:>+10}  {:<10} from {}",
            pl,
            world.ship_types.get(s.ship_type).name,
            port
        );
    }

    println!();
    println!(
        "Fleet total P/L: {:+.0} pesos   (outstanding debt: {:.0})",
        total_pl, total_debt
    );
    println!("Bankrupt ships:  {}/{}", bankrupt, world.ships.len());
    println!(
        "Ships built by shipyards: {}  (last_month_avg_profit = {:+.0})",
        world.ships_built, world.last_month_avg_profit
    );
    // Step 8 / Step 9 diagnostics: counts of active pirates and of
    // ships that were spawned/built over the run but no longer exist
    // (sunk by broadside or burned after boarding).
    {
        let n_pirate = world
            .ships
            .iter()
            .filter(|(_, s)| s.policy == ShipPolicy::Pirate)
            .count();
        let n_navy = world
            .ships
            .iter()
            .filter(|(_, s)| s.policy != ShipPolicy::Pirate && s.policy != ShipPolicy::Merchant)
            .count();
        let n_mutinied = world.mutinies_total as usize;
        // Step 11.a: "captured" now means prize-flips (rare path).
        // The old derivation (n_pirate − seeded − mutinied) was a
        // proxy that worked when every prize became a pirate; with
        // the new outcome split, read it straight from the counter.
        let n_captured = world.prizes_taken as usize;
        let n_known = ship_ids.len();
        let n_alive = world.ships.len();
        let n_lost = n_known.saturating_sub(n_alive);
        println!(
            "Combat ledger: {} pirate(s) afloat ({} seeded + {} captured + {} mutinied), {} navy/privateer, {} lost (sunk or burned prize)",
            n_pirate, n_seeded_pirates, n_captured, n_mutinied, n_navy, n_lost,
        );
        // Step 11.a: prize-outcome breakdown. Almost all successful
        // boardings now result in cargo strip + hull dispatch (sink /
        // sold / released); flipping the prize to pirate is rare.
        println!(
            "Prize outcomes: {} taken (flipped to pirate), {} sold at haven, {} sunk, {} released",
            world.prizes_taken, world.prizes_sold, world.prizes_sunk, world.prizes_released,
        );
        // Step 10.b: non-combat attrition breakdown. Storm/foundering/
        // fire counters are sinkings only; storm damage that didn't
        // sink the hull lives in `weather.hazards.counters.storms_damaged`.
        let hz = world.weather.hazards.counters;
        println!(
            "Attrition: {} storm sinkings ({} damage-only), {} foundered, {} fires ({} sunk)",
            world.attrition_storms,
            hz.storms_damaged,
            world.attrition_foundered,
            hz.fires,
            world.attrition_fires,
        );
    }

    // Sailor pool snapshot (Step 3.a diagnostic). Categories the
    // calibration sweep will watch for collapse/explosion.
    {
        use sim_core::pop::PortCategory;
        let mut sums: std::collections::BTreeMap<&str, (u32, u32, u32)> =
            std::collections::BTreeMap::new();
        for (port, d) in world.ports.iter().zip(world.demographics.iter()) {
            let key = match d.category {
                PortCategory::EuropeanHub => "EuropeanHub",
                PortCategory::CaribbeanEntrepot => "CaribbeanEntrepot",
                PortCategory::SmallColonial => "SmallColonial",
                PortCategory::PirateHaven => "PirateHaven",
            };
            let e = sums.entry(key).or_insert((0, 0, 0));
            e.0 += 1;
            e.1 += d.seasoned;
            e.2 += d.unseasoned;
            // Silence unused warning if Display ever drops port.
            let _ = port;
        }
        println!();
        println!("Sailor pools by port category (Step 3.a):");
        println!(
            "  {:<20} {:>7} {:>12} {:>12} {:>12}",
            "category", "ports", "seasoned", "unseasoned", "total"
        );
        for (cat, (n, s, u)) in &sums {
            println!("  {:<20} {:>7} {:>12} {:>12} {:>12}", cat, n, s, u, s + u);
        }
    }

    // Builds-by-type summary: only count ships beyond the starter
    // fleet (those are the shipyard's actual output, not what we
    // seeded for the demo).
    if world.ships.len() > n_ships {
        let mut counts: std::collections::BTreeMap<&str, u32> = std::collections::BTreeMap::new();
        for &id in ship_ids.iter().skip(n_ships) {
            let Some(ship) = world.ships.get(id) else {
                continue;
            };
            let name = world.ship_types.get(ship.ship_type).name.as_str();
            *counts.entry(name).or_insert(0) += 1;
        }
        let parts: Vec<String> = counts.iter().map(|(n, c)| format!("{} {}", c, n)).collect();
        println!("  by type: {}", parts.join(", "));
    }

    // ── System-wide commodity flow (per month). Shows the structural
    //    economy underneath the trade noise: production capacity,
    //    consumption demand, and how much actual inventory accumulates
    //    on wharves vs. sloshes around in ship holds.
    println!();
    println!("System-wide commodity accounting (totals across all ports + ships):");
    let n_months = stockpile_per_month.len();
    let header_months: Vec<String> = (0..n_months).map(|m| format!("m{}", m)).collect();
    println!(
        "  {:<18} {:<7} {}",
        "good",
        "kind",
        header_months
            .iter()
            .map(|h| format!("{:>10}", h))
            .collect::<String>()
    );
    // Union of all goods that show up anywhere in any snapshot.
    let mut all_goods: std::collections::BTreeSet<u8> = std::collections::BTreeSet::new();
    for snap in stockpile_per_month
        .iter()
        .chain(in_transit_per_month.iter())
        .chain(production_per_month.iter())
        .chain(consumption_per_month.iter())
    {
        for (gid, _) in snap {
            all_goods.insert(gid.0);
        }
    }
    let lookup = |snap: &Vec<(GoodId, f32)>, gid: GoodId| -> f32 {
        snap.iter()
            .find(|(g, _)| *g == gid)
            .map(|(_, v)| *v)
            .unwrap_or(0.0)
    };
    let series_line = |snaps: &Vec<Vec<(GoodId, f32)>>, gid: GoodId| -> String {
        (0..n_months)
            .map(|m| format!("{:>10.0}", lookup(&snaps[m], gid)))
            .collect()
    };
    // Δtotal: net change in (stock + in-transit) between m-1 and m. The
    // gap between this and (prod - cons) is the simulation's commodity
    // "leak" — almost entirely ship provision consumption while sailing.
    let delta_total_line = |gid: GoodId| -> String {
        (0..n_months)
            .map(|m| {
                if m == 0 {
                    format!("{:>10}", "-")
                } else {
                    let prev = lookup(&stockpile_per_month[m - 1], gid)
                        + lookup(&in_transit_per_month[m - 1], gid);
                    let now = lookup(&stockpile_per_month[m], gid)
                        + lookup(&in_transit_per_month[m], gid);
                    format!("{:>+10.0}", now - prev)
                }
            })
            .collect()
    };
    // net = prod - cons each month (the system's structural flow before
    // accounting for ship attrition and trade dynamics).
    let net_line = |gid: GoodId| -> String {
        (0..n_months)
            .map(|m| {
                let p = lookup(&production_per_month[m], gid);
                let c = lookup(&consumption_per_month[m], gid);
                format!("{:>+10.0}", p - c)
            })
            .collect()
    };
    for gid_raw in all_goods {
        let gid = GoodId(gid_raw);
        let name = &world.goods.get(gid).name;
        println!(
            "  {:<18} {:<7} {}",
            name,
            "prod",
            series_line(&production_per_month, gid)
        );
        println!(
            "  {:<18} {:<7} {}",
            "",
            "cons",
            series_line(&consumption_per_month, gid)
        );
        println!("  {:<18} {:<7} {}", "", "net", net_line(gid));
        println!(
            "  {:<18} {:<7} {}",
            "",
            "stock",
            series_line(&stockpile_per_month, gid)
        );
        println!(
            "  {:<18} {:<7} {}",
            "",
            "transit",
            series_line(&in_transit_per_month, gid)
        );
        println!("  {:<18} {:<7} {}", "", "Δtotal", delta_total_line(gid));
    }

    // ── Annualized structural flow. Recipe × prosperity is steady-state
    //    over a sim this short, so annual figures are just 12 × monthly[0].
    //    Useful for sanity-checking whether the catalog's nominal capacity
    //    can sustain the fleet's demand and a year of port consumption.
    println!();
    println!("Annualized structural flow (12 × steady-state month):");
    println!(
        "  {:<18} {:>10} {:>10} {:>10}",
        "good", "prod/yr", "cons/yr", "net/yr"
    );
    for gid_raw in stockpile_per_month[0]
        .iter()
        .map(|(g, _)| g.0)
        .chain(production_per_month[0].iter().map(|(g, _)| g.0))
        .chain(consumption_per_month[0].iter().map(|(g, _)| g.0))
        .collect::<std::collections::BTreeSet<_>>()
    {
        let gid = GoodId(gid_raw);
        let name = &world.goods.get(gid).name;
        let p = lookup(&production_per_month[0], gid) * 12.0;
        let c = lookup(&consumption_per_month[0], gid) * 12.0;
        println!("  {:<18} {:>10.0} {:>10.0} {:>+10.0}", name, p, c, p - c);
    }

    // ── Equilibrium divergence diagnostic. Solve the Kantorovich LP
    //    against the same world (linear and voyage-cost models). For
    //    every (port, good) cell where the equilibrium has an opinion,
    //    average the day-0 vs end-of-run simulation prices and compare
    //    to the equilibrium "delivered" price. Big numbers = the
    //    simulation is structurally far from where a frictionless
    //    trade-allocator would push it.
    let port_specs: Vec<PortSpec> = world
        .ports
        .iter()
        .map(|p| PortSpec::from_world(p, archetype_for(&p.name).recipe()))
        .collect();
    let voyage_freight = FreightCostModel::ShipBased {
        stats: ShipStats::sloop(),
        day_rate_pesos: 8.0,
        provisions_price_per_ton: 18.0,
    };
    let eq = equilibrium::solve(&EquilibriumScenario {
        ports: port_specs.clone(),
        goods: &world.goods,
        freight: voyage_freight,
    });

    let mut diffs: Vec<(String, String, f32, f32, f32)> = Vec::new();
    for (port_idx, port) in world.ports.iter().enumerate() {
        let spec = &port_specs[port_idx];
        let mut seen = std::collections::HashSet::new();
        for (good, _) in spec
            .recipe
            .monthly_outputs
            .iter()
            .chain(spec.recipe.monthly_inputs.iter())
        {
            let good: GoodId = *good;
            if !seen.insert(good) {
                continue;
            }
            if let Some(p_eq) = eq.price_at(port_idx, good) {
                let p_sim = world.markets[port_idx].buy_price(good, &world.goods);
                let pct = if p_eq > 1.0 {
                    ((p_sim - p_eq) / p_eq).abs()
                } else {
                    0.0
                };
                diffs.push((
                    port.name.to_string(),
                    world.goods.get(good).name.clone(),
                    p_sim,
                    p_eq,
                    pct,
                ));
            }
        }
    }

    println!();
    println!("Equilibrium vs end-of-run simulation prices (voyage-cost LP):");
    println!("  cells compared: {}", diffs.len());
    if !diffs.is_empty() {
        let mean = diffs.iter().map(|d| d.4).sum::<f32>() / diffs.len() as f32;
        let max = diffs.iter().map(|d| d.4).fold(0.0_f32, f32::max);
        println!("  mean abs % deviation: {:.0}%", mean * 100.0);
        println!("  max  abs % deviation: {:.0}%", max * 100.0);

        // Show the 10 worst-divergence cells — these are the leading
        // candidates for "something's mispriced" investigation.
        let mut sorted = diffs.clone();
        sorted.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap());
        println!();
        println!("  Top 10 mispriced cells (|sim − eq| / eq):");
        println!(
            "    {:<24} {:<18} {:>10} {:>10} {:>8}",
            "port", "good", "sim", "eq", "Δ%"
        );
        for d in sorted.iter().take(10) {
            println!(
                "    {:<24} {:<18} {:>10.1} {:>10.1} {:>7.0}%",
                d.0,
                d.1,
                d.2,
                d.3,
                d.4 * 100.0
            );
        }
    }

    // A coarse "calibration health" verdict. In absence of piracy and
    // shipwreck (Phase 3 work), every ship that follows the trader AI
    // should reliably prosper — bankruptcies signal an underpaying
    // route, and a flat zero-profit fleet would mean the AI is just
    // sitting still. Wide variance between ships is fine and expected
    // (geography matters).
    println!();
    println!("Calibration verdict:");
    let losers = world
        .ships
        .values()
        .filter(|s| (s.silver - s.starting_silver) + s.lifetime_dividends < 0.0)
        .count();
    if bankrupt > 0 {
        println!(
            "  ⚠ {} bankrupt ship(s) — Phase 2 should not bankrupt anyone before piracy/wreck risk is added",
            bankrupt
        );
    } else if losers > 0 {
        println!(
            "  ⚠ {} ship(s) ended in the red — successful voyages should reliably profit",
            losers
        );
    } else if total_pl < world.ships.values().map(|s| s.starting_silver).sum::<f32>() * 0.5 {
        println!("  ⚠ fleet barely profitable — trade margins or world prices may be too thin");
    } else {
        println!("  ✓ every ship in the black with healthy variance");
    }
}
