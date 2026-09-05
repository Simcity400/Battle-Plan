#[derive(Clone, Debug, PartialEq)]
pub struct Xmap07SnowMudPhaseState {
    fade: f32,
    previous_absolute_hour: i64,
    hour_of_day: i32,
    hour_started_seconds: f64,
    previous_update_seconds: f64,
}

impl Default for Xmap07SnowMudPhaseState {
    fn default() -> Self {
        Self {
            fade: 0.0,
            previous_absolute_hour: 0,
            hour_of_day: -1,
            hour_started_seconds: 0.0,
            previous_update_seconds: 0.0,
        }
    }
}

impl Xmap07SnowMudPhaseState {
    pub fn advance(
        &mut self,
        canonical_hour: i64,
        now_seconds: f64,
        hour_duration_seconds: f32,
    ) -> crate::Result<f32> {
        let absolute_hour = canonical_hour
            .checked_sub(1)
            .and_then(|hour| i32::try_from(hour).ok())
            .ok_or("XMAP07 viewer phase: hour is outside the engine date coordinate")?;
        if !now_seconds.is_finite() || now_seconds < 0.0 {
            return Err("XMAP07 viewer phase: timer is not finite and nonnegative".into());
        }
        if !hour_duration_seconds.is_finite() {
            return Err("XMAP07 viewer phase: authored hour duration is not finite".into());
        }
        let mut next = self.clone();
        let hour_of_day = absolute_hour % 24;
        if hour_of_day != next.hour_of_day || next.hour_started_seconds <= 0.0 {
            next.hour_of_day = hour_of_day;
            next.hour_started_seconds = now_seconds;
        }
        let deadline = f64::from(hour_duration_seconds) + next.hour_started_seconds;
        let mut weight = 1.0f32;
        if deadline > 0.0 && now_seconds < deadline {
            let remaining = (deadline - now_seconds) as f32;
            let elapsed = (now_seconds - next.previous_update_seconds) as f32;
            weight = if elapsed >= 0.0 {
                if elapsed <= remaining {
                    elapsed
                } else {
                    remaining
                }
            } else {
                0.0
            } / remaining;
        }
        next.previous_update_seconds = now_seconds;
        let absolute_hour = i64::from(absolute_hour);
        let previous = next.previous_absolute_hour;
        next.previous_absolute_hour = absolute_hour;
        let target = (absolute_hour % 24) as f32 / 24.0;
        next.fade = if absolute_hour.wrapping_sub(previous) as u64 >= 24 {
            target
        } else if next.fade <= target {
            (target - next.fade) * weight + next.fade
        } else {
            let wrapped = ((target + 1.0) - next.fade) * weight + next.fade;
            if wrapped >= 1.0 {
                wrapped - 1.0
            } else {
                wrapped
            }
        };
        if !next.fade.is_finite() {
            return Err("XMAP07 viewer phase: timer arithmetic produced a nonfinite phase".into());
        }
        let fade = next.fade;
        *self = next;
        Ok(fade)
    }
}

#[derive(Clone, Debug)]
pub struct Xmap07SnowMudPhaseDurations {
    seconds: [f32; 5],
    content_sha256: String,
}

