//! Android's memory reports as flankr's memory grades (plan 20261022_03,
//! the L3 amendments of 2026-10-24 12:21 EDT, "memory pressure in grades",
//! and 13:45 EDT, "Android's running trim levels mapped onto the memory
//! grades"). libmorph 0.4.1 gives `on_trim_memory(TrimLevel)` and
//! `on_low_memory`; [`App::memory_pressure`](crate::App::memory_pressure)
//! takes the grade.
//!
//! While the foreground service runs, Android counts the process as in
//! the foreground and sends only the running levels, so those map onto the
//! grades too:
//!
//! | Android level | grade |
//! |---|---|
//! | `RunningModerate`, `UiHidden` | [`Pressure::UiHidden`]: the most recent closed view kept, the open view ±5 screens |
//! | `RunningLow`, `Background`, `Moderate` | [`Pressure::Background`]: every closed view paged out, the open view ±1 screen |
//! | `RunningCritical`, `Complete`, `on_low_memory` | [`Pressure::Low`]: the full drop |

use flankr_companion::Pressure;
use libmorph::TrimLevel;

/// The grade for `level`. `None` for a level libmorph adds later, which is
/// logged and left alone.
pub fn pressure_for(level: TrimLevel) -> Option<Pressure> {
    Some(match level {
        TrimLevel::RunningModerate | TrimLevel::UiHidden => Pressure::UiHidden,
        TrimLevel::RunningLow | TrimLevel::Background | TrimLevel::Moderate => Pressure::Background,
        TrimLevel::RunningCritical | TrimLevel::Complete => Pressure::Low,
        other => {
            log::warn!("memory: a trim level flankr does not know: {other:?}");
            return None;
        }
    })
}

/// `on_low_memory`'s grade: the full drop, so the app is not killed.
pub const LOW_MEMORY: Pressure = Pressure::Low;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_gives_its_grade() {
        use TrimLevel::*;
        for (level, grade) in [
            (RunningModerate, Pressure::UiHidden),
            (UiHidden, Pressure::UiHidden),
            (RunningLow, Pressure::Background),
            (Background, Pressure::Background),
            (Moderate, Pressure::Background),
            (RunningCritical, Pressure::Low),
            (Complete, Pressure::Low),
        ] {
            assert_eq!(pressure_for(level), Some(grade), "{level:?}");
        }
        assert_eq!(LOW_MEMORY, Pressure::Low);
    }

    /// Not the order of Android's levels: `RunningCritical` is under
    /// `UiHidden` there, but the full drop here.
    #[test]
    fn the_running_levels_are_graded_on_their_own() {
        assert!(TrimLevel::RunningCritical < TrimLevel::UiHidden);
        assert!(
            pressure_for(TrimLevel::RunningCritical) > pressure_for(TrimLevel::UiHidden),
            "graded by what it asks, not by Android's order"
        );
    }
}
