#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct StrategicRedeploymentInputs {
    pub country_modifier: i64,
    pub state_modifier: Option<i64>,
    pub friendly_state_modifier: Option<i64>,
}

impl StrategicRedeploymentInputs {
    pub fn blocked(self) -> bool {
        self.country_modifier != 0
            || self.state_modifier.is_some_and(|value| value != 0)
            || self.friendly_state_modifier.is_some_and(|value| value != 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum StrategicActionFailure {
    Owner,
    CountryModifier,
    CurrentState,
    StateModifier,
    Friendly,
    FriendlyStateModifier,
}

impl StrategicActionFailure {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Owner => "strategic action owner-country input unavailable",
            Self::CountryModifier => "strategic action owner-country modifier unavailable",
            Self::CurrentState => "strategic action current-state input unavailable",
            Self::StateModifier => "strategic action roster-state modifier unavailable",
            Self::Friendly => "strategic action friendly-controller input unavailable",
            Self::FriendlyStateModifier => "strategic action friendly-state modifier unavailable",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum StrategicAction {
    #[default]
    Unreached,
    Available(StrategicRedeploymentInputs),
    Withheld(StrategicActionFailure),
}
