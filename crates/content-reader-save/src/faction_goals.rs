use crate::{SavedDate, block, bytes, field, integer, invalid, scalar};
use content_reader_parser::parser::Item;
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub manifest: Option<Vec<u8>>,
    pub goals: Vec<Goal>,
    pub completed: Vec<Vec<u8>>,
    pub cancelled: Vec<Vec<u8>>,
    pub extra_slots: BTreeMap<Vec<u8>, i32>,
    pub term_dates: Vec<i32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Goal {
    pub token: Vec<u8>,
    pub status: Vec<u8>,
    pub date: i32,
}
fn dates(value: &[Item]) -> io::Result<Vec<i32>> {
    value
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("faction goal dates must be an ordinary list"));
            }
            SavedDate::parse(scalar(&item.value)?)?.engine_hour()
        })
        .collect()
}
fn names(items: &[Item], key: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let Some(value) = field(items, key)? else {
        return Ok(Vec::new());
    };
    block(value)?
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("faction goal history must be an ordinary list"));
            }
            Ok(scalar(&item.value)?.to_vec())
        })
        .collect()
}
pub(crate) fn parse(items: &[Item]) -> io::Result<Status> {
    let Some(value) = field(items, b"goal_status")? else {
        return Ok(Status::default());
    };
    let fields = block(value)?;
    let mut result = Status {
        manifest: bytes(fields, b"manifest")?,
        completed: names(fields, b"completed_goals")?,
        cancelled: names(fields, b"canceled_goals")?,
        ..Default::default()
    };
    if let Some(value) = field(fields, b"goals")? {
        for item in block(value)? {
            if item.key.is_some() {
                return Err(invalid("faction goals must be anonymous blocks"));
            }
            let goal = block(&item.value)?;
            result.goals.push(Goal {
                token: bytes(goal, b"goal")?
                    .ok_or_else(|| invalid("faction goal has no definition"))?,
                status: bytes(goal, b"status")?
                    .ok_or_else(|| invalid("faction goal has no status"))?,
                date: SavedDate::parse(scalar(
                    field(goal, b"game_data")?
                        .ok_or_else(|| invalid("faction goal has no date"))?,
                )?)?
                .engine_hour()?,
            });
        }
    }
    if let Some(value) = field(fields, b"extra_goal_slots")? {
        for item in block(value)? {
            let term = item
                .key
                .as_ref()
                .ok_or_else(|| invalid("faction goal slot has no term"))?;
            if result
                .extra_slots
                .insert(term.clone(), integer(scalar(&item.value)?)?)
                .is_some()
            {
                return Err(invalid("duplicate faction goal slot term"));
            }
        }
    }
    if let Some(value) = field(fields, b"game_data")? {
        result.term_dates = dates(block(value)?)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn faction_goal_save_keeps_order_repeated_history_signed_slots_and_unset_dates() {
        let doc = content_reader_parser::parser::parse(br#"goal_status={manifest=mod_manifest goals={{goal=mod_goal status=active game_data=-1.1.1.1}{goal=other status=canceled game_data=1940.2.3.4}} completed_goals={mod_goal mod_goal} canceled_goals={other} extra_goal_slots={short_term=-2 medium_term=0 long_term=7} game_data={-1.1.1.1 1940.2.3.4}}"#).unwrap();
        let status = parse(&doc.items).unwrap();
        assert_eq!(status.manifest.as_deref(), Some(b"mod_manifest".as_slice()));
        assert_eq!(
            status.completed,
            [b"mod_goal".to_vec(), b"mod_goal".to_vec()]
        );
        assert_eq!(status.goals[0].date, SavedDate::INVALID_HOUR);
        assert_eq!(status.goals[1].status, b"canceled");
        assert_eq!(
            status.term_dates,
            [status.goals[0].date, status.goals[1].date]
        );
        assert_eq!(status.extra_slots[b"short_term".as_slice()], -2);
        assert_eq!(status.extra_slots[b"long_term".as_slice()], 7);
    }
}
