use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum TemplateNameValue {
    #[default]
    SameAsDivisionTemplates,
    Changed {
        literal: Vec<u8>,
        fallback_key: Option<Vec<u8>>,
    },
}

impl TemplateNameValue {
    fn validate(&self) -> io::Result<()> {
        if let Self::Changed {
            literal,
            fallback_key,
        } = self
            && literal.is_empty() != fallback_key.is_some()
        {
            return Err(invalid("template display fallback demand is noncanonical"));
        }
        Ok(())
    }
}

impl Value for TemplateNameValue {
    fn heap_bytes(&self) -> usize {
        match self {
            Self::SameAsDivisionTemplates => 0,
            Self::Changed {
                literal,
                fallback_key,
            } => Value::heap_bytes(literal) + Value::heap_bytes(fallback_key),
        }
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;

    fn encoded_bytes(&self) -> io::Result<usize> {
        self.validate()?;
        match self {
            Self::SameAsDivisionTemplates => Ok(1),
            Self::Changed {
                literal,
                fallback_key,
            } => allocation::add(
                allocation::add(1, Value::encoded_bytes(literal)?)?,
                Value::encoded_bytes(fallback_key)?,
            ),
        }
    }

    fn try_clone(&self) -> io::Result<Self> {
        self.validate()?;
        Ok(match self {
            Self::SameAsDivisionTemplates => Self::SameAsDivisionTemplates,
            Self::Changed {
                literal,
                fallback_key,
            } => Self::Changed {
                literal: Value::try_clone(literal)?,
                fallback_key: Value::try_clone(fallback_key)?,
            },
        })
    }

    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.validate()?;
        match self {
            Self::SameAsDivisionTemplates => output.push(0),
            Self::Changed {
                literal,
                fallback_key,
            } => {
                output.push(1);
                Value::encode_reserved(literal, output)?;
                Value::encode_reserved(fallback_key, output)?;
            }
        }
        Ok(())
    }

    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let value = match input.u8()? {
            0 => Self::SameAsDivisionTemplates,
            1 => Self::Changed {
                literal: Vec::<u8>::decode(input)?,
                fallback_key: Option::<Vec<u8>>::decode(input)?,
            },
            _ => return Err(invalid("template display value tag is not 0 or 1")),
        };
        value.validate()?;
        Ok(value)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TemplateDisplayName {
    pub sample_hour: PacketHour,
    pub value: TemplateNameValue,
}
value_struct!(TemplateDisplayName {
    sample_hour: PacketHour,
    value: TemplateNameValue
});
impl OpaqueValue for TemplateDisplayName {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateDisplayNameParts<'a> {
    pub literal: &'a [u8],
    pub fallback_key: Option<&'a [u8]>,
}

impl DivisionTemplateIconRow {
    pub fn display_name_inputs<'a>(
        &'a self,
        hour: i32,
        daily: Option<&'a DivisionTemplateRow>,
    ) -> Result<TemplateDisplayNameParts<'a>, &'static str> {
        let inputs = match &self.display_name {
            Recorded::NotRecorded => return Err("DivisionTemplateIcons.display_name not recorded"),
            Recorded::Value(None) => {
                return Err(
                    "DivisionTemplateIcons.display_name guarded read or name arena refusal",
                );
            }
            Recorded::Value(Some(inputs)) => inputs,
        };
        if !inputs.sample_hour.fresh_at(hour) {
            return Err("DivisionTemplateIcons.display_name sample is not current");
        }
        match &inputs.value {
            TemplateNameValue::Changed {
                literal,
                fallback_key,
            } => {
                if literal.is_empty() != fallback_key.is_some() {
                    return Err(
                        "DivisionTemplateIcons.display_name fallback demand is noncanonical",
                    );
                }
                Ok(TemplateDisplayNameParts {
                    literal,
                    fallback_key: fallback_key.as_deref(),
                })
            }
            TemplateNameValue::SameAsDivisionTemplates => {
                let daily = daily.filter(|row| row.template == self.template)
                    .ok_or("DivisionTemplateIcons.display_name equality certificate has no same-frame DivisionTemplates row")?;
                let fallback_key = if daily.name.is_empty() {
                    match &daily.name_key {
                        Recorded::Value(bytes) => Some(bytes.as_slice()),
                        Recorded::NotRecorded => {
                            return Err(
                                "DivisionTemplates.name_key not recorded for display-name equality certificate",
                            );
                        }
                    }
                } else {
                    None
                };
                Ok(TemplateDisplayNameParts {
                    literal: &daily.name,
                    fallback_key,
                })
            }
        }
    }
}
