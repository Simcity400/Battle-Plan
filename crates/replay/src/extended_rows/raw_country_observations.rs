//! Sparse exact-raw observations reached by simulation input consumers.
//! Mapping and spelling have independent native selection laws. A missing
//! positive CountryLinks row never certifies a reserved-country mapping.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RawCountryMapping {
    #[default]
    Unreached,
    Refused,
    Observed {
        canonical_slot: u32,
        country_id: u32,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RawCountryTag {
    #[default]
    Unreached,
    Refused,
    Bytes(Vec<u8>),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawCountryObservation {
    pub raw: i32,
    pub mapping: RawCountryMapping,
    pub tag: RawCountryTag,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawCountryObservations {
    pub sample_hour: i32,
    pub entries: Vec<RawCountryObservation>,
}

impl Value for RawCountryMapping {
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(if matches!(self, Self::Observed { .. }) {
            9
        } else {
            1
        })
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, out: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Unreached => out.push(0),
            Self::Refused => out.push(1),
            Self::Observed {
                canonical_slot,
                country_id,
            } => {
                out.push(2);
                canonical_slot.encode_reserved(out)?;
                country_id.encode_reserved(out)?;
            }
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match input.u8()? {
            0 => Ok(Self::Unreached),
            1 => Ok(Self::Refused),
            2 => Ok(Self::Observed {
                canonical_slot: u32::decode(input)?,
                country_id: u32::decode(input)?,
            }),
            _ => Err(invalid("raw country mapping discriminator invalid")),
        }
    }
}
impl Value for RawCountryTag {
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        match self {
            Self::Bytes(bytes) => allocation::add(1, Value::encoded_bytes(bytes)?),
            _ => Ok(1),
        }
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(match self {
            Self::Unreached => Self::Unreached,
            Self::Refused => Self::Refused,
            Self::Bytes(bytes) => Self::Bytes(Value::try_clone(bytes)?),
        })
    }
    fn encode_reserved(&self, out: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Unreached => out.push(0),
            Self::Refused => out.push(1),
            Self::Bytes(bytes) => {
                out.push(2);
                bytes.encode_reserved(out)?;
            }
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match input.u8()? {
            0 => Ok(Self::Unreached),
            1 => Ok(Self::Refused),
            2 => Ok(Self::Bytes(Vec::<u8>::decode(input)?)),
            _ => Err(invalid("raw country tag discriminator invalid")),
        }
    }
}
value_struct!(RawCountryObservation {
    raw: i32,
    mapping: RawCountryMapping,
    tag: RawCountryTag
});

impl RawCountryObservations {
    pub fn check(&self) -> io::Result<()> {
        let mut previous = None;
        for entry in &self.entries {
            if previous.is_some_and(|raw| raw >= entry.raw) {
                return Err(invalid(
                    "raw country subjects not signed increasing and unique",
                ));
            }
            previous = Some(entry.raw);
            if matches!(entry.mapping, RawCountryMapping::Unreached)
                && matches!(entry.tag, RawCountryTag::Unreached)
            {
                return Err(invalid("raw country observation has no reached side"));
            }
            if entry.raw < 0 && matches!(entry.tag, RawCountryTag::Bytes(_)) {
                return Err(invalid("negative raw country spelling is unsupported"));
            }
            if let RawCountryMapping::Observed {
                canonical_slot,
                country_id,
            } = entry.mapping
                && (canonical_slot == 0) != (country_id == 0)
            {
                return Err(invalid("raw country mapping reserved identity mismatch"));
            }
        }
        Ok(())
    }
    pub fn entry(&self, hour: i32, raw: i32) -> Result<&RawCountryObservation, &'static str> {
        if self.sample_hour != hour {
            return Err("raw country observations sample is not current");
        }
        self.check()
            .map_err(|_| "raw country observations malformed")?;
        self.entries
            .binary_search_by_key(&raw, |entry| entry.raw)
            .ok()
            .and_then(|index| self.entries.get(index))
            .ok_or("raw country subject was not reached")
    }
}
impl Value for RawCountryObservations {
    const MINIMUM_ENCODED_BYTES: usize =
        i32::MINIMUM_ENCODED_BYTES + Vec::<RawCountryObservation>::MINIMUM_ENCODED_BYTES;
    fn encoded_bytes(&self) -> io::Result<usize> {
        self.check()?;
        allocation::add(
            Value::encoded_bytes(&self.sample_hour)?,
            Value::encoded_bytes(&self.entries)?,
        )
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.check()?;
        Ok(Self {
            sample_hour: self.sample_hour,
            entries: Value::try_clone(&self.entries)?,
        })
    }
    fn encode_reserved(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.check()?;
        self.sample_hour.encode_reserved(out)?;
        self.entries.encode_reserved(out)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let value = Self {
            sample_hour: i32::decode(input)?,
            entries: Vec::<RawCountryObservation>::decode(input)?,
        };
        value.check()?;
        Ok(value)
    }
}
opaque_value!(Option<RawCountryObservations>);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_zero_mapping_empty_bytes_and_refused_sides_round_trip() {
        let packet = RawCountryObservations {
            sample_hour: -8,
            entries: vec![
                RawCountryObservation {
                    raw: -1,
                    mapping: RawCountryMapping::Unreached,
                    tag: RawCountryTag::Refused,
                },
                RawCountryObservation {
                    raw: 7,
                    mapping: RawCountryMapping::Observed {
                        canonical_slot: 0,
                        country_id: 0,
                    },
                    tag: RawCountryTag::Bytes(Vec::new()),
                },
                RawCountryObservation {
                    raw: 8,
                    mapping: RawCountryMapping::Refused,
                    tag: RawCountryTag::Bytes(vec![0, 0xff]),
                },
            ],
        };
        let mut out = Vec::new();
        packet.encode(&mut out).unwrap();
        let mut input = Decoder::new(&out);
        assert_eq!(RawCountryObservations::decode(&mut input).unwrap(), packet);
        input.finish().unwrap();
        assert!(packet.entry(-7, 7).is_err());
        for len in 0..out.len() {
            assert!(RawCountryObservations::decode(&mut Decoder::new(&out[..len])).is_err());
        }
    }
    #[test]
    fn ambiguous_sparse_entries_and_invalid_discriminators_refuse() {
        let entry = RawCountryObservation {
            raw: 2,
            mapping: RawCountryMapping::Refused,
            tag: RawCountryTag::Unreached,
        };
        let packet = RawCountryObservations {
            entries: vec![entry.clone(), entry],
            ..Default::default()
        };
        assert!(packet.check().is_err());
        assert!(RawCountryMapping::decode(&mut Decoder::new(&[3])).is_err());
        assert!(RawCountryTag::decode(&mut Decoder::new(&[3])).is_err());
        assert!(
            RawCountryObservations {
                entries: vec![RawCountryObservation::default()],
                ..Default::default()
            }
            .check()
            .is_err()
        );
    }
}
