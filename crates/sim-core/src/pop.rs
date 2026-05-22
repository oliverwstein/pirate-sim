//! Port demographics: sailor populations and their monthly dynamics.
//!
//! Each port carries a two-tier sailor pool (seasoned + unseasoned)
//! whose size evolves on the monthly tick via:
//!   1. organic growth (apprenticeships, fishing → deep-sea) — category-driven
//!   2. maturation (~3%/month of unseasoned mature into seasoned)
//!   3. mortality (seasoning disease in Caribbean ports dominates)
//!
//! Transient supply from ship arrivals lands in Step 3.c when ships
//! actually dock and discharge. For 3.a we wire the standing dynamics
//! only — the pool evolves in the background and is reported by
//! bench_trade so we can calibrate it before any ship touches it.
//!
//! Numerical defaults are calibrated against
//! `planning/research/sailor-recruitment.md §§7.1, 7.5`. They are
//! midpoint values picked for determinism; the Step 3.d sweep will
//! tune them. See `planning/crewing-plan.md §4`.

use crate::port::Faction;
use serde::Deserialize;

/// Coarse category that drives organic growth and the fraction of
/// transient sailors a port retains. Per crewing-plan §2.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize)]
pub enum PortCategory {
    /// Seville, Amsterdam, London, Bristol, Nantes — large
    /// apprenticeship + fishing populations feeding the deep-sea trade.
    EuropeanHub,
    /// Port Royal, Havana, Cartagena, Curaçao — the major nodes of
    /// the Caribbean circuit. Transient pool dominates the resident pool.
    CaribbeanEntrepot,
    /// Bridgetown, Boston, Charleston, San Juan, etc. — smaller
    /// colonial settlements with modest local maritime populations.
    SmallColonial,
    /// Tortuga, Nassau, Petit-Goâve — sailors only when ships arrive;
    /// negative organic growth from high desertion.
    PirateHaven,
}

/// Sailor population state of one port. Evolves on the monthly tick.
#[derive(Clone, Debug)]
pub struct PortDemographics {
    /// Skilled, low-mortality sailors. Drawn first by recruiters.
    pub seasoned: u32,
    /// Recently-arrived or freshly-converted hands; high mortality,
    /// matures into `seasoned` over months.
    pub unseasoned: u32,
    pub category: PortCategory,
}

impl PortDemographics {
    /// Total pool size (seasoned + unseasoned).
    pub fn total(&self) -> u32 {
        self.seasoned + self.unseasoned
    }

    /// Initial pool sized for a freshly-loaded world. Calibrated as
    /// crewing-plan §10 step 3.a — large enough that v1 ships can
    /// always crew, but small enough that pirate havens feel scarce.
    pub fn seed(category: PortCategory, faction: Faction) -> Self {
        let (seasoned, unseasoned) = match category {
            PortCategory::EuropeanHub => (4000, 2000),
            PortCategory::CaribbeanEntrepot => (180, 120),
            PortCategory::SmallColonial => (40, 30),
            PortCategory::PirateHaven => (15, 25),
        };
        // Apply the faction multiplier from crewing-plan §4.5.
        let mult = faction_growth_multiplier(faction);
        Self {
            seasoned: ((seasoned as f32) * mult).round() as u32,
            unseasoned: ((unseasoned as f32) * mult).round() as u32,
            category,
        }
    }
}

/// Faction culture multiplier on organic pool growth and seed size.
/// Per crewing-plan §4.5 / research §7.3.
pub fn faction_growth_multiplier(faction: Faction) -> f32 {
    match faction {
        Faction::England => 1.00,
        Faction::France => 0.90,
        Faction::Holland => 1.20,
        Faction::Spain => 0.50,
        Faction::Pirate => 0.30,
    }
}

/// Organic new sailors per month into the unseasoned pool, by
/// category (midpoint of research §7.1 ranges, monthly):
///
/// | Category          | Sailors/month |
/// |-------------------|---------------|
/// | EuropeanHub       | 100           |
/// | CaribbeanEntrepot | 3             |
/// | SmallColonial     | 1             |
/// | PirateHaven       | 0             |
fn organic_growth_per_month(cat: PortCategory) -> f32 {
    match cat {
        PortCategory::EuropeanHub => 100.0,
        PortCategory::CaribbeanEntrepot => 3.0,
        PortCategory::SmallColonial => 1.0,
        PortCategory::PirateHaven => 0.0,
    }
}

/// Monthly maturation rate: fraction of unseasoned that become
/// seasoned this month. Crewing-plan §4.3.
const MATURATION_RATE: f32 = 0.03;

/// Monthly mortality of unseasoned sailors in tropical (Caribbean /
/// pirate-haven / entrepot) ports. Research §7.5.
const UNSEASONED_TROPICAL_MORTALITY: f32 = 0.025;

