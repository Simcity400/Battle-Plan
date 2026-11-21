
use std::io;

use super::{Input, put_bytes, put_u32};
use crate::{
    ResolvedLayers,
    equipment::Database,
    unit_leader::needs::SubUnitNeeds,
};

pub const VERSION: u32 = 1;

pub fn gather(layers: &ResolvedLayers) -> Result<Vec<u8>, String> {
    let database = Database::load_complete(layers).map_err(|error| format!("the equipment database cannot be read: {error}"))?;
    let needs = SubUnitNeeds::read(layers, &database).map_err(|error| format!("the sub-unit needs cannot be read: {error}"))?;
    encode_needs(&needs)
}

pub fn encode_needs(needs: &SubUnitNeeds) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut write = || -> io::Result<()> {
        put_u32(&mut out, super::count(needs.needs.len())?);
        for (key, need) in &needs.needs {
            put_bytes(&mut out, key.as_bytes())?;
            put_u32(&mut out, super::count(need.len())?);
            for (archetype, amount) in need {
                put_bytes(&mut out, archetype.as_bytes())?;
                out.extend_from_slice(&amount.to_le_bytes());
            }
        }
        put_u32(&mut out, super::count(needs.dropped.len())?);
        for dropped in &needs.dropped {
            put_bytes(&mut out, dropped.as_bytes())?;
        }
        Ok(())
    };
    write().map_err(|error| format!("the sub-unit needs cannot be packed: {error}"))?;
    Ok(out)
}

pub fn decode_needs(bytes: &[u8]) -> io::Result<SubUnitNeeds> {
    let mut input = Input(bytes);
    let mut out = SubUnitNeeds::default();
    for _ in 0..counted(&mut input)? {
        let key = input.string()?;
        let mut need = Vec::new();
        for _ in 0..counted(&mut input)? {
            let archetype = input.string()?;
            need.push((archetype, i64::from_le_bytes(input.take(8)?.try_into().expect("eight bytes"))));
        }
        out.needs.insert(key, need);
    }
    for _ in 0..counted(&mut input)? {
        out.dropped.push(input.string()?);
    }
    finished(&input)?;
    Ok(out)
}

fn counted(input: &mut Input<'_>) -> io::Result<usize> {
    let count = input.u32()? as usize;
    if count > input.0.len() / 4 + 1 {
        return Err(super::invalid("an equipment fact counts more entries than it holds"));
    }
    Ok(count)
}

fn finished(input: &Input<'_>) -> io::Result<()> {
    if input.0.is_empty() { Ok(()) } else { Err(super::invalid("an equipment fact runs past its entries")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn needs() -> SubUnitNeeds {
        SubUnitNeeds {
            needs: [
                ("infantry".to_owned(), vec![("infantry_equipment".to_owned(), 10_000_000), ("support_equipment".to_owned(), 50_000)]),
                ("engineer".to_owned(), Vec::new()),
            ]
            .into(),
            dropped: vec!["sub-unit A needs ghost, which is no equipment".to_owned()],
        }
    }

    #[test]
    fn sub_unit_needs_decode_to_what_was_encoded_and_encode_the_same_bytes() {
        let needs = needs();
        let bytes = encode_needs(&needs).unwrap();
        assert_eq!(bytes, encode_needs(&needs).unwrap());
        assert_eq!(decode_needs(&bytes).unwrap(), needs);
        assert!(decode_needs(&bytes[..bytes.len() - 1]).is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode_needs(&longer).is_err());
    }
}
