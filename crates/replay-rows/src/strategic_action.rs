use super::*;
use crate::value::value_struct;

pub use game_model::strategic_action::{
    StrategicAction, StrategicActionFailure, StrategicRedeploymentInputs,
};

value_struct!(StrategicRedeploymentInputs {
    country_modifier: i64,
    state_modifier: Option<i64>,
    friendly_state_modifier: Option<i64>,
});

impl crate::value::Value for StrategicAction {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;

    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(match self {
            Self::Unreached => 1,
            Self::Available(inputs) => 1 + crate::value::Value::encoded_bytes(inputs)?,
            Self::Withheld(_) => 2,
        })
    }

    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }

    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Unreached => output.push(0),
            Self::Available(inputs) => {
                output.push(1);
                crate::value::Value::encode_reserved(inputs, output)?;
            }
            Self::Withheld(reason) => {
                output.push(2);
                output.push(match reason {
                    StrategicActionFailure::Owner => 0,
                    StrategicActionFailure::CountryModifier => 1,
                    StrategicActionFailure::CurrentState => 2,
                    StrategicActionFailure::StateModifier => 3,
                    StrategicActionFailure::Friendly => 4,
                    StrategicActionFailure::FriendlyStateModifier => 5,
                });
            }
        }
        Ok(())
    }

    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Ok(match input.u8()? {
            0 => Self::Unreached,
            1 => Self::Available(crate::value::Value::decode(input)?),
            2 => Self::Withheld(match input.u8()? {
                0 => StrategicActionFailure::Owner,
                1 => StrategicActionFailure::CountryModifier,
                2 => StrategicActionFailure::CurrentState,
                3 => StrategicActionFailure::StateModifier,
                4 => StrategicActionFailure::Friendly,
                5 => StrategicActionFailure::FriendlyStateModifier,
                _ => return Err(invalid("strategic action refusal code invalid")),
            }),
            _ => return Err(invalid("strategic action status invalid")),
        })
    }
}
opaque_value!(StrategicAction);
