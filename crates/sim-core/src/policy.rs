//! Faction trade policies and per-port overrides.
//!
//! This module captures the *legal* dimension of trade: which ships may
//! enter which ports, which goods may be bought or sold under which
//! flag, and what tariff the crown levies. It is intentionally
//! decoupled from military hostility (the Phase 4 `Relations` matrix)
//! and from smuggling (a Phase 4 layer that will wrap these queries).
//!
//! # Two-tier cascade
//!
//! Each faction has a `FactionTradePolicy` (the metropole's standard
//! regime — Spanish Casa de Contratación, English Navigation Acts,
//! French Exclusif, Dutch entrepôt, Free 0%). Each port may carry a
//! sparse `PortPolicy` of deltas — typically zero, but used for
//! historical exceptions like Petit-Goâve (lax French Caribbean
//! governor) or Charleston (Crown writ thin in the 1680s).
//!
//! # Smuggling-ready API
//!
//! Every query returns a structured legality enum, not a `bool`. A
//! future `SmugglingResolver` will wrap these methods: it inspects
//! `Refused` / `Prohibited` outcomes, rolls detection probability, and
//! optionally upgrades them to a smuggling deal with a bribe cost.
//! Today's transactional callers (`market::buy/sell`, the auction in
//! `world`, and the harbor docking check) treat `Refused` /
//! `Prohibited` as hard denials. When smuggling lands, only the
//! resolver changes — none of the call sites do.
//!
//! See `planning/research/trade-restrictions.md` for the historical
//! sourcing of the baseline numbers.

use std::collections::BTreeMap;

use crate::cargo::CARGO_SLOTS;
use crate::goods::{ids, GoodId, GoodsRegistry};
use crate::port::{Faction, Port};

/// Compact bitset over the 5 factions. Uses `Faction as u8` for the
/// bit index, so the set fits in one byte. Cheap to copy and stored
/// inline in policy data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FactionSet(pub u8);

impl FactionSet {
    pub const EMPTY: Self = FactionSet(0);
    pub const ALL: Self = FactionSet(0b0001_1111);

    pub const fn single(f: Faction) -> Self {
        FactionSet(1u8 << (f as u8))
    }

    pub fn contains(self, f: Faction) -> bool {
        (self.0 >> (f as u8)) & 1 == 1
    }

    pub fn with(mut self, f: Faction) -> Self {
        self.0 |= 1u8 << (f as u8);
        self
    }

    pub fn without(mut self, f: Faction) -> Self {
        self.0 &= !(1u8 << (f as u8));
        self
    }
}

/// Who may enter the harbor zone at all. Distinct from per-good trade
/// permission: a Dutch ship at Cartagena is refused docking outright;
/// a Dutch ship at Boston can dock freely but may find half its hold
/// is sell-prohibited.
#[derive(Clone, Debug)]
pub enum DockingRule {
    /// Any flag may enter (Curaçao, St. Eustatius, Free ports).
    Open,
    /// Only listed flags may enter (Spanish own-and-licensed model).
    OnlyFlags(FactionSet),
    /// All flags may enter except the listed ones (single-flag embargo).
    ClosedTo(FactionSet),
}

impl DockingRule {
    pub fn admits(&self, flag: Faction) -> bool {
        match self {
            DockingRule::Open => true,
            DockingRule::OnlyFlags(set) => set.contains(flag),
            DockingRule::ClosedTo(set) => !set.contains(flag),
        }
    }
}

/// Outcome of a docking-permission query. Structured rather than
/// `bool` so a future smuggling layer can introduce `Restricted` with
/// a detection probability without breaking call sites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockLegality {
    Open,
    Refused,
    // Future: Restricted { detection_base: f32 } — smuggler may try
    // under a false flag with a per-port detection roll.
}

