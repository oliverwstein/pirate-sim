# Phase 2: Goods, Cargo, and Production

> **Goal of Phase 2:** make ships *do something* economic. The Caribbean
> map and the ships that move on it should now have a reason to move —
> goods produced in one place are valuable somewhere else, and merchant
> ships are the network that connects them.

## Where we are

- **Goods today:** exactly one — `provisions`, tracked as `f32` tons on
  each ship, magically available in unlimited supply at every port.
- **Production today:** none. Provisions appear at ports at a fixed
  resupply rate.
- **Money today:** none.
- **AI today:** ships pick ports without economic motive (random / round
  robin), sail there, resupply, repeat.

The architecture doc already sketched a richer system (`GoodData`,
`BuildingTypeData`, `Settlement`, Rhai trade rules). That sketch is good
*long-term* but is too much for Phase 2 to land at once. This plan aims
for a **sound economic skeleton** — fewer goods, no buildings yet, port
production aggregated to a single recipe per port — with deliberate hooks
where the bigger ideas (Rhai rules, buildings, populations, factions)
plug in later.

## The thesis

> Provisions are not a special case. They are the first instance of a
> generalized **Good × Stockpile × Price** system. Generalize them, then
> add 6–8 more goods to populate the network.

Once that's done, every interesting Phase 3+ feature (combat, factions,
smuggling, scripted laws, building economies) is "just" a layer on top
of: ships carrying goods, prices that vary, ports producing and
consuming, and AI that chases profit.

## What "sound" means for Phase 2

The system is sound if all of these hold in a 60-day demo:

1. Six independent merchant sloops, each with starting silver, complete
   round-trip voyages and end with non-zero P/L (most positive, some
   negative — bankruptcies are allowed).
2. Sugar is cheaper at Bridgetown than at Boston; provisions are cheaper
   at Boston than at Bridgetown; the price gap *narrows* visibly when a
   ship dumps cargo at the destination (stockpiles move prices).
3. A port left alone for a sim-month accumulates output goods and
   depletes input goods — production is real.
4. Goods, ports, and recipes are loaded from RON files, not hardcoded
   in Rust.
5. Bench `cargo run --release -p sim-core --example bench_pathfind`
   stays green; new bench `bench_trade` runs a 60-day economy in
   ≤ 5 seconds and prints P/L per ship.

## Scope (IN)

1. **Goods registry** (`data/goods.ron`): a small but historically real
   set covering all archetypes — initially **9 goods** (see §1.2). New
   goods are a one-line RON addition.
2. **Cargo on ships**: a `Cargo` struct (per-good tons), with a separate
   `provisions` allocation that keeps Phase 1 ration semantics.
3. **Port market**: stockpile per good, dynamic price = base_price ×
   supply_factor(stockpile). One market per port.
4. **Port production recipe** (`data/port_markets.ron`): a per-port
   monthly output and consumption vector. No buildings yet — production
   is at the port granularity.
5. **Money**: `Ship::silver` (pesos). Buy/sell at port prices. Bankrupt
   ships retire (despawn → respawn at a home port with stake silver).
6. **Trading AI**: replace random destination selection with a profit
   evaluator. Ships pick the most profitable (buy_port, good, sell_port)
   they can reach, given their current cargo capacity, silver, and DR
   range.
7. **World prices for Europe-bound goods**: an off-map "Europe sink"
   reachable from a small set of Atlantic-facing ports (Charles Town,
   Boston, Philadelphia, Bridgetown). Selling sugar there hits a world
   price, not a local one. This is the demand pump.
8. **HUD + visualization**: HUD shows selected ship's cargo, silver,
   active deal. Clicking a port shows its market (stockpiles + prices).

## Scope (OUT — explicitly deferred)

- **Buildings as entities** (sugar mills, distilleries). Production stays
  at port granularity. Buildings come in Phase 3 when populations and
  labor matter.
- **Population / labor models.** A port's monthly output is a fixed
  number times a "prosperity" multiplier; we don't yet model who works.
