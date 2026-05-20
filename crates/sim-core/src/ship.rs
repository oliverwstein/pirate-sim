use crate::types::{Position, WindVector};

/// Ship performance characteristics.
pub struct ShipStats {
    pub speed_typical: f32,    // knots in moderate trade winds
    pub speed_max: f32,        // absolute maximum
    pub windward_ability: f32, // 0.0-1.0 (how well it sails upwind)
    pub no_go_half_angle: f32, // degrees from wind that ship cannot sail into
    pub crew: u32,             // crew complement (determines provision consumption)
    pub provision_capacity: f32, // max tons of provisions
}

impl ShipStats {
    pub fn sloop() -> Self {
        Self {
            speed_typical: 9.0,
            speed_max: 12.0,
            windward_ability: 0.8,
            no_go_half_angle: 40.0,
            crew: 25,
            provision_capacity: 3.0, // ~40 days of food for 25 crew
        }
    }

    /// Daily provision consumption in tons (based on crew size).
    /// Historical: ~4 lbs/man/day total food = 0.0018 tons/man/day.
    pub fn daily_provision_consumption(&self) -> f32 {
        self.crew as f32 * 0.0018
    }
}

/// The physical state of a ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipState {
    Sailing,
    Docked,
    Anchored,
}

/// A ship: purely physical entity. Heading is set externally by AI/player.
pub struct Ship {
    pub position: Position,
    pub heading: f32,          // degrees (0=N, 90=E, clockwise)
    pub speed: f32,            // current speed in knots
    pub state: ShipState,
    pub provisions: f32,       // tons of food remaining
    pub hull_fouling: f32,     // 0 = clean, 100 = fully encrusted
}

impl Ship {
    pub fn new(position: Position, state: ShipState) -> Self {
        let stats = ShipStats::sloop();
        Self {
            position,
            heading: 0.0,
            speed: 0.0,
            state,
            provisions: stats.provision_capacity,
            hull_fouling: 0.0,
        }
    }

    /// Set heading and commanded speed (the primary control inputs from
    /// AI/player). The commanded speed is what the ship will actually make
    /// good this tick (before fouling); the navigator is responsible for
    /// reducing it to reflect upwind tacking, sail damage, etc.
    pub fn set_steering(&mut self, heading: f32, speed: f32) {
        self.heading = heading;
        self.speed = speed;
    }

    /// Transition to sailing state.
    pub fn undock(&mut self) {
        self.state = ShipState::Sailing;
    }

    /// Dock at current position.
    pub fn dock(&mut self) {
        self.state = ShipState::Docked;
        self.speed = 0.0;
    }

    /// Anchor at current position.
    pub fn anchor(&mut self) {
        self.state = ShipState::Anchored;
        self.speed = 0.0;
    }

    /// Calculate effective speed: the commanded speed (set by the navigator)
    /// reduced by hull fouling (up to 30% penalty at full fouling).
    ///
    /// `_stats` and `_wind` are kept in the signature for API compatibility
    /// and future use (e.g., gust gusts overriding command), but the speed
    /// model is now driven by the navigator via `set_steering`.
    pub fn effective_speed(&self, _stats: &ShipStats, _wind: &WindVector) -> f32 {
        let fouling_penalty = 1.0 - self.hull_fouling * 0.003;
        self.speed * fouling_penalty
    }

    /// Advance position by one time step. Returns new position (doesn't apply it).
    pub fn compute_next_position(
        &self,
        stats: &ShipStats,
        wind: &WindVector,
        dt_hours: f32,
    ) -> Position {
        let speed = self.effective_speed(stats, wind);
        let distance_nm = speed * dt_hours;
        let rad = self.heading.to_radians();
        let dx = distance_nm * rad.sin();
        let dy = distance_nm * rad.cos();
        self.position + Position::new(dx, dy)
    }

    /// Consume provisions and accumulate fouling for one hour.
    /// Called by world tick. Returns true if provisions are exhausted.
    pub fn tick_resources(&mut self, stats: &ShipStats) -> bool {
        // TODO: provisions should only be consumed while sailing. Likewise, a ship should not accumulate fouling while careened, and should accumulate more while docked or anchored than while sailing.
        // Provision consumption: per hour = daily / 24
        let hourly_consumption = stats.daily_provision_consumption() / 24.0;
        self.provisions = (self.provisions - hourly_consumption).max(0.0);

        // Hull fouling: accumulates ~1 point per 5 days in tropics
        // = 1/(5*24) per hour ≈ 0.0083/hour
        self.hull_fouling = (self.hull_fouling + 0.0083).min(100.0);

        self.provisions <= 0.0
    }

    /// Resupply provisions for one hour at a port. Returns `true` once
    /// provisions have reached capacity (the AI uses this as a "done" flag).
    pub fn tick_resupply(&mut self, stats: &ShipStats) -> bool {
        self.provisions = (self.provisions + RESUPPLY_RATE_PER_HOUR).min(stats.provision_capacity);
        self.provisions >= stats.provision_capacity
    }

