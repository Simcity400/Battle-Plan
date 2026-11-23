use super::{GameDate, block, bytes, field, integer, invalid, null_permitted_handle, scalar};
use content_reader_parser::parser::{Item, Value};
use std::{collections::BTreeMap, io};

pub const REFERENCE_KEYS: [&[u8]; 9] = [
    b"character",
    b"operation",
    b"ace",
    b"unit",
    b"industrial_organisation",
    b"purchase_contract",
    b"raid_instance",
    b"project",
    b"faction",
];
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Definition {
    Name(Vec<u8>),
    Reference(u64),
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub country: Option<Vec<u8>>,
    pub state: Option<i32>,
    pub random: Option<[u32; 2]>,
    pub references: [Option<u64>; 9],
    pub strategic_region: Option<i32>,
    pub bindings: [Option<usize>; 3],
    pub saved_targets: Vec<super::saved_event_targets::Target>,
    pub extra_keys: Vec<Vec<u8>>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeGraph {
    pub nodes: Vec<Scope>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub identity: Option<Vec<u8>>,
    pub instance: Option<u32>,
    pub due: Option<GameDate>,
    pub scope: Option<ScopeGraph>,
    pub extra_keys: Vec<Vec<u8>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delayed {
    pub definition: Option<Definition>,
    pub hours: Option<i32>,
    pub days: Option<i32>,
    pub months: Option<i32>,
    pub originator: Option<Vec<u8>>,
    pub scope: Option<ScopeGraph>,
    pub extra_keys: Vec<Vec<u8>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    pub pending: Result<Option<Vec<Pending>>, String>,
    pub fired: Result<Option<Vec<Vec<u8>>>, String>,
    pub delayed: BTreeMap<Vec<u8>, Result<Vec<Delayed>, String>>,
    pub pending_extra: Vec<Vec<u8>>,
    pub fired_extra: Vec<Vec<u8>>,
}
impl Default for Pack {
    fn default() -> Self {
        Self {
            pending: Ok(None),
            fired: Ok(None),
            delayed: BTreeMap::new(),
            pending_extra: Vec::new(),
            fired_extra: Vec::new(),
        }
    }
}
fn extra(items: &[Item], allowed: &[&[u8]]) -> io::Result<Vec<Vec<u8>>> {
    items
        .iter()
        .map(|item| {
            item.key
                .as_ref()
                .ok_or_else(|| invalid("keyless retained event field"))
        })
        .filter_map(|key| match key {
            Ok(key) if allowed.contains(&key.as_slice()) => None,
            Ok(key) => Some(Ok(key.clone())),
            Err(error) => Some(Err(error)),
        })
        .collect()
}
fn number<T: std::str::FromStr>(items: &[Item], key: &[u8]) -> io::Result<Option<T>> {
    field(items, key)?
        .map(|value| integer(scalar(value)?))
        .transpose()
}
fn reference(value: &Value) -> io::Result<u64> {
    if !extra(block(value)?, &[b"id", b"type"])?.is_empty() {
        return Err(invalid("event scope reference has unknown fields"));
    }
    null_permitted_handle(value)
}
fn definition(value: &Value) -> io::Result<Definition> {
    match value {
        Value::Block(_) => reference(value).map(Definition::Reference),
        _ => scalar(value).map(|bytes| Definition::Name(bytes.to_vec())),
    }
}
pub fn scope(value: &Value) -> io::Result<ScopeGraph> {
    let mut nodes = vec![Scope::default()];
    let mut pending = vec![(0, block(value)?)];
    while let Some((index, items)) = pending.pop() {
        let random = field(items, b"random")?
            .map(|value| {
                let values = block(value)?;
                if values.len() != 2 || values.iter().any(|item| item.key.is_some()) {
                    return Err(invalid("event scope random needs two unkeyed words"));
                }
                Ok([
                    integer(scalar(&values[0].value)?)?,
                    integer(scalar(&values[1].value)?)?,
                ])
            })
            .transpose()?;
        let mut references = [None; 9];
        for (output, key) in references.iter_mut().zip(REFERENCE_KEYS) {
            *output = field(items, key)?.map(reference).transpose()?;
        }
        let mut bindings = [None; 3];
        for (binding, key) in bindings
            .iter_mut()
            .zip([b"root".as_slice(), b"from", b"prev"])
        {
            if let Some(value) = field(items, key)? {
                let id = nodes.len();
                nodes.push(Scope::default());
                *binding = Some(id);
                pending.push((id, block(value)?));
            }
        }
        let mut allowed = vec![
            b"country".as_slice(),
            b"state",
            b"random",
            b"strategic_region",
            b"root",
            b"from",
            b"prev",
            b"saved_event_target",
        ];
        allowed.extend(REFERENCE_KEYS);
        nodes[index] = Scope {
            country: bytes(items, b"country")?,
            state: number(items, b"state")?,
            random,
            references,
            strategic_region: number(items, b"strategic_region")?,
            bindings,
            saved_targets: super::saved_event_targets::parse(items)?,
            extra_keys: extra(items, &allowed)?,
        };
    }
    Ok(ScopeGraph { nodes })
}
fn pending(root: &[Item], output: &mut Pack) -> io::Result<Option<Vec<Pending>>> {
    let Some(value) = field(root, b"pending_events")? else {
        return Ok(None);
    };
    let items = block(value)?;
    let mut rows = Vec::new();
    output.pending_extra = extra(items, &[b"event"])?;
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"event"))
    {
        let fields = block(&item.value)?;
        rows.push(Pending {
            identity: bytes(fields, b"id")?,
            instance: number(fields, b"pending_id")?,
            due: field(fields, b"timeout")?
                .map(|value| GameDate::parse(scalar(value)?))
                .transpose()?,
            scope: field(fields, b"scope")?.map(scope).transpose()?,
            extra_keys: extra(fields, &[b"id", b"pending_id", b"timeout", b"scope"])?,
        });
    }
    Ok(Some(rows))
}
fn fired(root: &[Item], output: &mut Pack) -> io::Result<Option<Vec<Vec<u8>>>> {
    let Some(value) = field(root, b"fired_event_names")? else {
        return Ok(None);
    };
    let items = block(value)?;
    output.fired_extra = extra(items, &[b"id"])?;
    items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"id"))
        .map(|item| scalar(&item.value).map(<[u8]>::to_vec))
        .collect::<io::Result<_>>()
        .map(Some)
}
fn delayed(country: &Value) -> io::Result<Vec<Delayed>> {
    let mut rows = Vec::new();
    for item in block(country)?
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"delayed_event"))
    {
        let fields = block(&item.value)?;
        rows.push(Delayed {
            definition: field(fields, b"event")?.map(definition).transpose()?,
            hours: number(fields, b"hours")?,
            days: number(fields, b"days")?,
            months: number(fields, b"months")?,
            originator: bytes(fields, b"originator")?,
            scope: field(fields, b"scope")?.map(scope).transpose()?,
            extra_keys: extra(
                fields,
                &[
                    b"event",
                    b"hours",
                    b"days",
                    b"months",
                    b"originator",
                    b"scope",
                ],
            )?,
        });
    }
    Ok(rows)
}
pub fn parse(root: &[Item], countries: &[&Item]) -> io::Result<Pack> {
    let mut output = Pack::default();
    output.pending = pending(root, &mut output).map_err(|error| error.to_string());
    output.fired = fired(root, &mut output).map_err(|error| error.to_string());
    for country in countries {
        let tag = country
            .key
            .as_ref()
            .ok_or_else(|| invalid("keyless delayed-event country"))?;
        let rows = delayed(&country.value).map_err(|error| error.to_string());
        if output.delayed.insert(tag.clone(), rows).is_some() {
            return Err(invalid("duplicate unprojected delayed-event country"));
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_events_preserve_repeats_scope_branches_signed_components_and_absence() {
        let document=content_reader_parser::parser::parse(br#"
            fired_event_names={id=synthetic.1 id=synthetic.1 id=123}
            pending_events={event={id=synthetic.1 pending_id=7 timeout=1936.1.2.1 scope={country=AAA random={4 3} root={random={6 5}}}}}
            countries={AAA={delayed_event={event={id=13 type=42} hours=-3 days=-2 months=-1 scope={random={2 1}} originator=BBB}} BBB={}}
        "#).unwrap();
        let country_items = block(field(&document.items, b"countries").unwrap().unwrap())
            .unwrap()
            .iter()
            .collect::<Vec<_>>();
        let pack = parse(&document.items, &country_items).unwrap();
        assert_eq!(
            pack.fired.unwrap().unwrap(),
            [
                b"synthetic.1".to_vec(),
                b"synthetic.1".to_vec(),
                b"123".to_vec()
            ]
        );
        let pending = &pack.pending.unwrap().unwrap()[0];
        assert_eq!(pending.instance, Some(7));
        let scope = pending.scope.as_ref().unwrap();
        assert_eq!(scope.nodes[0].random, Some([4, 3]));
        assert_eq!(scope.nodes[0].bindings, [Some(1), None, None]);
        assert_eq!(scope.nodes[1].random, Some([6, 5]));
        let delayed = &pack.delayed[b"AAA".as_slice()].as_ref().unwrap()[0];
        assert_eq!(
            delayed.definition,
            Some(Definition::Reference((13_u64 << 32) | 42))
        );
        assert_eq!(
            (delayed.hours, delayed.days, delayed.months),
            (Some(-3), Some(-2), Some(-1))
        );
        assert!(pack.delayed[b"BBB".as_slice()].as_ref().unwrap().is_empty());
        let empty = parse(&[], &[]).unwrap();
        assert!(empty.pending.unwrap().is_none());
        assert!(empty.fired.unwrap().is_none());
    }
    #[test]
    fn one_unreadable_registry_or_country_queue_leaves_the_others_readable() {
        let document = content_reader_parser::parser::parse(
            br#"
            fired_event_names={id=synthetic.1}
            pending_events={event={id=synthetic.1 pending_id=malformed}}
            countries={AAA={delayed_event={hours=malformed}} BBB={delayed_event={hours=1}}}
        "#,
        )
        .unwrap();
        let country_items = block(field(&document.items, b"countries").unwrap().unwrap())
            .unwrap()
            .iter()
            .collect::<Vec<_>>();
        let pack = parse(&document.items, &country_items).unwrap();
        assert!(pack.pending.is_err());
        assert_eq!(pack.fired.unwrap().unwrap(), [b"synthetic.1".to_vec()]);
        assert!(pack.delayed[b"AAA".as_slice()].is_err());
        let sibling = pack.delayed[b"BBB".as_slice()].as_ref().unwrap();
        assert_eq!(sibling[0].hours, Some(1));
    }
}