- **Trade laws / duties / Navigation Acts.** Buyers and sellers don't
  care about flag or origin yet. Rhai trade-rules hook is reserved
  in code (`fn check_trade_legality(...) -> bool { true }`) but not
  implemented.
- **Smuggling, contraband.** Falls out of trade laws — Phase 3.
- **Combat & cargo capture.** No combat in Phase 2; it's the natural
  Phase 3 follow-on.
- **Demand seasonality / hurricane disruptions.** The navigation plan
  introduces storms; this plan does not couple them to markets.
- **Slave trade narrative.** "Enslaved persons" appears in the goods
  registry as a historically central commodity; no narrative content,
  no "slaves used as labor input" mechanic. Tracked as price + flow
  only — there is no opt-out flag. The historical Caribbean economy
  cannot honestly be modeled without it.
- **Player UI for trading.** This is engine-and-AI only; no clickable
  "buy 5 hogsheads" button. That's a Phase 4 (interactive) item.

## Architecture

### Data shapes

```rust
// crates/sim-core/src/goods.rs (NEW)

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct GoodId(pub u8);  // small enum-like; 256 max is plenty

pub struct Good {
    pub id: GoodId,
    pub name: &'static str,           // "Sugar"
    pub category: GoodCategory,
    pub tons_per_unit: f32,           // 1 hogshead muscovado ≈ 0.6 tons
    pub base_price_pesos: f32,        // reference Caribbean price
    pub europe_price_pesos: f32,      // London price (or 0 if not Europe-bound)
    pub perishability: Perishability,
}

pub enum GoodCategory { Staple, Cash, Manufactured, Currency, Naval, Provision, Person }
pub enum Perishability { Indefinite, Months(u8), Days(u16) }

pub struct GoodsRegistry {
    goods: Vec<Good>,
}
impl GoodsRegistry {
    pub fn load(path: &Path) -> Self;
    pub fn get(&self, id: GoodId) -> &Good;
    pub fn by_name(&self, name: &str) -> Option<GoodId>;
    pub fn iter(&self) -> impl Iterator<Item = &Good>;
}
```

```rust
// crates/sim-core/src/cargo.rs (NEW)

pub struct Cargo {
    pub items: SmallVec<[(GoodId, f32); 4]>,  // tons per good
}
impl Cargo {
    pub fn total_tons(&self) -> f32;
    pub fn add(&mut self, id: GoodId, tons: f32);
    pub fn remove(&mut self, id: GoodId, tons: f32) -> f32;  // returns actually removed
    pub fn get(&self, id: GoodId) -> f32;
}
```

```rust
// crates/sim-core/src/market.rs (NEW)

pub struct PortMarket {
    pub stockpiles: Vec<(GoodId, f32)>,        // tons in stock
    pub silver: f32,                            // pesos in port treasury
    pub recipe: ProductionRecipe,
    pub is_europe_gateway: bool,                // sells to Europe at world price
}

pub struct ProductionRecipe {
    pub monthly_outputs: Vec<(GoodId, f32)>,    // tons produced
    pub monthly_inputs: Vec<(GoodId, f32)>,     // tons consumed (must be in stock)
    pub prosperity: f32,                        // multiplier; 1.0 baseline
}

impl PortMarket {
    pub fn price(&self, id: GoodId, registry: &GoodsRegistry) -> f32;
    pub fn buy_price(&self, id: GoodId, registry: &GoodsRegistry) -> f32;
    pub fn sell_price(&self, id: GoodId, registry: &GoodsRegistry) -> f32;
    pub fn try_buy(&mut self, id: GoodId, tons: f32, ship: &mut Ship) -> Result<f32, TradeError>;
    pub fn try_sell(&mut self, id: GoodId, tons: f32, ship: &mut Ship) -> Result<f32, TradeError>;
    pub fn tick_month(&mut self);
}
```

**Pricing rule (linear, simple, sound enough):**