impl Xmap07SnowMudPhaseDurations {
    pub fn load(resolver: &crate::GameFileResolver) -> crate::Result<Self> {
        let (sources, contents) = crate::xmap09_lighting_law::admitted_effective_define_sources(
            resolver, "XMAP07 viewer phase defines",
        )?;
        let mut value = Self::from_sources(&sources)?;
        value.content_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&contents)?);
        Ok(value)
    }

    pub(crate) fn from_sources(sources: &[(String, String)]) -> crate::Result<Self> {
        let defines = crate::xmap09_lighting_law::EffectiveDefines::from_sources(sources)?;
        let mut values = defines.number_array_allow_empty("NGame", "GAME_SPEED_SECONDS")?;
        values.resize(5, 0.0);
        let seconds = values.try_into().map_err(|_| "XMAP07 phase duration cardinality")?;
        Ok(Self {
            seconds,
            content_sha256: snapshot::sha256::Sha256::hex(&serde_json::to_vec(sources)?),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Xmap07SnowMudPhaseSample {
    world_sha256: String,
    fade_f32_bits: u32,
    pub(crate) provenance_sha256: String,
}

impl Xmap07SnowMudPhaseSample {
    pub fn for_frame(&self, world: &replay_core::WorldFrame) -> crate::Result<f32> {
        if self.world_sha256 != crate::xmap04_replay_frame_identity_sha256(world)? {
            return Err("XMAP07 viewer phase belongs to a different replay frame".into());
        }
        Ok(f32::from_bits(self.fade_f32_bits))
    }
}

impl Xmap07SnowMudPhaseState {
    pub fn sample(
        &mut self,
        world: &replay_core::WorldFrame,
        now_seconds: f64,
        speed_step: usize,
        durations: &Xmap07SnowMudPhaseDurations,
    ) -> crate::Result<Xmap07SnowMudPhaseSample> {
        let world_sha256 = crate::xmap04_replay_frame_identity_sha256(world)?;
        let duration = durations.seconds[speed_step.min(4)];
        let fade_f32_bits = self.advance(world.hour_index(), now_seconds, duration)?.to_bits();
        let provenance_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(
            &world_sha256, &durations.content_sha256, now_seconds.to_bits(),
            speed_step, duration.to_bits(), fade_f32_bits,
            self.hour_started_seconds.to_bits(), self.previous_absolute_hour,
        ))?);
        Ok(Xmap07SnowMudPhaseSample { world_sha256, fade_f32_bits, provenance_sha256 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_speed_arrays_apply_mod_expressions_and_engine_padding() {
        let sources = vec![
            ("00.lua".into(), "NDefines = { NGame = { GAME_SPEED_SECONDS = { 3, 2 } } }".into()),
            ("01.lua".into(), "NDefines.NGame.GAME_SPEED_SECONDS[1] = NDefines.NGame.GAME_SPEED_SECONDS[1] * 2".into()),
        ];
        assert_eq!(Xmap07SnowMudPhaseDurations::from_sources(&sources).unwrap().seconds,
            [6.0, 2.0, 0.0, 0.0, 0.0]);
        for (text, expected) in [
            ("{}", [0.0; 5]),
            ("{ 9, 8, 7, 6, 5, 4 }", [9.0, 8.0, 7.0, 6.0, 5.0]),
        ] {
            let sources = vec![("00.lua".into(), format!("NDefines = {{ NGame = {{ GAME_SPEED_SECONDS = {text} }} }}"))];
            assert_eq!(Xmap07SnowMudPhaseDurations::from_sources(&sources).unwrap().seconds, expected);
        }
    }

    #[test]
    fn constructor_and_first_map_update_follow_engine_hour_origin() {
        let mut phase = Xmap07SnowMudPhaseState::default();
        assert_eq!(phase.fade.to_bits(), 0);
        assert_eq!(phase.previous_absolute_hour, 0);
        assert_eq!(phase.hour_of_day, -1);
        assert_eq!(phase.hour_started_seconds.to_bits(), 0);
        assert_eq!(phase.previous_update_seconds.to_bits(), 0);
        assert_eq!(phase.advance(17_031_118, 100.0, 2.0).unwrap(), 0.875);
    }

    #[test]
    fn same_hour_updates_keep_timer_history_and_settle_while_paused() {
        let mut phase = Xmap07SnowMudPhaseState::default();
        assert_eq!(phase.advance(25, 100.0, 2.0).unwrap(), 0.0);
        assert_eq!(phase.advance(37, 100.5, 2.0).unwrap(), 0.125);
        assert_eq!(phase.advance(37, 101.0, 2.0).unwrap(), 0.25);
        assert_eq!(phase.advance(37, 102.5, 2.0).unwrap(), 0.5);
    }

    #[test]
    fn midnight_wrap_and_backward_or_whole_day_seek_use_distinct_paths() {
        let mut phase = Xmap07SnowMudPhaseState::default();
        assert_eq!(phase.advance(48, 100.0, 2.0).unwrap(), 23.0 / 24.0);
        assert_eq!(phase.advance(49, 100.5, 2.0).unwrap(), 31.0 / 32.0);
        assert_eq!(phase.advance(49, 103.0, 2.0).unwrap(), 0.0);
        assert_eq!(phase.advance(43, 103.5, 2.0).unwrap(), 0.75);
        assert_eq!(phase.advance(79, 104.0, 2.0).unwrap(), 0.25);
    }

    #[test]
    fn speed_changes_recompute_deadline_without_restarting_hour() {
        let mut phase = Xmap07SnowMudPhaseState::default();
        phase.advance(25, 100.0, 2.0).unwrap();
        assert_eq!(phase.advance(37, 100.5, 2.0).unwrap(), 0.125);
        assert_eq!(phase.advance(37, 100.5, 0.0).unwrap(), 0.5);
        assert_eq!(phase.hour_started_seconds, 100.5);
    }

    #[test]
    fn regressed_timer_clamps_elapsed_without_inventing_a_new_phase() {
        let mut phase = Xmap07SnowMudPhaseState::default();
        phase.advance(25, 100.0, 2.0).unwrap();
        assert_eq!(phase.advance(37, 100.5, 2.0).unwrap(), 0.125);
        assert_eq!(phase.advance(37, 100.25, 2.0).unwrap(), 0.125);
        assert_eq!(phase.previous_update_seconds, 100.25);
    }

    #[test]
    fn invalid_input_does_not_mutate_phase_history() {
        let mut phase = Xmap07SnowMudPhaseState::default();
        phase.advance(25, 100.0, 2.0).unwrap();
        let before = phase.clone();
        for (hour, now, duration) in [
            (25, f64::NAN, 2.0),
            (25, -1.0, 2.0),
            (25, 101.0, f32::INFINITY),
            (i64::MIN, 101.0, 2.0),
        ] {
            assert!(phase.advance(hour, now, duration).is_err());
            assert_eq!(phase, before);
        }
    }
}
