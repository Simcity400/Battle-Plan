use crate::{Decimal, block, boolean, bytes, field, integer, invalid, scalar};
use content_reader_parser::parser::{Item, Value};
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reference {
    pub country: Option<Vec<u8>>,
    pub pool: i32,
    pub id: i32,
    pub discriminant: u32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keyed {
    pub target: i32,
    pub values: [i64; 4],
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub id: i32,
    pub removed: bool,
    pub values: Vec<Keyed>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StaticPool {
    pub next_id: i32,
    pub sources: Vec<Source>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DynamicPool {
    pub pending: Vec<Keyed>,
    pub accumulated: Vec<Keyed>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub allied: Vec<[i64; 4]>,
    pub cryptology: Vec<[i64; 4]>,
    pub contributions: [Vec<Vec<Keyed>>; 3],
    pub statics: Vec<StaticPool>,
    pub dynamics: Vec<DynamicPool>,
    pub discriminant: Option<u32>,
    pub cheat: Option<Reference>,
}
pub fn reference(value: &Value) -> io::Result<Reference> {
    let fields = block(value)?;
    Ok(Reference {
        country: bytes(fields, b"country")?,
        pool: number(fields, b"pool")?.unwrap_or(0),
        id: number(fields, b"id")?.ok_or_else(|| invalid("intel source reference lacks id"))?,
        discriminant: number(fields, b"discriminant")?
            .ok_or_else(|| invalid("intel source reference lacks discriminant"))?
            as u32,
    })
}
fn number(fields: &[Item], name: &[u8]) -> io::Result<Option<i32>> {
    field(fields, name)?
        .map(|value| integer(scalar(value)?))
        .transpose()
}
fn lanes(value: &Value) -> io::Result<[i64; 4]> {
    let fields = block(value)?;
    let mut values = [0; 4];
    for (index, name) in [b"civilian".as_slice(), b"army", b"navy", b"airforce"]
        .into_iter()
        .enumerate()
    {
        values[index] = field(fields, name)?
            .map(|value| Decimal::parse(scalar(value)?).map(|value| value.raw))
            .transpose()?
            .unwrap_or(0);
    }
    for item in fields {
        if ![b"civilian".as_slice(), b"army", b"navy", b"airforce"]
            .contains(&item.key.as_deref().unwrap_or_default())
        {
            return Err(invalid("intel source has an unknown lane"));
        }
    }
    Ok(values)
}
fn list<'a>(fields: &'a [Item], name: &[u8]) -> io::Result<&'a [Item]> {
    field(fields, name)?
        .map(block)
        .transpose()
        .map(|items| items.unwrap_or(&[]))
}
fn dense(fields: &[Item], name: &[u8]) -> io::Result<Vec<[i64; 4]>> {
    list(fields, name)?
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("intel dense source list has a keyed occurrence"));
            }
            lanes(&item.value)
        })
        .collect()
}
fn keyed(value: &Value, named: bool) -> io::Result<Vec<Keyed>> {
    let mut output = Vec::new();
    for item in block(value)? {
        let (target, value) = if named {
            (
                integer(
                    item.key
                        .as_deref()
                        .ok_or_else(|| invalid("intel source target lacks numeric key"))?,
                )?,
                &item.value,
            )
        } else {
            if item.key.is_some() {
                return Err(invalid("intel contribution occurrence must be anonymous"));
            }
            let pair = block(&item.value)?;
            if pair.len() != 2 || pair.iter().any(|item| item.key.is_some()) {
                return Err(invalid(
                    "intel contribution needs a target and four-lane block",
                ));
            }
            (integer(scalar(&pair[0].value)?)?, &pair[1].value)
        };
        output.push(Keyed {
            target,
            values: lanes(value)?,
        });
    }
    Ok(output)
}
fn keyed_field(fields: &[Item], name: &[u8], named: bool) -> io::Result<Vec<Keyed>> {
    field(fields, name)?
        .map(|value| keyed(value, named))
        .transpose()
        .map(Option::unwrap_or_default)
}
pub fn parse(value: &Value) -> io::Result<Sources> {
    let fields = block(value)?;
    let mut output = Sources {
        allied: dense(fields, b"intel_from_allies")?,
        cryptology: dense(fields, b"intel_from_encryption_decryption")?,
        discriminant: number(fields, b"discriminant")?.map(|value| value as u32),
        cheat: field(fields, b"cheat")?.map(reference).transpose()?,
        ..Default::default()
    };
    for (index, name) in [
        b"intel_from_static_pools".as_slice(),
        b"intel_from_dynamic_pools",
        b"intel_from_dynamic_pools_prev_day",
    ]
    .into_iter()
    .enumerate()
    {
        for item in list(fields, name)? {
            if item.key.is_some() {
                return Err(invalid("intel contribution pool must be positional"));
            }
            output.contributions[index].push(keyed(&item.value, false)?);
        }
    }
    for item in list(fields, b"static_intel_pools")? {
        if item.key.is_some() {
            return Err(invalid("static intel pool must be positional"));
        }
        let fields = block(&item.value)?;
        let mut pool = StaticPool {
            next_id: number(fields, b"id")?.unwrap_or(0),
            ..Default::default()
        };
        for item in list(fields, b"intel")? {
            if item.key.is_some() {
                return Err(invalid("static intel source must be positional"));
            }
            let fields = block(&item.value)?;
            pool.sources.push(Source {
                id: number(fields, b"id")?
                    .ok_or_else(|| invalid("static intel source lacks id"))?,
                removed: field(fields, b"remove")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
                values: keyed_field(fields, b"intel", true)?,
            });
        }
        output.statics.push(pool);
    }
    for item in list(fields, b"dynamic_intel_pools")? {
        if item.key.is_some() {
            return Err(invalid("dynamic intel pool must be positional"));
        }
        let fields = block(&item.value)?;
        output.dynamics.push(DynamicPool {
            pending: keyed_field(fields, b"accumulator", false)?,
            accumulated: keyed_field(fields, b"values", false)?,
        });
    }
    Ok(output)
}

