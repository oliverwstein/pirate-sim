//! Ship commands — the "intent" half of the AI-Phase / Resolution-Phase
//! pipeline (see `planning/phase-3-plan.md` §3).
//!
//! A `ShipCommand` is emitted by a ship's AI during its read-only tick and
//! drained by the `World`'s Resolution Phase, which translates the intent
//! into a mutation. Step 5.c introduces only `Steer` — `act_sail` no longer
//! reaches through `ShipBtContext::ship` to call `set_steering` directly;
//! instead it pushes a `Steer` command that the world applies between the
//! AI and physics sub-steps.
//!
//! Future steps will extend this enum with `AttemptBoard` and
//! `StrikeColors`; Step 7 adds `FireBroadside`, routed through the same
//! buffer so inter-ship interactions can be resolved coherently within
//! a single tick.

use crate::types::ShipId;

/// A buffered intent issued by a ship's AI during the read-only AI Phase.
#[derive(Clone, Debug, PartialEq)]
pub enum ShipCommand {
    /// Set heading and commanded speed. Resolved by the world by calling
    /// `Ship::set_steering(heading, speed)` on the issuing ship before the
    /// physics sub-step.
    Steer { heading: f32, speed: f32 },
    /// Step 7: fire a single broadside at `target`. The attacker is the
    /// `ShipId` that issued the command (carried alongside it in the
    /// command queue tuple). Resolved in the Resolution Phase:
    /// reads attacker `cannons` and range to target; requires gunpowder
    /// and cannon shot in the attacker's cargo; applies hull + rigging
    /// damage to `target` via `combat::compute_broadside_damage`; and
    /// deducts powder + shot from the attacker's cargo. If the attacker
    /// is out of supply the command is silently dropped.
    FireBroadside { target: ShipId },
    /// Step 8: attempt to board `target`. Resolved deterministically in
    /// the Resolution Phase: re-checks that the attacker is within
    /// `combat::BOARDING_RANGE_NM` of the target at closest approach
    /// during this tick, and that the target's rigging is damaged
    /// enough (`< BOARDING_RIGGING_THRESHOLD * rigging_max`) that it
    /// cannot slip the grapples. On success, computes per-side losses
    /// via `combat::resolve_boarding`; if the attacker wins it either
    /// takes the prize (transferring half its crew aboard, flipping
    /// `target.policy = Pirate` and `target.faction = Free`) or — when
    /// the prize crew it would have to detach would leave the attacker
    /// below `stats.crew_min()` — burns the prize instead
    /// (`target.state = Sunk`). Loser flees if it survives.
    AttemptBoard { target: ShipId },
    /// Phase 4 §3c-1 (symmetric redesign): voluntarily break off an
    /// active engagement with `other`. Resolved by the world by
    /// mutually clearing `engaged_with` on both ships and stamping a
    /// short cooldown (`disengaged_until_minute`) so the pair does not
    /// immediately re-engage on the next fired broadside. Either party
    /// can emit this — the decision is purely tactical (out of
    /// ordnance, badly outclassed, outnumbered, or lost visual
    /// contact). See `ai.rs::COND_SHOULD_DISENGAGE`.
    Disengage { other: ShipId },
    /// Phase 4 §3c-2: strike colors (surrender) to `to`. The issuing
    /// ship is the prize; `to` is the victor. Resolved by clearing
    /// both ships' `engaged_with` and dispatching the surrendered
    /// hull to the shared prize-action resolver (take / sell / sink /
    /// release). Unlike `Disengage`, there is no cooldown — the prize
    /// is being handled, not just broken-off. See
    /// `ai.rs::COND_SHOULD_STRIKE`.
    Strike { to: ShipId },
}