/// Ad-valorem duty fractions on the cleared price. Applied as a wedge
/// between the ship's cashflow and the port's treasury receipt; the
/// difference accrues to `PortMarket::crown_silver`.
#[derive(Clone, Copy, Debug, Default)]
pub struct GoodTariff {
    /// Levied when the ship buys this good *from* the port
    /// (historically: almojarifazgo de salida / export duty). Ship
    /// pays `p × (1 + buy_duty)`; port treasury gets `p`; crown gets
    /// the difference.
    pub buy_duty: f32,
    /// Levied when the ship sells this good *to* the port
    /// (almojarifazgo de entrada / import duty). Ship receives
    /// `p × (1 - sell_duty)`; port treasury pays `p`; crown gets the
    /// difference.
    pub sell_duty: f32,
}

/// Per-good legality + tariff. The `default_rule` on a `TradeRules`
/// covers any good without an explicit override.
#[derive(Clone, Copy, Debug)]
pub struct GoodRule {
    pub buy_allowed: bool,
    pub sell_allowed: bool,
    pub tariff: GoodTariff,
}

impl GoodRule {
    /// All-permitted, zero-duty. Used as the Free / Dutch baseline
    /// (the Dutch policy then layers a small flat duty on top).
    pub const FREE: GoodRule = GoodRule {
        buy_allowed: true,
        sell_allowed: true,
        tariff: GoodTariff {
            buy_duty: 0.0,
            sell_duty: 0.0,
        },
    };

    /// All-prohibited. Used as the foreign-flag default at Spanish
    /// ports (everything is illegal absent a license).
    pub const PROHIBITED: GoodRule = GoodRule {
        buy_allowed: false,
        sell_allowed: false,
        tariff: GoodTariff {
            buy_duty: 0.0,
            sell_duty: 0.0,
        },
    };

    /// Convenience: flat-duty all-permitted.
    pub const fn flat(rate: f32) -> GoodRule {
        GoodRule {
            buy_allowed: true,
            sell_allowed: true,
            tariff: GoodTariff {
                buy_duty: rate,
                sell_duty: rate,
            },
        }
    }
}

/// Outcome of a per-good trade-permission query. Structured for the
/// same reason as `DockLegality`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TradeLegality {
    Legal { duty: f32 },
    Prohibited,
    // Future: Smuggling { base_duty, detection_base, bribe_floor } —
    // a `SmugglingResolver` will upgrade `Prohibited` to this when
    // appropriate.
}

/// Full trade rule-set for one ship-flag at one port. Sparse: only
/// goods that deviate from `default_rule` need an override.
#[derive(Clone, Debug)]
pub struct TradeRules {
    pub default_rule: GoodRule,
    pub overrides: BTreeMap<GoodId, GoodRule>,
}

impl TradeRules {
    pub fn uniform(rule: GoodRule) -> Self {
        TradeRules {
            default_rule: rule,
            overrides: BTreeMap::new(),
        }
    }

    pub fn rule_for(&self, good: GoodId) -> GoodRule {
        self.overrides
            .get(&good)
            .copied()
            .unwrap_or(self.default_rule)
    }
}

/// One faction's default trade policy — applied uniformly to every
/// port of that faction, then optionally overridden per-port via
/// `PortPolicy`. `trade_by_flag` is indexed by `Faction as usize`.
#[derive(Clone, Debug)]
pub struct FactionTradePolicy {
    pub docking: DockingRule,
    pub trade_by_flag: [TradeRules; 5],
}

/// Sparse per-port deltas. Loaded from `port_policies.ron`; ports not
/// listed there inherit their faction default verbatim.
#[derive(Clone, Debug, Default)]
pub struct PortPolicy {
    pub docking: Option<DockingRule>,
    /// Each entry replaces either one good's rule (Some(good)) or the
    /// entire `default_rule` (None) for the given ship-flag.
    pub trade_overrides: Vec<(Faction, Option<GoodId>, GoodRule)>,
}

// ============================================================
// Historical baselines (from planning/research/trade-restrictions.md)
// ============================================================

