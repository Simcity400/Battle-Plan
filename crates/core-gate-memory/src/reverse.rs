use crate::{
    manifest::{Operand, Relation, invalid},
    memory::cell,
};
use replay::columns;
use replay_columnar::{manifest::Status, state::Cell, walk::State};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Value {
    Number(u128),
    Text(Box<[u8]>),
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    value: Value,
    row: u128,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Size {
    pub edges: u64,
    pub bytes: u64,
    pub statuses: Vec<String>,
}

pub struct Index {
    edges: Vec<Edge>,
    statuses: Vec<Option<Status>>,
    pub size: Size,
}

impl Index {
    pub fn new(relation: &Relation) -> Self {
        Self {
            edges: Vec::new(),
            statuses: vec![None; relation.steps.len()],
            size: Size::default(),
        }
    }

    pub fn update(&mut self, relation: &Relation, state: &State, restore: bool) -> io::Result<()> {
        let mut changed = restore;
        for (at, (id, operand)) in relation.steps.iter().enumerate() {
            let id = *id;
            let status = columns::status(state, id);
            let changes = columns::changes(state, id)?;
            changed |= self.statuses[at] != Some(status)
                || changes.truncated
                || !changes.inserted.is_empty()
                || !changes.deleted.is_empty()
                || match operand {
                    Operand::Field(field) => changes
                        .written
                        .iter()
                        .chain(&changes.fill_masks)
                        .any(|mask| mask & (1_u64 << field) != 0),
                    Operand::Key(_) => false,
                };
            self.statuses[at] = Some(status);
        }
        if !changed {
            return Ok(());
        }
        self.edges.clear();
        let (source, _) = relation.steps[0];
        let table = columns::table(state, source)?;
        let slots: Box<dyn Iterator<Item = u32> + '_> = match columns::status(state, source) {
            Status::Complete => Box::new(table.live().ones()),
            Status::Truncated { .. } => Box::new(
                columns::truncated(state, source)
                    .ok_or_else(|| invalid("Truncated reverse input has no view"))?
                    .iter()
                    .copied(),
            ),
            _ => Box::new(std::iter::empty()),
        };
        for slot in slots {
            let row = table
                .key(slot)
                .ok_or_else(|| invalid("Reverse input has no live key"))?;
            let mut values = operands(state, relation, 0, slot)?;
            for step in 1..relation.steps.len() {
                let (id, _) = relation.steps[step];
                let table = columns::table(state, id)?;
                let mut next = Vec::new();
                for value in values {
                    let Value::Number(key) = value else {
                        return Err(invalid(format!(
                            "{} joins through a nonnumeric key",
                            relation.name
                        )));
                    };
                    if let Some(slot) = table.slot(key) {
                        if !visible(state, id, slot) {
                            continue;
                        }
                        next.extend(operands(state, relation, step, slot)?);
                    }
                }
                values = next;
            }
            self.edges
                .extend(values.into_iter().map(|value| Edge { value, row }));
        }
        self.edges.sort_unstable();
        self.edges.dedup();
        self.size = Size {
            edges: self.edges.len() as u64,
            bytes: (self.edges.capacity() * size_of::<Edge>()) as u64
                + self
                    .edges
                    .iter()
                    .map(|edge| match &edge.value {
                        Value::Text(text) => text.len() as u64,
                        _ => 0,
                    })
                    .sum::<u64>(),
            statuses: self
                .statuses
                .iter()
                .map(|status| format!("{status:?}"))
                .collect(),
        };
        Ok(())
    }
}

fn visible(state: &State, id: replay_data::rows::TableId, slot: u32) -> bool {
    match columns::status(state, id) {
        Status::Complete => true,
        Status::Truncated { .. } => {
            columns::truncated(state, id).is_some_and(|view| view.binary_search(&slot).is_ok())
        }
        _ => false,
    }
}

