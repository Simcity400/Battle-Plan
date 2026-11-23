
use std::io;

use super::{Decimal, GameDate, block, bytes, field, integer, invalid, required_bytes, scalar};
use content_reader_parser::parser::Item;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub speed: Option<i32>,
    pub threat_sources: Option<Vec<ThreatSource>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreatSource {
    pub ordinal: u32,
    pub tag: Option<Vec<u8>>,
    pub target: Option<Vec<u8>>,
    pub threat: Decimal,
    pub final_threat: Decimal,
    pub daily: Decimal,
    pub date: GameDate,
    pub label: Option<Vec<u8>>,
    pub extra_keys: Vec<Vec<u8>>,
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    let speed = field(items, b"speed")?
        .map(|value| integer_word(scalar(value)?))
        .transpose()?;
    let threat_sources = field(items, b"threat")?
        .map(|value| {
            let mut sources = Vec::new();
            for item in block(value)? {
                if item.key.as_deref() != Some(b"threat") {
                    return Err(invalid("world threat block holds a non-threat entry"));
                }
                let fields = block(&item.value)?;
                sources.push(ThreatSource {
                    ordinal: u32::try_from(sources.len())
                        .map_err(|_| invalid("saved threat source count exceeds u32"))?,
                    tag: bytes(fields, b"tag")?,
                    target: bytes(fields, b"target")?,
                    threat: Decimal::parse(&required_bytes(fields, b"threat")?)?,
                    final_threat: Decimal::parse(&required_bytes(fields, b"final_threat")?)?,
                    daily: Decimal::parse(&required_bytes(fields, b"daily")?)?,
                    date: GameDate::parse(&required_bytes(fields, b"date")?)?,
                    label: bytes(fields, b"label")?,
                    extra_keys: fields
                        .iter()
                        .filter_map(|field| field.key.clone())
                        .filter(|key| {
                            !matches!(
                                key.as_slice(),
                                b"tag"
                                    | b"target"
                                    | b"threat"
                                    | b"final_threat"
                                    | b"daily"
                                    | b"date"
                                    | b"label"
                            )
                        })
                        .collect(),
                });
            }
            Ok(sources)
        })
        .transpose()?;
    Ok(Pack {
        speed,
        threat_sources,
    })
}

fn integer_word(bytes: &[u8]) -> io::Result<i32> {
    if matches!(bytes, b"yes" | b"no") {
        return Err(invalid("saved speed is a boolean, not an integer index"));
    }
    integer(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(text: &[u8]) -> io::Result<Pack> {
        parse(&content_reader_parser::parser::parse(text)?.items)
    }

    #[test]
    fn saved_sources_keep_order_signed_words_and_omitted_country_references() {
        let saved = pack(
            br#"speed=4
threat={
    threat={ threat=-1.71336 final_threat=0 daily=0 tag="ENG"
             date="1941.4.20.19" label="Peace - Treaty of Bangkok" }
    threat={ threat=90071992547.40993 final_threat=-90071992547.40993
             daily=0.00001 date="1936.1.1.1" label="" }
}"#,
        )
        .unwrap();
        assert_eq!(saved.speed, Some(4));
        let sources = saved.threat_sources.unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].ordinal, 0);
        assert_eq!(sources[0].tag.as_deref(), Some(b"ENG".as_slice()));
        assert_eq!(sources[0].threat.raw, -171_336);
        assert_eq!(sources[0].final_threat.raw, 0);
        assert_eq!(sources[0].date.engine_hour().unwrap(), {
            let date = GameDate {
                year: 1941,
                month: 4,
                day: 20,
                hour: 19,
            };
            date.engine_hour().unwrap()
        });
        assert_eq!(
            sources[0].label.as_deref(),
            Some(b"Peace - Treaty of Bangkok".as_slice())
        );
        assert_eq!(sources[1].ordinal, 1);
        assert_eq!(sources[1].tag, None);
        assert_eq!(sources[1].threat.raw, 9_007_199_254_740_993);
        assert_eq!(sources[1].final_threat.raw, -9_007_199_254_740_993);
        assert_eq!(sources[1].daily.raw, 1);
        assert_eq!(sources[1].label.as_deref(), Some(b"".as_slice()));
        assert!(sources[1].extra_keys.is_empty());
    }

    #[test]
    fn a_missing_block_is_not_an_empty_one_and_extra_keys_are_reported() {
        assert_eq!(pack(b"").unwrap(), Pack::default());
        let empty = pack(b"threat={}").unwrap();
        assert_eq!(empty.threat_sources, Some(Vec::new()));
        assert_eq!(empty.speed, None);
        let extra = pack(
            br#"threat={ threat={ threat=0 final_threat=0 daily=0 date="1936.1.1.1" other="X" } }"#,
        )
        .unwrap()
        .threat_sources
        .unwrap();
        assert_eq!(extra[0].extra_keys, vec![b"other".to_vec()]);
    }

    #[test]
    fn threat_targets_preserve_omitted_empty_and_positive_reference_text() {
        for (target, expected) in [
            ("", None),
            ("target=\"\"", Some(b"".as_slice())),
            ("target=\"TST\"", Some(b"TST".as_slice())),
        ] {
            let text = format!(
                "threat={{threat={{threat=0 final_threat=0 daily=0 date=1936.1.1.1 {target}}}}}"
            );
            let sources = pack(text.as_bytes()).unwrap().threat_sources.unwrap();
            assert_eq!(sources[0].target.as_deref(), expected);
            assert!(sources[0].extra_keys.is_empty());
        }
        for invalid in ["target={}", "target=TST target=OTH"] {
            let text = format!(
                "threat={{threat={{threat=0 final_threat=0 daily=0 date=1936.1.1.1 {invalid}}}}}"
            );
            assert!(pack(text.as_bytes()).is_err());
        }
    }

    #[test]
    fn malformed_speed_and_threat_entries_are_refused_without_substitution() {
        for text in [
            "speed=yes",
            "speed=1 speed=2",
            "threat={ speed=1 }",
            r#"threat={ threat={ final_threat=0 daily=0 date="1936.1.1.1" } }"#,
            r#"threat={ threat={ threat=0 daily=0 date="1936.1.1.1" } }"#,
            r#"threat={ threat={ threat=0 final_threat=0 date="1936.1.1.1" } }"#,
            "threat={ threat={ threat=0 final_threat=0 daily=0 } }",
            r#"threat={ threat={ threat=x final_threat=0 daily=0 date="1936.1.1.1" } }"#,
            r#"threat={ threat={ threat=0 final_threat=0 daily=0 date="1936.2.30.1" } }"#,
        ] {
            assert!(pack(text.as_bytes()).is_err(), "{text}");
        }
    }
}