/// English enumerated goods (1660 / 1663 / 1672 Navigation Acts):
/// colonial produce that must be shipped via England before reaching
/// any non-English market. We model this as `sell_allowed = false`
/// for foreign flags at English Caribbean / colonial ports.
fn english_enumerated_goods() -> &'static [GoodId] {
    // Sugar, Molasses, Rum, Tobacco — the cash crops Parliament most
    // wanted to direct through London customs. The research doc also
    // lists cotton/indigo/ginger/dyewoods/cocoa but we don't have
    // those goods in the registry yet.
    const ENUM: &[GoodId] = &[ids::SUGAR, ids::MOLASSES, ids::RUM, ids::TOBACCO];
    ENUM
}

/// French enumerated colonial staples (Exclusif). Same shape as
/// English: must ship to France before any foreign market.
fn french_enumerated_goods() -> &'static [GoodId] {
    const ENUM: &[GoodId] = &[ids::SUGAR, ids::MOLASSES, ids::RUM, ids::TOBACCO];
    ENUM
}

/// Build the five-faction default policy table. Numbers are sourced
/// from `planning/research/trade-restrictions.md §Quick-Reference`.
pub fn faction_defaults() -> [FactionTradePolicy; 5] {
    // Helper: build a `TradeRules` from a base rule plus per-good
    // overrides (used for the enumerated-goods bans).
    let with_bans = |base: GoodRule, banned: &[GoodId]| {
        let mut t = TradeRules::uniform(base);
        for g in banned {
            t.overrides.insert(
                *g,
                GoodRule {
                    buy_allowed: false,
                    sell_allowed: false,
                    tariff: GoodTariff::default(),
                },
            );
        }
        t
    };

    // ---------- Netherlands: entrepôt model ----------
    // Curaçao + St. Eustatius were the great free ports of the period;
    // research §4.2 quotes a "1–3% duty rate" for foreigners with no
    // enumerated-good prohibitions. Open to all flags.
    let dutch = FactionTradePolicy {
        docking: DockingRule::Open,
        trade_by_flag: [
            TradeRules::uniform(GoodRule::flat(0.02)), // Spain
            TradeRules::uniform(GoodRule::flat(0.02)), // England
            TradeRules::uniform(GoodRule::flat(0.02)), // France
            TradeRules::uniform(GoodRule::flat(0.02)), // Netherlands (own)
            TradeRules::uniform(GoodRule::flat(0.02)), // Free
        ],
    };

    // ---------- England: Navigation Acts ----------
    // Open docking (smuggling was tolerated; the Acts targeted goods,
    // not hulls). Own flag: 5% plantation duty. Foreign flags: 10%
    // average impost + enumerated-goods sell ban.
    let english_own = TradeRules::uniform(GoodRule::flat(0.05));
    let english_foreign = with_bans(GoodRule::flat(0.10), english_enumerated_goods());
    let english = FactionTradePolicy {
        docking: DockingRule::Open,
        trade_by_flag: [
            english_foreign.clone(), // Spain
            english_own.clone(),     // England (own)
            english_foreign.clone(), // France
            english_foreign.clone(), // Netherlands
            english_foreign,         // Free
        ],
    };

    // ---------- France: Exclusif ----------
    // Metropolitan ports (Nantes) closed to foreigners as a default;
    // Caribbean ports inherit the same default and rely on per-port
    // overrides for the lax-governor exceptions (Petit-Goâve etc.).
    // Own flag: 5%. Foreign: 12% + enumerated bans.
    let french_own = TradeRules::uniform(GoodRule::flat(0.05));
    let french_foreign = with_bans(GoodRule::flat(0.12), french_enumerated_goods());
    let french = FactionTradePolicy {
        docking: DockingRule::ClosedTo(
            FactionSet::ALL
                .without(Faction::France)
                .without(Faction::Free), // Free ships (pirate buccaneers) historically tolerated at Tortuga/Petit-Goâve; we leave per-port override to widen further if needed.
        ),
        trade_by_flag: [
            french_foreign.clone(), // Spain
            french_foreign.clone(), // England
            french_own.clone(),     // France (own)
            french_foreign.clone(), // Netherlands
            french_foreign,         // Free
        ],
    };

    // ---------- Spain: Casa de Contratación ----------
    // Own-flag-only docking. Heavy crown duties on registered Spanish
    // commerce (research §1.6): almojarifazgo ~10% + averia ~15%
    // bundled here as a flat 22% on goods, 20% (quinto real) on silver.
    // Foreign flags: everything prohibited (the smuggling layer will
    // later upgrade `Prohibited` to `Smuggling { ... }`).
    let mut spanish_own = TradeRules::uniform(GoodRule {
        buy_allowed: true,
        sell_allowed: true,
        tariff: GoodTariff {
            buy_duty: 0.22,
            sell_duty: 0.22,
        },
    });
    spanish_own.overrides.insert(
        ids::SILVER,
        GoodRule {
            buy_allowed: true,
            sell_allowed: true,
            tariff: GoodTariff {
                buy_duty: 0.20, // quinto real on extraction; treated as a flat duty here
                sell_duty: 0.20,
            },
        },
    );
    let spanish = FactionTradePolicy {
        docking: DockingRule::OnlyFlags(FactionSet::single(Faction::Spain)),
        trade_by_flag: [
            spanish_own,                               // Spain (own)
            TradeRules::uniform(GoodRule::PROHIBITED), // England
            TradeRules::uniform(GoodRule::PROHIBITED), // France
            TradeRules::uniform(GoodRule::PROHIBITED), // Netherlands
            TradeRules::uniform(GoodRule::PROHIBITED), // Free
        ],
    };

    // ---------- Free: pirate havens ----------
    // Tortuga, Nassau, Tobago: zero duties, all flags welcome.
    let free = FactionTradePolicy {
        docking: DockingRule::Open,
        trade_by_flag: [
            TradeRules::uniform(GoodRule::FREE), // Spain
            TradeRules::uniform(GoodRule::FREE), // England
            TradeRules::uniform(GoodRule::FREE), // France
            TradeRules::uniform(GoodRule::FREE), // Netherlands
            TradeRules::uniform(GoodRule::FREE), // Free
        ],
    };

    // Indexed by `Faction as u8`.
    [spanish, english, french, dutch, free]
}

