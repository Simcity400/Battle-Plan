use super::{block, bytes, field, integer, invalid, null_permitted_handle, scalar};
use content_reader_parser::parser::Item;
use replay_data::rows::GlobalSavedEventTargetRow as Row;
use std::io;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub name: Vec<u8>,
    pub state: i32,
    pub country: Option<Vec<u8>>,
    pub character: u32,
    pub present: u16,
    pub extra_keys: Vec<Vec<u8>>,
}
fn number(items: &[Item], key: &[u8]) -> io::Result<i32> {
    bytes(items, key)?
        .map(|b| integer(&b))
        .transpose()
        .map(|n| n.unwrap_or(0))
}
pub(super) fn parse(items: &[Item]) -> io::Result<Vec<Target>> {
    items
        .iter()
        .filter(|i| i.key.as_deref() == Some(b"saved_event_target"))
        .map(|i| {
            let b = block(&i.value)?;
            let saved_kinds = || {
                Row::KINDS
                    .into_iter()
                    .filter(|(mask, _)| *mask != Row::COMBATANT)
            };
            let mut extra_keys = Vec::new();
            for i in b {
                let k = i
                    .key
                    .as_deref()
                    .ok_or_else(|| invalid("keyless saved event target field"))?;
                if ![b"name".as_slice(), b"state", b"country"].contains(&k)
                    && !saved_kinds().any(|(_, kind)| kind.as_bytes() == k)
                {
                    extra_keys.push(k.to_vec());
                }
            }
            let mut present = 0;
            let mut character = 0;
            for (mask, kind) in saved_kinds() {
                let Some(value) = field(b, kind.as_bytes())? else {
                    continue;
                };
                if mask == Row::REGION {
                    integer::<i32>(scalar(value)?)?;
                } else {
                    let reference = null_permitted_handle(value)?;
                    if mask == Row::CHARACTER {
                        character = (reference >> 32) as u32;
                    }
                }
                present |= mask;
            }
            Ok(Target {
                name: bytes(b, b"name")?.unwrap_or_default(),
                state: number(b, b"state")?,
                country: bytes(b, b"country")?,
                character,
                present,
                extra_keys,
            })
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_names_order_and_omission_follow_writer() {
        let items=content_reader_parser::parser::parse(b"saved_event_target={ name=x country=AAA } saved_event_target={name=x state=3 character={id=8 type=54}} saved_event_target={}").unwrap();
        let rows = parse(&items.items).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].country.as_deref(), Some(b"AAA".as_slice()));
        assert_eq!(rows[1].state, 3);
        assert_eq!(rows[1].character, 8);
        assert_eq!(rows[1].present, 1);
        assert_eq!(rows[2], Target::default());
        assert!(parse(&[]).unwrap().is_empty());
    }
}