/// Monthly mortality of unseasoned sailors in temperate (European)
/// ports. Research §7.5.
const UNSEASONED_TEMPERATE_MORTALITY: f32 = 0.0075;

/// Monthly mortality of seasoned sailors anywhere.
const SEASONED_MORTALITY: f32 = 0.005;

/// Apply one month of growth, maturation, and mortality to a single
/// port's pool. Deterministic — uses rounded expectations rather than
/// sampling, so a calibration run is reproducible. (Sampling can be
/// added later if the natural variance of small pools matters.)
pub fn tick_monthly(d: &mut PortDemographics, faction: Faction) {
    let temperate = matches!(d.category, PortCategory::EuropeanHub);

    // 1. Mortality first, on the standing pool before any inflows.
    let seasoned_dead = (d.seasoned as f32 * SEASONED_MORTALITY).round() as u32;
    let unseasoned_mortality_rate = if temperate {
        UNSEASONED_TEMPERATE_MORTALITY
    } else {
        UNSEASONED_TROPICAL_MORTALITY
    };
    let unseasoned_dead = (d.unseasoned as f32 * unseasoned_mortality_rate).round() as u32;
    d.seasoned = d.seasoned.saturating_sub(seasoned_dead);
    d.unseasoned = d.unseasoned.saturating_sub(unseasoned_dead);

    // 2. Maturation: a fraction of unseasoned graduate into seasoned.
    let matured = (d.unseasoned as f32 * MATURATION_RATE).round() as u32;
    d.unseasoned = d.unseasoned.saturating_sub(matured);
    d.seasoned += matured;

    // 3. Organic growth lands as fresh unseasoned hands.
    let mult = faction_growth_multiplier(faction);
    let new_unseasoned = (organic_growth_per_month(d.category) * mult).round() as u32;
    d.unseasoned += new_unseasoned;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_european_hub_is_large() {
        let d = PortDemographics::seed(PortCategory::EuropeanHub, Faction::England);
        assert!(d.total() > 5000);
    }

    #[test]
    fn seed_pirate_haven_is_small() {
        let d = PortDemographics::seed(PortCategory::PirateHaven, Faction::Pirate);
        assert!(d.total() < 50);
    }

    #[test]
    fn spain_seed_smaller_than_england_seed() {
        let s = PortDemographics::seed(PortCategory::CaribbeanEntrepot, Faction::Spain);
        let e = PortDemographics::seed(PortCategory::CaribbeanEntrepot, Faction::England);
        assert!(s.total() < e.total());
    }

    #[test]
    fn monthly_tick_european_hub_grows_steadily() {
        // A European hub should consistently grow even with mortality.
        let mut d = PortDemographics::seed(PortCategory::EuropeanHub, Faction::England);
        let start = d.total();
        for _ in 0..12 {
            tick_monthly(&mut d, Faction::England);
        }
        assert!(
            d.total() > start,
            "expected growth: start={} end={}",
            start,
            d.total()
        );
    }

    #[test]
    fn monthly_tick_pirate_haven_does_not_grow() {
        // No organic growth; with deterministic rounded expectations
        // and small pools, mortality may round to zero in any given
        // month — but the pool never *grows* without arrivals.
        // (Step 3.c adds transient supply from ship arrivals; stochastic
        // mortality is a Phase-4 refinement.)
        let mut d = PortDemographics::seed(PortCategory::PirateHaven, Faction::Pirate);
        let start = d.total();
        for _ in 0..12 {
            tick_monthly(&mut d, Faction::Pirate);
        }
        assert!(
            d.total() <= start,
            "expected no growth: start={} end={}",
            start,
            d.total()
        );
    }

    #[test]
    fn unseasoned_matures_into_seasoned() {
        // Stuff a pool with unseasoned only and run a tick: some should
        // mature even after mortality.
        let mut d = PortDemographics {
            seasoned: 0,
            unseasoned: 1000,
            category: PortCategory::SmallColonial,
        };
        tick_monthly(&mut d, Faction::England);
        assert!(d.seasoned > 0, "expected some maturation");
    }

    #[test]
    fn caribbean_pool_finds_steady_state() {
        // Over a year, a Caribbean entrepôt should neither vanish nor
        // explode — it should settle within ~2x of seed.
        let mut d = PortDemographics::seed(PortCategory::CaribbeanEntrepot, Faction::England);
        let start = d.total();
        for _ in 0..24 {
            tick_monthly(&mut d, Faction::England);
        }
        assert!(d.total() > start / 4, "collapsed: {} → {}", start, d.total());
        assert!(d.total() < start * 4, "exploded: {} → {}", start, d.total());
    }
}