// ============================================================
// Resolver: precomputed per-port lookups
// ============================================================

/// One ship-flag's effective rule table at one port, fully expanded
/// into a flat array indexed by `GoodId.0 as usize`. Hot-loop friendly:
/// `find_best_trade` and the auction read a rule with a single
/// indirection plus an offset, no map probe. For a 38-port world the
/// full `[port][flag]` table is ~36 KB and fits in L1.
#[derive(Clone, Copy)]
struct FlatTradeRules {
    goods: [GoodRule; CARGO_SLOTS],
}

impl FlatTradeRules {
    fn from_sparse(rules: &TradeRules) -> Self {
        let mut goods = [rules.default_rule; CARGO_SLOTS];
        for (gid, rule) in &rules.overrides {
            let i = gid.0 as usize;
            debug_assert!(i < CARGO_SLOTS, "GoodId out of range for policy table");
            goods[i] = *rule;
        }
        FlatTradeRules { goods }
    }

    #[inline]
    fn rule_for(&self, good: GoodId) -> GoodRule {
        self.goods[good.0 as usize]
    }
}

/// Per-port effective rules, precomputed once at world load.
struct ResolvedPortPolicy {
    docking: DockingRule,
    trade_by_flag: [FlatTradeRules; 5],
}

/// Read-only policy resolver. Construct via [`PolicyResolver::build`]
/// from the port list and an optional per-port override map (loaded
/// from `port_policies.ron`).
pub struct PolicyResolver {
    per_port: Vec<ResolvedPortPolicy>,
    defaults: [FactionTradePolicy; 5],
}

