//! A yard view's rows in one comparable form: each owner by name, each token by its spelling.
use crate::products::yards::{
    project::{Key, Projector, Spelled},
    segment::{Read, View},
};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Canon {
    Null,
    Integer(i128),
    Unsigned(u64),
    Token(String),
    List(Vec<Canon>),
}

/// Each row by its owner, stream and key: every cell after its key and whether it is held.
pub(crate) type Rows = BTreeMap<(String, u64, Canon), Vec<Canon>>;

pub(crate) fn canon(cell: &Spelled) -> Canon {
    match cell {
        Spelled::Null => Canon::Null,
        Spelled::Integer(value) => Canon::Integer(*value),
        Spelled::Unsigned(value) => Canon::Unsigned(*value),
        Spelled::Token(label) => Canon::Token(label.key.clone()),
        Spelled::List(cells) => Canon::List(cells.iter().map(canon).collect()),
    }
}

fn key(key: &Key) -> Canon {
    match key {
        Key::Number(number) => Canon::Unsigned(*number),
        Key::Token(label) => Canon::Token(label.key.clone()),
    }
}

/// Every row `frame` holds of `V` projected whole; none where its owners do not resolve.
pub(crate) fn whole<V: View>(frame: &DecodedFrame) -> Option<Rows> {
    let (_, rows) = Projector::build(frame, V::REFITS).unwrap()?;
    Some(
        rows.rows
            .into_iter()
            .map(|row| {
                let place = (rows.owners[row.owner as usize].clone(), row.stream, key(&row.key));
                (place, row.cells.unwrap().iter().map(canon).collect())
            })
            .collect(),
    )
}

/// `held` as `read` leaves it: what it changed, every row where it gives them whole, and none
/// where its owners do not resolve.
pub(crate) fn follow(held: &mut Option<Rows>, read: &Read) {
    let Some(rows) = &read.rows else {
        *held = None;
        return;
    };
    let state = held.get_or_insert_with(Rows::new);
    if rows.whole {
        state.clear();
    }
    for row in &rows.rows {
        let place = (
            rows.owners[row.owner as usize].clone(),
            row.stream,
            key(&row.key),
        );
        match &row.cells {
            Some(cells) => {
                state.insert(place, cells.iter().map(canon).collect());
            }
            None => {
                state.remove(&place);
            }
        }
    }
}
