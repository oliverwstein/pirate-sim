use glam::Vec2;

/// Position in nautical miles from origin (17.5°N, 72.5°W).
/// X = East (positive), Y = North (positive).
pub type Position = Vec2;

/// Wind vector in knots (meteorological: u=east component, v=north component).
#[derive(Debug, Clone, Copy)]
pub struct WindVector {
    pub u: f32,
    pub v: f32,
}

impl WindVector {
    pub fn speed(&self) -> f32 {
        (self.u * self.u + self.v * self.v).sqrt()
    }

    /// Direction wind is coming FROM in degrees (0=N, 90=E, meteorological convention).
    pub fn direction_from(&self) -> f32 {
        // Wind vector points in direction wind is blowing TO.
        // Meteorological convention: direction wind comes FROM.
        let to_dir = self.v.atan2(self.u).to_degrees(); // angle of (u,v) from east
        // Convert: atan2 gives angle from +x axis (east), we want from +y axis (north)
        let from_dir = (270.0 - to_dir) % 360.0;
        if from_dir < 0.0 { from_dir + 360.0 } else { from_dir }
    }

    /// Direction wind is blowing TO in degrees (0=N, 90=E).
    pub fn direction_to(&self) -> f32 {
        (self.direction_from() + 180.0) % 360.0
    }
}

/// Simulation date tracker.
#[derive(Debug, Clone)]
pub struct SimDate {
    pub year: u16,
    pub day_of_year: u16, // 1-365
    pub hour: u8,         // 0-23
}

impl SimDate {
    pub fn new(year: u16, month: u8, day: u8) -> Self {
        let day_of_year = month_day_to_doy(month, day);
        Self { year, day_of_year, hour: 0 }
    }

    /// Month index 0-11.
    pub fn month(&self) -> u8 {
        doy_to_month(self.day_of_year)
    }

    pub fn advance_hours(&mut self, n: u32) {
        let total_hours = self.hour as u32 + n;
        let extra_days = total_hours / 24;
        self.hour = (total_hours % 24) as u8;

        let total_days = self.day_of_year as u32 + extra_days;
        if total_days > 365 {
            self.year += (total_days / 365) as u16;
            self.day_of_year = ((total_days - 1) % 365 + 1) as u16;
        } else {
            self.day_of_year = total_days as u16;
        }
    }

    pub fn total_hours_elapsed(&self, start_year: u16) -> u64 {
        let years = (self.year - start_year) as u64;
        let days = years * 365 + (self.day_of_year as u64 - 1);
        days * 24 + self.hour as u64
    }
}

fn month_day_to_doy(month: u8, day: u8) -> u16 {
    const MONTH_STARTS: [u16; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    MONTH_STARTS[month.min(11) as usize] + day as u16
}

fn doy_to_month(doy: u16) -> u8 {
    const MONTH_STARTS: [u16; 12] = [1, 32, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335];
    for i in (0..12).rev() {
        if doy >= MONTH_STARTS[i] {
            return i as u8;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sim_date_advance() {
        let mut date = SimDate::new(1680, 0, 1); // Jan 1
        assert_eq!(date.month(), 0);
        date.advance_hours(24);
        assert_eq!(date.day_of_year, 2);
        assert_eq!(date.hour, 0);
    }

    #[test]
    fn test_sim_date_year_wrap() {
        let mut date = SimDate { year: 1680, day_of_year: 365, hour: 23 };
        date.advance_hours(2);
        assert_eq!(date.year, 1681);
        assert_eq!(date.day_of_year, 1);
        assert_eq!(date.hour, 1);
    }

    #[test]
    fn test_wind_direction() {
        // Pure eastward wind (u=10, v=0) → blowing TO east → coming FROM west (270°)
        let w = WindVector { u: 10.0, v: 0.0 };
        assert!((w.direction_from() - 270.0).abs() < 1.0);

        // Pure northward wind (u=0, v=10) → blowing TO north → coming FROM south (180°)
        let w = WindVector { u: 0.0, v: 10.0 };
        assert!((w.direction_from() - 180.0).abs() < 1.0);
    }
}
