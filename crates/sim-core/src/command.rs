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
//! Future steps will extend this enum with `FireBroadside`, `AttemptBoard`,
//! and `StrikeColors`, all routed through the same buffer so inter-ship
//! interactions can be resolved coherently within a single tick.

/// A buffered intent issued by a ship's AI during the read-only AI Phase.
#[derive(Clone, Debug, PartialEq)]
pub enum ShipCommand {
    /// Set heading and commanded speed. Resolved by the world by calling
    /// `Ship::set_steering(heading, speed)` on the issuing ship before the
    /// physics sub-step.
    Steer { heading: f32, speed: f32 },
}