pub fn owner_reference(fields: &[Item], holder: &[u8]) -> io::Result<Option<Reference>> {
    match field(fields, holder)? {
        Some(value) => field(block(value)?, b"intel_source")?
            .map(reference)
            .transpose(),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intel_sources_preserve_native_pool_order_duplicate_keys_signed_lanes_and_empty_extents() {
        let parsed=content_reader_parser::parser::parse(br#"intel={
            intel_from_allies={ {} {army=-1.00001} {} }
            intel_from_static_pools={ { {-7 {}} {-7 {civilian=2}} } {} }
            static_intel_pools={ {id=-3 intel={ {id=-17 remove=yes intel={ -9={} -9={navy=-3} }} }} {} }
            dynamic_intel_pools={ {} { values={ {-2 {}} } accumulator={ {4 {airforce=0.00001}} } } }
            cheat={id=-7 discriminant=-1} discriminant=-2
        }"#).unwrap();
        let output = parse(&parsed.items[0].value).unwrap();
        assert_eq!(output.allied, [[0; 4], [0, -100001, 0, 0], [0; 4]]);
        assert_eq!(output.contributions[0][0].len(), 2);
        assert!(output.contributions[0][1].is_empty());
        assert_eq!(output.statics[0].next_id, -3);
        assert_eq!(
            output.statics[0].sources[0].values[1].values,
            [0, 0, -300000, 0]
        );
        assert_eq!(output.dynamics[1].pending[0].values, [0, 0, 0, 1]);
        assert_eq!(output.dynamics[1].accumulated[0].values, [0; 4]);
        assert_eq!(output.cheat.unwrap().discriminant, u32::MAX);
        assert_eq!(output.discriminant, Some(u32::MAX - 1));
    }
}