impl PolicyResolver {
    /// Build a resolver. `overrides` maps `port_idx -> PortPolicy`
    /// and only needs entries for ports that deviate from their
    /// faction default.
    pub fn build(ports: &[Port], overrides: &BTreeMap<usize, PortPolicy>) -> Self {
        let defaults = faction_defaults();
        let mut per_port = Vec::with_capacity(ports.len());

        for (idx, port) in ports.iter().enumerate() {
            let base = &defaults[port.faction as usize];
            let ov = overrides.get(&idx);
            let docking = ov
                .and_then(|p| p.docking.clone())
                .unwrap_or_else(|| base.docking.clone());
            // Apply sparse per-port deltas on top of a clone of the
            // faction-default `TradeRules`, then flatten each
            // ship-flag's rules into a dense `[GoodRule; CARGO_SLOTS]`
            // so the hot loop never touches a map.
            let mut sparse = base.trade_by_flag.clone();
            if let Some(p) = ov {
                for (flag, good_opt, rule) in &p.trade_overrides {
                    let slot = &mut sparse[*flag as usize];
                    match good_opt {
                        Some(g) => {
                            slot.overrides.insert(*g, *rule);
                        }
                        None => {
                            // Replacing the default rule wipes any
                            // pre-existing per-good overrides for this
                            // flag — keeps override semantics simple.
                            slot.default_rule = *rule;
                            slot.overrides.clear();
                        }
                    }
                }
            }
            let trade_by_flag = [
                FlatTradeRules::from_sparse(&sparse[0]),
                FlatTradeRules::from_sparse(&sparse[1]),
                FlatTradeRules::from_sparse(&sparse[2]),
                FlatTradeRules::from_sparse(&sparse[3]),
                FlatTradeRules::from_sparse(&sparse[4]),
            ];
            per_port.push(ResolvedPortPolicy {
                docking,
                trade_by_flag,
            });
        }

        PolicyResolver { per_port, defaults }
    }

    /// Build a resolver with no per-port overrides (every port runs
    /// pure faction default). Useful for tests and the initial wiring
    /// before `port_policies.ron` exists.
    pub fn from_factions(ports: &[Port]) -> Self {
        Self::build(ports, &BTreeMap::new())
    }

    /// Borrow the raw faction-default table. Useful for diagnostics.
    pub fn faction_defaults(&self) -> &[FactionTradePolicy; 5] {
        &self.defaults
    }

    /// Can a ship of `flag` enter `port_idx`'s harbor zone?
    #[inline]
    pub fn dock_legality(&self, port_idx: usize, flag: Faction) -> DockLegality {
        let p = &self.per_port[port_idx];
        if p.docking.admits(flag) {
            DockLegality::Open
        } else {
            DockLegality::Refused
        }
    }

    /// May `flag` buy `good` at `port_idx` (and at what duty)?
    #[inline]
    pub fn buy_legality(&self, port_idx: usize, flag: Faction, good: GoodId) -> TradeLegality {
        let rule = self.per_port[port_idx].trade_by_flag[flag as usize].rule_for(good);
        if rule.buy_allowed {
            TradeLegality::Legal {
                duty: clamp_duty(rule.tariff.buy_duty),
            }
        } else {
            TradeLegality::Prohibited
        }
    }

    /// May `flag` sell `good` at `port_idx` (and at what duty)?
    #[inline]
    pub fn sell_legality(&self, port_idx: usize, flag: Faction, good: GoodId) -> TradeLegality {
        let rule = self.per_port[port_idx].trade_by_flag[flag as usize].rule_for(good);
        if rule.sell_allowed {
            TradeLegality::Legal {
                duty: clamp_duty(rule.tariff.sell_duty),
            }
        } else {
            TradeLegality::Prohibited
        }
    }
}