```
price = base_price * (1.0 + PRICE_K * (target_stock - current_stock) / target_stock)
```
clamped to `[0.25 * base, 4.0 * base]`. `target_stock` is set per
good per port (the recipe defines it implicitly — we treat one month's
output × N as target). `PRICE_K` = 1.0 for v1.

Buy adds a small spread (e.g., +5%) and sell subtracts the same; the
delta is the port's "vig" and accrues to its silver. (This stops
infinite arbitrage of stationary stockpiles.)

```rust
// crates/sim-core/src/ship.rs (MODIFIED)

pub struct Ship {
    // ... existing fields
    pub cargo: Cargo,           // NEW (replaces ad-hoc provisions handling)
    pub silver: f32,            // NEW
}

pub struct ShipStats {
    // ... existing
    pub cargo_capacity_tons: f32,    // NEW — separate from provision_capacity
}
```

`provisions` becomes a regular `GoodId` and is read via
`ship.cargo.get(GOOD_PROVISIONS)`. `tick_resources` decrements that
slot of cargo instead of a dedicated `provisions: f32`. **The provision
budget remains separate from trade cargo** — `provision_capacity` is its
own hold; `cargo_capacity_tons` is for tradeable goods. Historically
correct: lower hold = stores, upper hold = cargo.

### World wiring

```rust
// world.rs

pub struct World {
    // ... existing
    pub goods: GoodsRegistry,                    // NEW
    pub markets: Vec<PortMarket>,                // NEW — parallel to ports
}

impl World {
    fn tick(&mut self) {
        // existing: weather, ships, etc.
        if self.date.is_first_of_month() {
            for market in &mut self.markets {
                market.tick_month();
            }
        }
    }
}
```

### Trading AI

```rust
// ai.rs — new sub-module ai/trader.rs

pub fn pick_best_trade(
    ship: &Ship,
    stats: &ShipStats,
    current_port: usize,
    ports: &[Port],
    markets: &[PortMarket],
    goods: &GoodsRegistry,
    pathfind: &PathfindContext,
) -> Option<TradeDecision>;

pub struct TradeDecision {
    pub good: GoodId,
    pub buy_tons: f32,
    pub destination_port: usize,
    pub estimated_profit: f32,
    pub estimated_voyage_days: f32,
}
```

Algorithm (O(P × G × P) ≈ 27 × 9 × 27 = 6,561 ops, trivially fast):

1. For each good `g` available at `current_port` (positive stockpile,
   reasonable price):
2. For each potential destination `d`:
3. Compute `buy_cost = market[current_port].buy_price(g) * tons_loadable`
4. Compute `sell_revenue = market[d].sell_price(g) * tons_loadable`
   (or world price if `d` is Europe gateway and `g.europe_price > 0`)
5. Estimate voyage days = `path_distance(current_port, d) / 5 knots`
6. Profit per day = `(sell_revenue - buy_cost) / voyage_days`
7. Pick the (g, d) with highest profit per day, subject to
   `buy_cost <= ship.silver` and `tons_loadable <= cargo_capacity`.

If no profitable trade exists (all options are loss-making), the ship
takes the cheapest provisions option and waits (sim a "demurrage" day).

### BT integration

The behaviour tree gains two new actions:

- `BuyCargo(good, tons)` — debits silver, credits cargo, decrements
  port stockpile. Pre-condition checked.
- `SellCargo(good, tons)` — inverse.

The existing arrival-at-port flow becomes:

```
Sequence:
  ResupplyToFull             (existing)
  CareenIfFouled             (existing)
  SellCargoIfHave            (NEW)
  PickBestTrade              (NEW; chooses next destination + buy)
  BuyCargo                   (NEW)
  Undock                     (existing)
  PathfindToDestination      (existing)
```

If `PickBestTrade` returns nothing, the ship still picks a destination
(falls back to current random selection) — degrades gracefully if the
market is empty.

### Visualization

