
use replay_data::{day_night::Clock, rows::ClockState, rows::WeatherProvinceRow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalClock {
    pub hour: i32,
    pub night: i64,
    pub icon_frame: i32,
}

impl From<Clock> for LocalClock {
    fn from(clock: Clock) -> Self {
        Self {
            hour: clock.local_hour,
            night: clock.night_factor,
            icon_frame: clock.icon_frame(),
        }
    }
}

pub fn resolve(
    province: &WeatherProvinceRow,
    updated: impl FnOnce(u32) -> Result<Clock, String>,
    gates: impl FnOnce() -> Result<Option<[u8; 2]>, String>,
) -> Result<LocalClock, String> {
    match province.clock.state(province.id)? {
        ClockState::Held(clock) => Ok(clock.into()),
        ClockState::Updated => updated(province.id).map(Into::into),
        ClockState::NotRecorded => {
            match gates()? {
                Some([0, 0]) => {}
                Some(flags) => {
                    return Err(format!(
                        "Province local clock/night refused: weather manager is suspended ({}, {})",
                        flags[0], flags[1]
                    ));
                }
                None => {
                    return Err(
                        "Province local clock/night refused: weather-manager suspension flags unavailable"
                            .into(),
                    );
                }
            }
            if province.id == 0 {
                return Err(
                    "Province local clock/night refused: slot 0 is never updated and its clock was not recorded"
                        .into(),
                );
            }
            updated(province.id).map(Into::into)
        }
    }
}

#[cfg(test)]
mod tests;