// ============================================================
// RON loading: data/registries/port_policies.ron
// ============================================================

/// Clamp duties to a safe interval so that gross-of-duty buys and net-of-duty
/// sells stay well-defined (in particular, `1 - sell_duty` stays positive so
/// the ship's net proceeds never go negative). Misconfigured duties beyond
/// this range are silently capped rather than panicking — historically all
/// quoted rates are well under 25%.
#[inline]
fn clamp_duty(d: f32) -> f32 {
    d.clamp(0.0, 0.99)
}

const PORT_POLICIES_RON: &str = include_str!("../../../data/registries/port_policies.ron");

#[derive(Clone, Debug, serde::Deserialize)]
enum DockingRuleRecord {
    Open,
    OnlyFlags(Vec<Faction>),
    ClosedTo(Vec<Faction>),
}

impl From<DockingRuleRecord> for DockingRule {
    fn from(r: DockingRuleRecord) -> Self {
        match r {
            DockingRuleRecord::Open => DockingRule::Open,
            DockingRuleRecord::OnlyFlags(v) => {
                let mut s = FactionSet::EMPTY;
                for f in v {
                    s = s.with(f);
                }
                DockingRule::OnlyFlags(s)
            }
            DockingRuleRecord::ClosedTo(v) => {
                let mut s = FactionSet::EMPTY;
                for f in v {
                    s = s.with(f);
                }
                DockingRule::ClosedTo(s)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
struct GoodTariffRecord {
    buy_duty: f32,
    sell_duty: f32,
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
struct GoodRuleRecord {
    buy_allowed: bool,
    sell_allowed: bool,
    tariff: GoodTariffRecord,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct TradeOverrideRecord {
    flag: Faction,
    /// `None` replaces the default_rule for this flag; `Some(name)`
    /// overrides one good. Good name is looked up against the goods
    /// registry at load.
    good: Option<String>,
    rule: GoodRuleRecord,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct PortPolicyRecord {
    port: String,
    docking: Option<DockingRuleRecord>,
    #[serde(default)]
    trade: Vec<TradeOverrideRecord>,
}

/// Load the bundled `port_policies.ron`, resolving port names to
/// indices in `ports` and good names against `goods`.
pub fn load_port_policies(
    ports: &[Port],
    goods: &GoodsRegistry,
) -> Result<BTreeMap<usize, PortPolicy>, PortPolicyLoadError> {
    load_port_policies_from_str(PORT_POLICIES_RON, ports, goods)
}

pub fn load_port_policies_from_str(
    s: &str,
    ports: &[Port],
    goods: &GoodsRegistry,
) -> Result<BTreeMap<usize, PortPolicy>, PortPolicyLoadError> {
    let records: Vec<PortPolicyRecord> = ron::from_str(s).map_err(PortPolicyLoadError::Ron)?;
    let mut out = BTreeMap::new();
    for r in records {
        let idx = ports
            .iter()
            .position(|p| p.name == r.port)
            .ok_or_else(|| PortPolicyLoadError::UnknownPort(r.port.clone()))?;
        let mut policy = PortPolicy {
            docking: r.docking.map(Into::into),
            trade_overrides: Vec::with_capacity(r.trade.len()),
        };
        for t in r.trade {
            let good_id =
                match t.good {
                    None => None,
                    Some(name) => Some(goods.by_name(&name).ok_or_else(|| {
                        PortPolicyLoadError::UnknownGood {
                            port: r.port.clone(),
                            good: name,
                        }
                    })?),
                };
            let rule = GoodRule {
                buy_allowed: t.rule.buy_allowed,
                sell_allowed: t.rule.sell_allowed,
                tariff: GoodTariff {
                    buy_duty: t.rule.tariff.buy_duty,
                    sell_duty: t.rule.tariff.sell_duty,
                },
            };
            policy.trade_overrides.push((t.flag, good_id, rule));
        }
        if out.insert(idx, policy).is_some() {
            return Err(PortPolicyLoadError::DuplicatePort(r.port));
        }
    }
    Ok(out)
}

#[derive(Debug)]
pub enum PortPolicyLoadError {
    Ron(ron::error::SpannedError),
    UnknownPort(String),
    UnknownGood { port: String, good: String },
    DuplicatePort(String),
}

impl std::fmt::Display for PortPolicyLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortPolicyLoadError::Ron(e) => write!(f, "port_policies.ron parse error: {e}"),
            PortPolicyLoadError::UnknownPort(p) => {
                write!(f, "port_policies.ron references unknown port {p:?}")
            }
            PortPolicyLoadError::UnknownGood { port, good } => write!(
                f,
                "port_policies.ron entry for port {port:?} references unknown good {good:?}"
            ),
            PortPolicyLoadError::DuplicatePort(p) => {
                write!(f, "port_policies.ron has duplicate entry for port {p:?}")
            }
        }
    }
}

impl std::error::Error for PortPolicyLoadError {}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goods::GoodsRegistry;
    use crate::pop::PortCategory;
    use crate::shiptype::ShipTypeRegistry;
    use crate::types::Position;

    fn synth_port(name: &str, faction: Faction) -> Port {
        Port {
            name: name.to_string(),
            position: Position::new(0.0, 0.0),
            faction,
            harbor_radius_nm: 5.0,
            shipyard: None,
            category: PortCategory::SmallColonial,
        }
    }

    #[test]
    fn faction_set_round_trip() {
        let s = FactionSet::EMPTY.with(Faction::Spain).with(Faction::France);
        assert!(s.contains(Faction::Spain));
        assert!(s.contains(Faction::France));
        assert!(!s.contains(Faction::England));
        assert!(!s.without(Faction::Spain).contains(Faction::Spain));
        assert!(FactionSet::ALL.contains(Faction::Free));
    }

    #[test]
    fn docking_rule_admits() {
        assert!(DockingRule::Open.admits(Faction::England));
        let only_spain = DockingRule::OnlyFlags(FactionSet::single(Faction::Spain));
        assert!(only_spain.admits(Faction::Spain));
        assert!(!only_spain.admits(Faction::England));
        let closed_to_dutch = DockingRule::ClosedTo(FactionSet::single(Faction::Netherlands));
        assert!(closed_to_dutch.admits(Faction::Spain));
        assert!(!closed_to_dutch.admits(Faction::Netherlands));
    }

    #[test]
    fn spanish_port_refuses_foreign_dockers() {
        let ports = vec![synth_port("Havana", Faction::Spain)];
        let r = PolicyResolver::from_factions(&ports);
        assert_eq!(r.dock_legality(0, Faction::Spain), DockLegality::Open);
        assert_eq!(r.dock_legality(0, Faction::England), DockLegality::Refused);
        assert_eq!(
            r.dock_legality(0, Faction::Netherlands),
            DockLegality::Refused
        );
    }

    #[test]
    fn dutch_port_open_to_all_low_duty() {
        let ports = vec![synth_port("Willemstad", Faction::Netherlands)];
        let r = PolicyResolver::from_factions(&ports);
        for f in [
            Faction::Spain,
            Faction::England,
            Faction::France,
            Faction::Netherlands,
            Faction::Free,
        ] {
            assert_eq!(r.dock_legality(0, f), DockLegality::Open);
            match r.buy_legality(0, f, ids::SUGAR) {
                TradeLegality::Legal { duty } => assert!((duty - 0.02).abs() < 1e-6),
                _ => panic!("dutch port should allow buying sugar under any flag"),
            }
        }
    }

    #[test]
    fn english_port_bans_foreign_sale_of_enumerated_sugar() {
        let ports = vec![synth_port("Port Royal", Faction::England)];
        let r = PolicyResolver::from_factions(&ports);
        // English own flag can sell sugar.
        match r.sell_legality(0, Faction::England, ids::SUGAR) {
            TradeLegality::Legal { duty } => assert!((duty - 0.05).abs() < 1e-6),
            _ => panic!("english own-flag should be able to sell sugar"),
        }
        // Foreign flags cannot.
        assert_eq!(
            r.sell_legality(0, Faction::Netherlands, ids::SUGAR),
            TradeLegality::Prohibited
        );
        assert_eq!(
            r.buy_legality(0, Faction::Netherlands, ids::SUGAR),
            TradeLegality::Prohibited
        );
        // But a non-enumerated good (manufactures) is permitted with 10% duty.
        match r.buy_legality(0, Faction::Netherlands, ids::MANUFACTURES) {
            TradeLegality::Legal { duty } => assert!((duty - 0.10).abs() < 1e-6),
            _ => panic!("english port should allow Dutch ship to buy manufactures"),
        }
    }

    #[test]
    fn spanish_silver_taxed_at_quinto() {
        let ports = vec![synth_port("Cartagena", Faction::Spain)];
        let r = PolicyResolver::from_factions(&ports);
        match r.buy_legality(0, Faction::Spain, ids::SILVER) {
            TradeLegality::Legal { duty } => assert!((duty - 0.20).abs() < 1e-6),
            _ => panic!("Spanish own-flag should be able to buy silver under quinto"),
        }
    }

    #[test]
    fn free_port_is_completely_open() {
        let ports = vec![synth_port("Nassau", Faction::Free)];
        let r = PolicyResolver::from_factions(&ports);
        for f in [
            Faction::Spain,
            Faction::England,
            Faction::France,
            Faction::Netherlands,
            Faction::Free,
        ] {
            assert_eq!(r.dock_legality(0, f), DockLegality::Open);
            match r.buy_legality(0, f, ids::RUM) {
                TradeLegality::Legal { duty } => assert_eq!(duty, 0.0),
                _ => panic!("free port should allow rum buys with zero duty"),
            }
        }
    }

    #[test]
    fn bundled_port_policies_load_and_apply() {
        // The bundled file must parse against the real port + goods
        // registries and produce a valid resolver.
        let ship_types = ShipTypeRegistry::starter();
        let ports = crate::port::all_ports(&ship_types);
        let goods = GoodsRegistry::starter();
        let overrides =
            load_port_policies(&ports, &goods).expect("bundled port_policies.ron must load");
        let _r = PolicyResolver::build(&ports, &overrides);
    }

    #[test]
    fn port_override_replaces_docking() {
        let ports = vec![synth_port("Cartagena", Faction::Spain)];
        let mut overrides = BTreeMap::new();
        overrides.insert(
            0,
            PortPolicy {
                docking: Some(DockingRule::Open),
                trade_overrides: Vec::new(),
            },
        );
        let r = PolicyResolver::build(&ports, &overrides);
        // Per-port override flips Spanish refusal to Open.
        assert_eq!(r.dock_legality(0, Faction::England), DockLegality::Open);
    }

    #[test]
    fn port_override_replaces_per_flag_default_rule() {
        let ports = vec![synth_port("Petit-Goâve", Faction::France)];
        let mut overrides = BTreeMap::new();
        overrides.insert(
            0,
            PortPolicy {
                docking: Some(DockingRule::Open),
                trade_overrides: vec![(Faction::England, None, GoodRule::flat(0.05))],
            },
        );
        let r = PolicyResolver::build(&ports, &overrides);
        // English flag now faces 5% on a non-enumerated good (where it
        // would have been 12% by French default).
        match r.buy_legality(0, Faction::England, ids::MANUFACTURES) {
            TradeLegality::Legal { duty } => assert!((duty - 0.05).abs() < 1e-6),
            _ => panic!("override should permit buy"),
        }
    }
}
