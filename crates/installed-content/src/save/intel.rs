//! Exact serialized intel and diplomatic inputs. Missing payloads retain their
//! presence; native fields without a proved save mapping are not synthesized.

use std::{collections::BTreeMap, io};

use super::{
    Decimal, GameDate, block, boolean, bytes, decimal, field, integer, invalid, required_block,
    required_bytes, scalar,
};
use crate::parser::Item;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub countries: BTreeMap<Vec<u8>, Country>,
    pub global_variables: Option<Variables>,
    /// Preserve an absent states root separately from a present empty root,
    /// and each state's absent variables payload separately from an empty one.
    pub state_variables: Option<BTreeMap<u32, Option<Variables>>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub agency: Option<Agency>,
    pub diplomacy: Option<Diplomacy>,
    pub flags: Option<Vec<Flag>>,
    pub variables: Option<Variables>,
    pub operation_tokens: Option<OperationTokenManager>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agency {
    pub upgrade_progress: Decimal,
    pub defense: Decimal,
    pub active_count: i32,
    /// The save calls +0xF0 max_operative_count; its native meaning is unresolved.
    pub unknown_f0: u32,
    pub admission_ceiling: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diplomacy {
    pub subjects: Option<Vec<Subject>>,
    pub capitulated: Option<bool>,
    pub capitulated_date: Option<GameDate>,
    pub last_surrender_date: Option<GameDate>,
    pub faction_join_date: Option<GameDate>,
    pub legitimacy: Option<Decimal>,
    pub exile_army_leaders: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    pub roster_owner: Vec<u8>,
    pub target: Vec<u8>,
    pub first: Vec<u8>,
    pub second: Vec<u8>,
    pub state: Option<Vec<u8>>,
    pub autonomy_ratio: Decimal,
    pub start_date: Option<GameDate>,
    pub end_date: Option<GameDate>,
    pub cancel: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flag {
    pub name: Vec<u8>,
    pub value: i16,
    pub date: GameDate,
    /// CScriptFlag saves only positive durations. None does not imply -1 or 0.
    pub remaining_days: Option<i16>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Variables {
    /// Retain every repeated emitted name and value. The save representation
    /// does not itself establish the corresponding full native key identities.
    pub entries: Vec<Variable>,
    /// Token 0x27BB is reserved for CVariables random-state metadata. Its values
    /// are outside the recorded variable row; literal-key collisions are limited.
    pub reserved_entries: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variable {
    pub name: Vec<u8>,
    pub value: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationTokenManager {
    pub operation_assets: Option<Vec<OperationTokens>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationTokens {
    pub target: Vec<u8>,
    /// Retain serialized order and multiplicity. Native order is compared only
    /// as a multiset because the engine sorts a copy before writing this list.
    pub names: Vec<Vec<u8>>,
}

fn date(items: &[Item], key: &[u8]) -> io::Result<Option<GameDate>> {
    bytes(items, key)?
        .map(|value| GameDate::parse(&value))
        .transpose()
}

fn parse_agency(items: &[Item]) -> io::Result<Agency> {
    // CIntelligenceAgency::Save FCA0F0 omits +C8 only when its qword is zero;
    // +128, +F0 and +F4 are emitted unconditionally. Every active pointer in
    // +D8/+E4 produces one operative block, including repeated memberships.
    let active_count = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"operative"))
        .try_fold(0_i32, |count, item| {
            block(&item.value)?;
            count
                .checked_add(1)
                .ok_or_else(|| invalid("saved agency active count exceeds i32"))
        })?;
    Ok(Agency {
        upgrade_progress: decimal(items, b"upgrade_progress")?.unwrap_or(Decimal::parse(b"0")?),
        defense: Decimal::parse(&required_bytes(items, b"defense")?)?,
        active_count,
        unknown_f0: integer::<i32>(&required_bytes(items, b"max_operative_count")?)? as u32,
        admission_ceiling: integer(&required_bytes(items, b"usable_operative_slots")?)?,
    })
}

fn parse_subject(owner: &[u8], target: &[u8], items: &[Item]) -> io::Result<Subject> {
    // D195E0 writes stored definition and freedom, then the same base D19260
    // date envelope used by war relations. Dynamic current autonomy is separate.
    let cancel = field(items, b"cancel")?.map(boolean).transpose()?;
    if cancel == Some(false) {
        return Err(invalid(
            "subject relation explicitly saves an unset cancel flag",
        ));
    }
    let cancel = cancel.unwrap_or(false);
    let start_date = date(items, b"start_date")?;
    let end_date = date(items, b"end_date")?;
    if cancel == start_date.is_some() {
        return Err(invalid(
            "subject relation must save start_date exactly when not cancelled",
        ));
    }
    if let (Some(start), Some(end)) = (start_date, end_date)
        && start.engine_hour()? >= end.engine_hour()?
    {
        return Err(invalid("subject relation end_date is not after start_date"));
    }
    Ok(Subject {
        roster_owner: owner.to_vec(),
        target: target.to_vec(),
        first: required_bytes(items, b"first")?,
        second: required_bytes(items, b"second")?,
        state: bytes(items, b"autonomy_state")?,
        autonomy_ratio: decimal(items, b"value")?.unwrap_or(Decimal::parse(b"0")?),
        start_date,
        end_date,
        cancel,
    })
}

fn parse_diplomacy(owner: &[u8], items: &[Item]) -> io::Result<Diplomacy> {
    // D32BD0 emits both capitulation fields only for a nonzero native byte.
    let capitulated = field(items, b"capitulated")?.map(boolean).transpose()?;
    let capitulated_date = date(items, b"capitulated_date")?;
    if capitulated == Some(false) || capitulated.is_some() != capitulated_date.is_some() {
        return Err(invalid(
            "capitulation flag and date must be emitted together for a true flag",
        ));
    }
    let subjects = field(items, b"active_relations")?
        .map(|value| {
            let mut result = Vec::new();
            for relation in block(value)? {
                let target = relation
                    .key
                    .as_deref()
                    .filter(|tag| !tag.is_empty())
                    .ok_or_else(|| invalid("active relation lacks its country target"))?;
                if let Some(subject) = field(block(&relation.value)?, b"puppet")? {
                    result.push(parse_subject(owner, target, block(subject)?)?);
                }
            }
            Ok::<_, io::Error>(result)
        })
        .transpose()?;
    Ok(Diplomacy {
        subjects,
        capitulated,
        capitulated_date,
        last_surrender_date: date(items, b"last_surrender_date")?,
        faction_join_date: date(items, b"faction_join_date")?,
        legitimacy: decimal(items, b"legitimacy")?,
        exile_army_leaders: bytes(items, b"exile_army_leaders")?
            .map(|value| integer(&value))
            .transpose()?,
    })
}

fn parse_flags(items: &[Item]) -> io::Result<Vec<Flag>> {
    // CB0870 writes the resolved lexer name as the key; CB0970 writes the
    // signed word value, date, and positive-only remaining-days word.
    let mut result = Vec::new();
    for item in items {
        let name = item
            .key
            .clone()
            .ok_or_else(|| invalid("saved flag lacks its literal key"))?;
        let items = block(&item.value)?;
        let remaining_days = bytes(items, b"days")?
            .map(|value| integer::<i16>(&value))
            .transpose()?;
        if remaining_days.is_some_and(|days| days <= 0) {
            return Err(invalid("saved flag duration must be positive when present"));
        }
        result.push(Flag {
            name,
            value: integer(&required_bytes(items, b"value")?)?,
            date: GameDate::parse(&required_bytes(items, b"date")?)?,
            remaining_days,
        });
    }
    Ok(result)
}

fn parse_operation_tokens(items: &[Item]) -> io::Result<Vec<OperationTokens>> {
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        let target = item
            .key
            .clone()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("operation token list lacks target country"))?;
        if !seen.insert(target.clone()) {
            return Err(invalid("duplicate operation-token target country"));
        }
        let mut names = Vec::new();
        for name in block(&item.value)? {
            if name.key.is_some() {
                return Err(invalid("operation token list contains a keyed entry"));
            }
            names.push(scalar(&name.value)?.to_vec());
        }
        if names.is_empty() {
            return Err(invalid(
                "native operation-token writer omits empty target lists",
            ));
        }
        result.push(OperationTokens { target, names });
    }
    Ok(result)
}

fn parse_variables(items: &[Item]) -> io::Result<Variables> {
    // BB9D60 emits occupied-node values through 24ADC10 -> 24AF9A0 ->
    // 24B89F0 with exact fixed-five precision. BB9850 treats random specially.
    let mut variables = Variables::default();
    for item in items {
        let name = item
            .key
            .as_deref()
            .ok_or_else(|| invalid("saved variable lacks a literal key"))?;
        if name == b"random" {
            variables.reserved_entries += 1;
            continue;
        }
        variables.entries.push(Variable {
            name: name.to_vec(),
            value: Decimal::parse(scalar(&item.value)?)?,
        });
    }
    Ok(variables)
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    // CGameState::Save 1F2A20 and CState::Save 9D4040 use the same
    // variables token and CVariables serializer as CCountry::Save 70C580.
    let global_variables = field(items, b"variables")?
        .map(|value| parse_variables(block(value)?))
        .transpose()?;
    let state_variables = field(items, b"states")?
        .map(|value| {
            let mut states = BTreeMap::new();
            for item in block(value)? {
                let id = integer::<u32>(
                    item.key
                        .as_deref()
                        .ok_or_else(|| invalid("intel state lacks its numeric key"))?,
                )?;
                let variables = field(block(&item.value)?, b"variables")?
                    .map(|value| parse_variables(block(value)?))
                    .transpose()?;
                if states.insert(id, variables).is_some() {
                    return Err(invalid("duplicate state id in intel save pack"));
                }
            }
            Ok::<_, io::Error>(states)
        })
        .transpose()?;
    let mut countries = BTreeMap::new();
    for item in required_block(items, b"countries")? {
        let tag = item
            .key
            .as_deref()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("intel country lacks tag"))?;
        let items = block(&item.value)?;
        let country = Country {
            agency: field(items, b"intelligence_agency")?
                .map(|value| parse_agency(block(value)?))
                .transpose()?,
            diplomacy: field(items, b"diplomacy")?
                .map(|value| parse_diplomacy(tag, block(value)?))
                .transpose()?,
            flags: field(items, b"flags")?
                .map(|value| parse_flags(block(value)?))
                .transpose()?,
            variables: field(items, b"variables")?
                .map(|value| parse_variables(block(value)?))
                .transpose()?,
            operation_tokens: field(items, b"tokens")?
                .map(|value| {
                    Ok::<_, io::Error>(OperationTokenManager {
                        operation_assets: field(block(value)?, b"operation_assets")?
                            .map(|value| parse_operation_tokens(block(value)?))
                            .transpose()?,
                    })
                })
                .transpose()?,
        };
        if countries.insert(tag.to_vec(), country).is_some() {
            return Err(invalid("duplicate country tag in intel save pack"));
        }
    }
    Ok(Pack {
        countries,
        global_variables,
        state_variables,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(input: &[u8]) -> io::Result<Pack> {
        parse(&crate::parser::parse(input)?.items)
    }

    #[test]
    fn agency_keeps_qword_precision_signed_dwords_and_active_membership_count() {
        let pack = parsed(
            br#"countries={GER={intelligence_agency={
            defense=90071992547.40993 max_operative_count=-1 usable_operative_slots=-2147483648
            operative={id={id=4294967295 type=4713}} operative={id={id=4294967295 type=4713}}
        }} FRA={}}"#,
        )
        .unwrap();
        let agency = pack.countries[b"GER".as_slice()].agency.as_ref().unwrap();
        assert_eq!(agency.defense.raw, 9_007_199_254_740_993);
        assert_eq!(agency.upgrade_progress.raw, 0);
        assert_eq!((agency.active_count, agency.unknown_f0), (2, u32::MAX));
        assert_eq!(agency.admission_ceiling, i32::MIN);
        assert!(pack.countries[b"FRA".as_slice()].agency.is_none());
        assert!(parsed(b"countries={GER={intelligence_agency={max_operative_count=1 usable_operative_slots=2}}}").is_err());
    }

    #[test]
    fn scoped_subject_fields_and_omitted_envelopes_are_distinct() {
        let pack = parsed(
            br#"countries={GER={diplomacy={active_relations={FRA={puppet={
            first=GER second=FRA autonomy_state="puppet exact" value=-0.00001 start_date=1936.1.1.1
        }}}}} FRA={diplomacy={active_relations={}}}}"#,
        )
        .unwrap();
        let subject = &pack.countries[b"GER".as_slice()]
            .diplomacy
            .as_ref()
            .unwrap()
            .subjects
            .as_ref()
            .unwrap()[0];
        assert_eq!(subject.autonomy_ratio.raw, -1);
        assert_eq!(subject.state.as_deref(), Some(b"puppet exact".as_slice()));
        assert_eq!(subject.roster_owner, b"GER");
        assert_eq!(subject.target, b"FRA");
        assert!(subject.end_date.is_none());
        assert!(parsed(b"countries={GER={diplomacy={active_relations={FRA={puppet={first=GER second=FRA cancel=yes start_date=1936.1.1.1}}}}}}").is_err());
    }

    #[test]
    fn flags_keep_signed_words_literal_bytes_and_duration_presence() {
        let pack = parsed(b"countries={GER={flags={\"a\\x\"={value=-32768 date=1936.1.1.1} z={value=32767 date=1936.1.2.1 days=32767}}} FRA={flags={}} ENG={}}").unwrap();
        let flags = pack.countries[b"GER".as_slice()].flags.as_ref().unwrap();
        assert_eq!(flags[0].name, b"a\\x");
        assert_eq!(flags[0].value, i16::MIN);
        assert_eq!(flags[0].remaining_days, None);
        assert_eq!(flags[1].remaining_days, Some(i16::MAX));
        assert_eq!(pack.countries[b"FRA".as_slice()].flags, Some(vec![]));
        assert!(pack.countries[b"ENG".as_slice()].flags.is_none());
        assert!(parsed(b"countries={GER={flags={x={value=1 date=1936.1.1.1 days=0}}}}").is_err());
        assert!(parsed(b"countries={GER={flags={x={value=32768 date=1936.1.1.1}}}}").is_err());
        let repeated = parsed(b"countries={GER={flags={x={value=-1 date=1936.1.1.1} x={value=2 date=1936.1.2.1 days=3}}}}").unwrap();
        let flags = repeated.countries[b"GER".as_slice()]
            .flags
            .as_ref()
            .unwrap();
        assert_eq!(flags.len(), 2);
        assert_eq!(flags[0].name, flags[1].name);
        assert_eq!((flags[0].value, flags[1].value), (-1, 2));
    }

    #[test]
    fn operation_token_save_order_and_multiplicity_are_preserved() {
        let pack = parsed(b"countries={GER={tokens={operation_assets={FRA={z a a}}}}}").unwrap();
        let targets = pack.countries[b"GER".as_slice()]
            .operation_tokens
            .as_ref()
            .unwrap()
            .operation_assets
            .as_ref()
            .unwrap();
        assert_eq!(targets[0].target, b"FRA");
        assert_eq!(
            targets[0].names,
            [b"z".to_vec(), b"a".to_vec(), b"a".to_vec()]
        );
        assert!(parsed(b"countries={GER={tokens={operation_assets={FRA={}}}}}").is_err());
        assert!(parsed(b"countries={GER={tokens={operation_assets={FRA={x} FRA={y}}}}}").is_err());
    }

    #[test]
    fn conditional_payloads_keep_missing_empty_and_inactive_values_distinct() {
        let pack = parsed(b"countries={GER={tokens={}} FRA={tokens={operation_assets={}} diplomacy={exile_army_leaders=-2147483648}} ENG={diplomacy={capitulated=yes capitulated_date=1936.1.1.1 legitimacy=-0.00001 exile_army_leaders=0}}}").unwrap();
        assert!(
            pack.countries[b"GER".as_slice()]
                .operation_tokens
                .as_ref()
                .unwrap()
                .operation_assets
                .is_none()
        );
        assert_eq!(
            pack.countries[b"FRA".as_slice()]
                .operation_tokens
                .as_ref()
                .unwrap()
                .operation_assets,
            Some(vec![])
        );
        let inactive = pack.countries[b"FRA".as_slice()]
            .diplomacy
            .as_ref()
            .unwrap();
        assert_eq!(inactive.capitulated, None);
        assert_eq!(inactive.capitulated_date, None);
        assert_eq!(inactive.legitimacy, None);
        assert_eq!(inactive.exile_army_leaders, Some(i32::MIN));
        let active = pack.countries[b"ENG".as_slice()]
            .diplomacy
            .as_ref()
            .unwrap();
        assert_eq!(active.capitulated, Some(true));
        assert_eq!(active.legitimacy.as_ref().unwrap().raw, -1);
        assert!(parsed(b"countries={GER={diplomacy={capitulated=no}}}").is_err());
        assert!(parsed(b"countries={GER={diplomacy={capitulated=yes}}}").is_err());
        assert!(parsed(b"countries={GER={diplomacy={capitulated_date=1936.1.1.1}}}").is_err());
    }

    #[test]
    fn variables_preserve_repeated_literal_keys_and_exact_signed_values() {
        let pack = parsed(b"countries={GER={variables={random=1 a=90071992547.40993 a=-0.00001 \"\xff\"=-1.00001 \"\"=0 random=2}} FRA={variables={}} ENG={}}").unwrap();
        let variables = pack.countries[b"GER".as_slice()]
            .variables
            .as_ref()
            .unwrap();
        assert_eq!(variables.reserved_entries, 2);
        assert_eq!(variables.entries.len(), 4);
        assert_eq!(variables.entries[0].name, b"a");
        assert_eq!(variables.entries[1].name, b"a");
        assert_eq!(variables.entries[0].value.raw, 9_007_199_254_740_993);
        assert_eq!(variables.entries[1].value.raw, -1);
        assert_eq!(variables.entries[2].name, b"\xff");
        assert_eq!(variables.entries[2].value.raw, -100_001);
        assert!(variables.entries[3].name.is_empty());
        assert_eq!(
            pack.countries[b"FRA".as_slice()].variables,
            Some(Variables::default())
        );
        assert!(pack.countries[b"ENG".as_slice()].variables.is_none());
        assert!(parsed(b"countries={GER={variables={a={value=1}}}}").is_err());
    }

    #[test]
    fn variables_preserve_global_state_and_payload_scope_presence() {
        let pack = parsed(b"variables={same=-0.00001} countries={GER={variables={same=0.00002}}} states={0={variables={same=90071992547.40993}} 4294967295={variables={}} 7={}}").unwrap();
        assert_eq!(
            pack.global_variables.as_ref().unwrap().entries[0].value.raw,
            -1
        );
        assert_eq!(
            pack.countries[b"GER".as_slice()]
                .variables
                .as_ref()
                .unwrap()
                .entries[0]
                .value
                .raw,
            2
        );
        let states = pack.state_variables.as_ref().unwrap();
        assert_eq!(
            states[&0].as_ref().unwrap().entries[0].value.raw,
            9_007_199_254_740_993
        );
        assert_eq!(states[&u32::MAX], Some(Variables::default()));
        assert_eq!(states[&7], None);
        let absent = parsed(b"countries={}").unwrap();
        assert!(absent.global_variables.is_none());
        assert!(absent.state_variables.is_none());
        let empty = parsed(b"variables={} countries={} states={}").unwrap();
        assert_eq!(empty.global_variables, Some(Variables::default()));
        assert_eq!(empty.state_variables, Some(BTreeMap::new()));
        assert!(parsed(b"countries={} states={1={} 01={}}").is_err());
    }
}
