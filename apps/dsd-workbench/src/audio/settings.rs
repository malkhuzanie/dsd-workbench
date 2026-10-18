//! User-selectable audition quality.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RateTarget {
    /// 88.2 kHz (96 kHz for the 48 kHz family).
    #[default]
    Standard,
    /// 176.4 / 192 kHz.
    High,
    /// 352.8 / 384 kHz.
    Maximum,
}

impl RateTarget {
    pub const ALL: [Self; 3] = [Self::Standard, Self::High, Self::Maximum];

    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard · 88.2 / 96 kHz",
            Self::High => "High · 176.4 / 192 kHz",
            Self::Maximum => "Maximum · 352.8 / 384 kHz",
        }
    }

    /// Desired output rate for a DSD rate (the filter clamps it to what is achievable).
    pub fn target_hz(self, dsd_rate: u32) -> u32 {
        let base = if dsd_rate.is_multiple_of(44_100) {
            44_100
        } else {
            48_000
        };
        base * match self {
            Self::Standard => 2,
            Self::High => 4,
            Self::Maximum => 8,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::High => "high",
            Self::Maximum => "maximum",
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.key() == key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    Fast,
    #[default]
    Balanced,
    Precise,
    Maximum,
}

impl Quality {
    pub const ALL: [Self; 4] = [Self::Fast, Self::Balanced, Self::Precise, Self::Maximum];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast · shortest filter",
            Self::Balanced => "Balanced",
            Self::Precise => "Precise · steeper cut-off",
            Self::Maximum => "Maximum · longest filter",
        }
    }

    /// Filter length as a multiple of the decimation factor.
    pub fn taps_per_decimation(self) -> usize {
        match self {
            Self::Fast => 8,
            Self::Balanced => 16,
            Self::Precise => 32,
            Self::Maximum => 64,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Precise => "precise",
            Self::Maximum => "maximum",
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.key() == key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settings {
    pub rate: RateTarget,
    pub quality: Quality,
}
