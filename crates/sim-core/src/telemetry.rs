//! Per-entity telemetry: small running aggregates on factions and ports
//! that the sim accrues to as a byproduct of normal operation.
//!
//! These are *observability* aggregates — the simulation itself doesn't
//! read them to make decisions. They exist so benches, the visualizer,
//! and analytical tools can answer "how is this faction / port doing?"
//! without re-walking the ship list every query. See
//! `planning/observability-plan.md` for the deferred full event-journal
//! design; this module is the interim approach.
//!
//! Concurrency: every field is mutated either in a serial phase (most
//! economic accrual happens inside the resolution phase, which holds
//! `&mut World`) or via atomics (`dockings_by_flag`), which lets the
//! parallel AI phase increment counts without taking a mutex on the
//! port. Counts are commutative, so ordering of increments doesn't
//! affect the final value — runs remain deterministic.

use crate::goods::GoodId;
use crate::money::Pesos;
use crate::port::Faction;
use std::sync::atomic::{AtomicU32, Ordering};

/// Number of factions. Mirrors the `Faction` enum (Spain, England,
/// France, Netherlands, Free). Used for fixed-size per-faction arrays
/// so a (faction → count) lookup is a single index.
pub const N_FACTIONS: usize = 5;

/// Aggregates attached to a polity (the metropole / crown).
///
/// All counters are monotonically non-decreasing across the run; none
/// of them bleed or decay. The simulation never reads these fields;
/// they exist purely for end-of-run analysis.
#[derive(Default, Debug, Clone)]
pub struct FactionTelemetry {
    /// Total ad-valorem duty pesos collected at ports of this faction.
    /// Sum of buy-side + sell-side wedges credited to `PortMarket::crown_silver`
    /// in every clearing pass. Distinct from the live `crown_silver`
    /// (which bleeds back into the port treasury monthly) — this is
    /// the cumulative gross.
    pub crown_revenue: Pesos,
    /// Trade profits remitted home: cumulative dividends paid by
    /// ships of this flag into their `owner_port` treasury, summed
    /// across the run. Models the bullion / specie flow from the
    /// New World back to the metropole as it actually arises in the
    /// sim — a merchant captain dropping his surplus silver at the
    /// home port at the end of a successful voyage. Credited at the
    /// `ShipCommand::MarketDeposit` site in `world::clear_port`,
    /// which is also where `Ship::lifetime_dividends` is incremented.
    pub silver_returned_home: Pesos,
    /// Total ships built at shipyards of ports controlled by this
    /// faction. Incremented at `World::run_shipyard` whenever a new
    /// hull is launched.
    pub ships_built: u32,
    /// Total ships of this flag removed from the world (sunk by combat,
    /// storm, fire, foundering, scuttling, etc.). Incremented in
    /// `World::cleanup` before the removed-ship list is reaped.
    pub ships_lost: u32,
}

/// Aggregates attached to a port.
///
/// `dockings_by_flag` is `AtomicU32` because the docking transition
/// happens inside the parallel AI phase (`ai.rs::act_sail` calling
/// `ship.dock()`), where only a `&[PortTelemetry]` is available.
/// Other fields are written serially during clearing and creation.
#[derive(Debug)]
pub struct PortTelemetry {
    /// Cumulative pesos of duty collected at this specific port,
    /// summed across all goods and both sides. Matches the duty share
    /// of every credit into `PortMarket::crown_silver` at this port.
    pub lifetime_duties: Pesos,
    /// Base-currency value of every ton this port has sold to a
    /// visiting ship. Accumulates `tons × base_price` at clearing,
    /// across every good and every clearing pass. A port whose
    /// `lifetime_production` is zero after a long run has never
    /// successfully cleared a buy-side trade — useful for spotting
    /// stranded production caused by policy filtering.
    pub lifetime_production: Pesos,
    /// Same as `lifetime_production` but split by good. Sparse vec
    /// kept sorted by `GoodId.0` so equality + display are stable.
    pub lifetime_production_by_good: Vec<(GoodId, Pesos)>,
    /// Count of completed docking transitions at this port, split
    /// by the docking ship's flag. Incremented exactly once per
    /// `Ship::dock` call inside `ai::act_sail` once the harbor-zone
    /// gate is passed. Use `Ordering::Relaxed` — these counters are
    /// independent across (port_idx, faction) pairs and don't need
    /// to synchronize with any other memory.
    pub dockings_by_flag: [AtomicU32; N_FACTIONS],
}

impl Default for PortTelemetry {
    fn default() -> Self {
        Self {
            lifetime_duties: Pesos::ZERO,
            lifetime_production: Pesos::ZERO,
            lifetime_production_by_good: Vec::new(),
            dockings_by_flag: [
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
            ],
        }
    }
}

impl PortTelemetry {
    /// Record `pesos` of base-currency production for `good`. Adds
    /// `good` to the sparse map if it's not already present; keeps
    /// the vec sorted by `GoodId.0`.
    pub fn add_production(&mut self, good: GoodId, pesos: Pesos) {
        self.lifetime_production += pesos;
        match self
            .lifetime_production_by_good
            .binary_search_by_key(&good.0, |(g, _)| g.0)
        {
            Ok(i) => self.lifetime_production_by_good[i].1 += pesos,
            Err(i) => self.lifetime_production_by_good.insert(i, (good, pesos)),
        }
    }

    /// Record one docking by a ship flying `flag`. Safe to call from
    /// any thread.
    pub fn record_docking(&self, flag: Faction) {
        let idx = flag as usize;
        if idx < N_FACTIONS {
            self.dockings_by_flag[idx].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Snapshot the per-flag docking counters as a plain `[u32; 5]`.
    /// Use `Ordering::Relaxed` to mirror the writer side — analyses
    /// run between ticks, so there's no concurrent writer to fence.
    pub fn dockings_snapshot(&self) -> [u32; N_FACTIONS] {
        let mut out = [0u32; N_FACTIONS];
        for (i, slot) in self.dockings_by_flag.iter().enumerate() {
            out[i] = slot.load(Ordering::Relaxed);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goods::GoodId;
    use crate::money::Pesos;

    #[test]
    fn add_production_is_sorted_and_summed() {
        let mut t = PortTelemetry::default();
        t.add_production(GoodId(3), Pesos::from_pesos(100));
        t.add_production(GoodId(1), Pesos::from_pesos(50));
        t.add_production(GoodId(3), Pesos::from_pesos(25));
        assert_eq!(t.lifetime_production_by_good.len(), 2);
        assert_eq!(t.lifetime_production_by_good[0].0, GoodId(1));
        assert_eq!(t.lifetime_production_by_good[0].1, Pesos::from_pesos(50));
        assert_eq!(t.lifetime_production_by_good[1].0, GoodId(3));
        assert_eq!(t.lifetime_production_by_good[1].1, Pesos::from_pesos(125));
        assert_eq!(t.lifetime_production, Pesos::from_pesos(175));
    }

    #[test]
    fn docking_counter_per_flag() {
        let t = PortTelemetry::default();
        t.record_docking(Faction::England);
        t.record_docking(Faction::England);
        t.record_docking(Faction::Spain);
        let snap = t.dockings_snapshot();
        assert_eq!(snap[Faction::England as usize], 2);
        assert_eq!(snap[Faction::Spain as usize], 1);
        assert_eq!(snap[Faction::Free as usize], 0);
    }
}