fn operands(state: &State, relation: &Relation, step: usize, slot: u32) -> io::Result<Vec<Value>> {
    let (id, ref operand) = relation.steps[step];
    let table = columns::table(state, id)?;
    let field = match operand {
        Operand::Key(shift) => {
            return Ok(vec![Value::Number(
                table
                    .key(slot)
                    .ok_or_else(|| invalid("Reverse key is not live"))?
                    >> shift,
            )]);
        }
        Operand::Field(field) => *field,
    };
    if !table.recorded(field, slot) {
        return Ok(Vec::new());
    }
    let value = cell(table, field, slot)?;
    let number = match value {
        Cell::U8(v) => Some(v.into()),
        Cell::I8(v) => Some(v as u128),
        Cell::U16(v) => Some(v.into()),
        Cell::I16(v) => Some(v as u128),
        Cell::U32(v) => Some(v.into()),
        Cell::I32(v) => Some(v as u128),
        Cell::U64(v) => Some(v.into()),
        Cell::I64(v) => Some(v as u128),
        Cell::Bool(v) => Some(u128::from(v)),
        Cell::OptionalI32(v) => v.map(|v| v as u128),
        Cell::OptionalU32(v) => v.map(Into::into),
        Cell::OptionalI64(v) => v.map(|v| v as u128),
        Cell::OptionalU64(v) => v.map(Into::into),
        Cell::U32List(v) => return Ok(v.iter().map(|&v| Value::Number(v.into())).collect()),
        Cell::U64List(v) => return Ok(v.iter().map(|&v| Value::Number(v.into())).collect()),
        Cell::Bytes(id) => {
            let mut scratch = None;
            let text = state
                .dictionary()
                .resolve(id, &mut scratch)?
                .ok_or_else(|| invalid("Reverse input names a missing dictionary string"))?;
            return Ok(vec![Value::Text(text.into())]);
        }
        Cell::Opaque(_) => {
            return Err(invalid(format!(
                "{} requires a declared opaque relation decoder",
                relation.name
            )));
        }
    };
    Ok(number.map(Value::Number).into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_columnar::{
        restart::{self, Acquisition},
        state::{Bits, Table, Values},
    };
    use replay_data::rows::TableId;

    fn restore(state: &mut State, id: TableId, key: u128, field: &str, value: u32, unknown: bool) {
        let schema = replay::columnar_frames::schema().unwrap();
        let mut table = Table::with_rows(
            &schema[id.index()],
            1,
            Bits::from_words(vec![1]).unwrap(),
            &[key],
        )
        .unwrap();
        let f = columns::field(id, field).unwrap();
        table.set(f, &[0], Values::U32(&[value])).unwrap();
        if unknown {
            table.unrecord(f, &[0]).unwrap();
        }
        state
            .restore(
                id.index(),
                &restart::write(&table, Status::Complete, &Acquisition::default(), &[]).unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn a_link_change_moves_an_unchanged_owner_and_unknown_removes_it() {
        let mut state = State::new(replay::columnar_frames::schema().unwrap()).unwrap();
        restore(&mut state, TableId::States, 9, "owner", 7, false);
        restore(
            &mut state,
            TableId::CountryLinks,
            7,
            "canonical_slot",
            11,
            false,
        );
        let relation = Relation {
            name: "synthetic ownership".into(),
            steps: vec![
                (TableId::States, Operand::Field(0)),
                (TableId::CountryLinks, Operand::Field(0)),
            ],
        };
        let mut index = Index::new(&relation);
        index.update(&relation, &state, true).unwrap();
        assert_eq!(index.edges[0].value, Value::Number(11));
        restore(
            &mut state,
            TableId::CountryLinks,
            7,
            "canonical_slot",
            12,
            false,
        );
        index.update(&relation, &state, true).unwrap();
        assert_eq!(index.edges[0].value, Value::Number(12));
        restore(
            &mut state,
            TableId::CountryLinks,
            7,
            "canonical_slot",
            12,
            true,
        );
        index.update(&relation, &state, true).unwrap();
        assert_eq!(index.size.edges, 0);
    }
}