- HUD on selected ship: silver, cargo manifest (good × tons), current
  trade plan ("Buy 12t sugar at Bridgetown → Sell at Boston, est.
  profit $340, 18 d").
- Click a port: side panel shows market (stockpiles + buy/sell prices).
- Optional: faint colored dots on shipping lanes representing flow of
  high-value goods. Stretch — defer to Phase 3.

## The 9 starter goods

| GoodId | Name | Category | Tons/unit | Base price (pesos/ton) | Europe price | Source ports | Sink ports | Why include |
|---|---|---|---|---|---|---|---|---|
| 0 | Provisions | Provision | 1.0 | 18 | — | Boston, Philadelphia, Charles Town | All Caribbean | Already exists; food |
| 1 | Muscovado Sugar | Staple | 1.0 | 70 | 130 | Bridgetown, Port Royal, Martinique | Europe gateways | Dominant export |
| 2 | Molasses | Staple | 1.0 | 25 | 35 | Same as sugar | Boston (→ rum) | Sugar by-product |
| 3 | Rum | Cash | 1.0 | 200 | 280 | Bridgetown, Port Royal, Boston | Europe, Africa-abstract | Easy distillation |
| 4 | Tobacco | Staple | 1.0 | 40 | 90 | Charles Town, Cuba | Europe gateways | Alt export |
| 5 | Manufactures | Manufactured | 1.0 | 250 | 200 | Europe gateways (imports!) | All Caribbean | European demand pump |
| 6 | Naval Stores | Naval | 1.0 | 80 | 110 | Boston, Philadelphia | Caribbean shipyards | Ship maintenance economy |
| 7 | Spanish Silver | Currency | 0.5 | 1000 | 1000 | Cartagena, Portobelo, Veracruz | Anywhere | High-value, low-volume |
| 8 | Enslaved Persons | Person | 0.5 | 600 | — | (none — abstracted off-map source) | Sugar islands | Historically central; included as a normal good |

(Prices are research-derived per-ton from `goods-taxonomy.md`. Tons-per-unit
of 1.0 is a simplification — real hogsheads vary; we'll calibrate later.)

The crucial "demand pump": **Manufactures** flow Europe → Caribbean
(imports) and **Sugar/Tobacco/Rum** flow Caribbean → Europe (exports).
Without this asymmetry, every ship would just oscillate between two
nearby Caribbean ports. With it, the trade circle is real:
*Europe gateway → buy manufactures → sail to sugar island → sell
manufactures, buy sugar → return to gateway → sell sugar to Europe →
repeat*. Triangular trade re-emerges as an economic equilibrium, not a
scripted route.

## File-level diff outline

```
crates/sim-core/src/
├── lib.rs                  re-export goods, market
├── goods.rs                NEW
├── cargo.rs                NEW
├── market.rs               NEW
├── ship.rs                 add cargo+silver fields; provisions read via cargo
├── port.rs                 (no breaking changes; markets live in World)
├── world.rs                load goods+markets; tick monthly
├── ai.rs                   shrink to BT glue
└── ai/
    └── trader.rs           NEW — pick_best_trade

data/
├── goods.ron               NEW
└── port_markets.ron        NEW — recipe per port

tools/preprocess/
└── (no changes — markets are hand-authored, not preprocessed)

crates/sim-viz/src/main.rs  HUD cargo + silver lines, click-port market panel
```

## Implementation order (each step ships green)

1. **Goods registry + RON loader.** Hardcode 9 goods initially in code,
   then move to RON. Add unit test: load registry, look up by name,
   iterate. No behavioural change.

2. **Cargo struct on Ship.** `provisions: f32` migrates to
   `cargo.get(GOOD_PROVISIONS)`. All resupply/consumption code is
   re-routed through cargo. Bench unchanged.

3. **PortMarket + stockpiles** with infinite-ish stockpiles and fixed
   prices. Resupply at a port now formally takes provisions out of
   the market's stockpile (which we top up artificially). No AI change
   yet; we just verify the plumbing.

4. **Production recipes + monthly tick.** Stockpiles now grow and
   shrink with production/consumption. Provisions actually run out at
   sugar islands that don't import enough. *This intentionally breaks
   some routes* — fixed in next step.

5. **Money + buy/sell actions.** Ships have silver; buying provisions
   costs silver; resupply still always succeeds while silver lasts.
   Add `bench_trade` example: 30 days, 3 ships, log P/L.

6. **Trading AI.** Profit evaluator picks destinations and cargo.
   Ships now actively trade — sugar moves from Barbados to Boston,
   manufactures the other way. P/L should turn positive on average.

7. **Europe gateway sink.** Atlantic-facing ports with
   `is_europe_gateway = true` accept exports at world price; we model
   the off-map European demand as an infinite buyer. Sugar profits
   stabilize the system.

8. **Visualization polish.** Cargo HUD, market panel.

9. **Calibration pass.** Run the demo for 60 days. Tune prosperity,
   monthly outputs, and `PRICE_K` until 1–2 ships go bankrupt over the
   period (some volatility), most thrive, and prices oscillate without
   exploding.

## Open questions to resolve before step 1

1. **Cargo capacity vs provision capacity:** keep separate (current
   plan) or unify into one hold? **Recommended: separate.** It mirrors
   reality and lets us tune the demo without provisions starving cargo.

2. **Initial silver per ship:** flat (e.g., 1,000 pesos for a sloop),
   or proportional to ship cargo capacity? **Recommended: 5 ×
   cargo_capacity_tons × base_price_of_provisions** ≈ enough to fill
   the hold once with a low-value good.

3. **Bankruptcy behaviour:** despawn-and-respawn, or merge into a
   "wreck" entity? **Recommended: despawn-respawn for v1**; "ship
   wreck" can be Phase 3 combat content.

4. **Slave-trade handling:** included as a normal good, no opt-out
   flag. The historical Caribbean economy cannot be modeled honestly
   without it; we treat it as data, not narrative content.

5. **AI knowledge:** does each ship know *all* port prices everywhere
   (broadcast / omniscient market), or only ports it has visited
   recently? **Recommended: omniscient for v1.** Fog-of-war markets
   are a great Phase 3 feature ("the Bahamas captain doesn't know
   Boston's flour glut yet").

6. **Price elasticity (`PRICE_K`):** linear with K=1.0, or non-linear?
   **Recommended: linear K=1.0 for v1.** Simple and easy to debug.

7. **Goods perishability:** model real spoilage in v1, or ignore?
   **Recommended: ignore in v1**; perishability is data-only, used
   by Phase 3 to discount over voyage days.

## What this plan deliberately does NOT do

- It does not introduce **buildings**. The architecture doc has
  `BuildingTypeData` reserved; we use a single port-level `recipe`
  instead. Buildings as in-world objects (with construction cost,
  upgrades, employees) are Phase 3.
- It does not introduce **factions caring about trade**. Spain doesn't
  charge tariffs on English sugar in v1. The Rhai-trade-rules hook is
  reserved (a `fn legal_trade(...) -> bool { true }` stub) and that's
  it.
- It does not introduce **player interaction**. This is an autonomous
  economy. Player commands come in Phase 4.
- It does not generalize **production beyond a port**. Plantations,
  mines, and shipyards are abstracted into the port's recipe. We are
  modeling *trade flow*, not *production locations*. That's enough for
  combat and faction conflict to have meaning later.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Price oscillations spiral (stockpiles → 0 → max price → buy frenzy → flood → min price) | Linear pricing + price clamps + slow monthly tick + buy/sell spread. |
| All ships pile onto one route | Profit-per-day evaluator already accounts for distance; competition narrows the spread; remaining concentration is realistic. |
| Provisions become too expensive at sugar islands and ships starve | Cargo separate from provisions hold; provisions resupply still works on credit if silver runs out (small overdraft). |
| RON loader complexity | Use serde + ron, mirror existing GEBCO/wind binary loaders' pattern. |
| Trading AI churn (re-evaluating every tick) | Re-evaluate only on port arrival, not in transit. |