    /// Careen the hull for one hour at a port. Returns `true` once the
    /// hull is fully clean.
    pub fn tick_careen(&mut self) -> bool {
        self.hull_fouling = (self.hull_fouling - CAREEN_RATE_PER_HOUR).max(0.0);
        self.hull_fouling <= 0.0
    }

    /// Days of provisions remaining at current consumption rate.
    pub fn provisions_days_remaining(&self, stats: &ShipStats) -> f32 {
        let daily = stats.daily_provision_consumption();
        if daily > 0.0 { self.provisions / daily } else { f32::INFINITY }
    }
}

/// Tons of provisions taken on per hour while resupplying at a port.
const RESUPPLY_RATE_PER_HOUR: f32 = 0.5;

/// Fouling points removed per hour while careening at a port.
const CAREEN_RATE_PER_HOUR: f32 = 3.0;
pub fn speed_at_heading(heading: f32, stats: &ShipStats, wind: &WindVector) -> f32 {
    let wind_to = wind.direction_to();
    let relative_angle = angle_diff(heading, wind_to).abs();
    let efficiency = sail_efficiency(relative_angle, stats.windward_ability);
    let wind_factor = (wind.speed() / 15.0).clamp(0.3, 1.5);
    (stats.speed_typical * efficiency * wind_factor).clamp(0.5, stats.speed_max)
}

/// Sail efficiency based on relative wind angle.
fn sail_efficiency(relative_angle: f32, windward_ability: f32) -> f32 {
    let a = relative_angle.abs();
    if a < 30.0 {
        1.3
    } else if a < 60.0 {
        1.3 - (a - 30.0) / 30.0 * 0.3
    } else if a < 90.0 {
        1.0
    } else if a < 135.0 {
        1.0 - (a - 90.0) / 45.0 * (1.0 - 0.4 * windward_ability)
    } else {
        0.1 + 0.3 * windward_ability
    }
}

/// Signed angle difference in degrees, normalized to [-180, 180].
pub fn angle_diff(a: f32, b: f32) -> f32 {
    let mut diff = a - b;
    while diff > 180.0 { diff -= 360.0; }
    while diff < -180.0 { diff += 360.0; }
    diff
}

/// Normalize angle to [0, 360).
pub fn normalize_angle(mut a: f32) -> f32 {
    while a < 0.0 { a += 360.0; }
    while a >= 360.0 { a -= 360.0; }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_running_fast() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 };
        // Simulate navigator commanding running speed.
        ship.speed = speed_at_heading(ship.heading, &stats, &wind);
        assert!(ship.effective_speed(&stats, &wind) > 10.0);
    }

    #[test]
    fn test_beating_slow() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.heading = 0.0; // heading north
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: -15.0 }; // from north
        // Simulate navigator commanding the raw upwind hull speed (slow).
        ship.speed = speed_at_heading(ship.heading, &stats, &wind);
        assert!(ship.effective_speed(&stats, &wind) < 5.0);
    }

    #[test]
    fn test_state_transitions() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        assert_eq!(ship.state, ShipState::Docked);

        ship.undock();
        assert_eq!(ship.state, ShipState::Sailing);

        ship.anchor();
        assert_eq!(ship.state, ShipState::Anchored);
        assert_eq!(ship.speed, 0.0);

        ship.undock();
        ship.dock();
        assert_eq!(ship.state, ShipState::Docked);
    }

    #[test]
    fn test_provisions_consumption() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        let stats = ShipStats::sloop();
        let initial = ship.provisions;

        // Tick 24 hours
        for _ in 0..24 {
            ship.tick_resources(&stats);
        }

        let consumed = initial - ship.provisions;
        let expected_daily = stats.daily_provision_consumption();
        assert!((consumed - expected_daily).abs() < 0.001,
            "Expected ~{:.4} tons consumed in a day, got {:.4}", expected_daily, consumed);
    }

    #[test]
    fn test_hull_fouling_speed_penalty() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 }; // from south, running
        ship.speed = speed_at_heading(ship.heading, &stats, &wind);
        let clean_speed = ship.effective_speed(&stats, &wind);

        ship.hull_fouling = 50.0;
        let fouled_speed = ship.effective_speed(&stats, &wind);
        assert!(fouled_speed < clean_speed, "Fouled ship should be slower");
        // 50 fouling = 15% penalty
        let expected_ratio = 1.0 - 50.0 * 0.003;
        let actual_ratio = fouled_speed / clean_speed;
        assert!((actual_ratio - expected_ratio).abs() < 0.01);
    }

    #[test]
    fn test_provisions_days_remaining() {
        let ship = Ship::new(Position::ZERO, ShipState::Sailing);
        let stats = ShipStats::sloop();
        let days = ship.provisions_days_remaining(&stats);
        // 3.0 tons / (25 * 0.0018 tons/day) = ~66.7 days
        assert!(days > 60.0 && days < 70.0, "Expected ~67 days, got {}", days);
    }
}
