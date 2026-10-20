//! Typed operands shared by the management panel and prepared yard histories.
use replay_data::rows::{
    NavalRefitProgressRow, NavalRefitShipRow, ProductionLineRow, ShipModelRow, ShipRow,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hull {
    pub current_raw: Option<i64>,
    pub maximum_raw: Option<i64>,
}

/// Each operand is independently available. References retain all handle bits.
pub fn hull(ship: Option<&ShipRow>, model: Option<&ShipModelRow>) -> Hull {
    Hull {
        current_raw: ship.map(|row| row.strength),
        maximum_raw: model
            .and_then(|row| row.counter.as_ref())
            .map(|row| row.maximum_strength),
    }
}

pub struct Refit<'a> {
    /// None is unknown, Some(false) is a complete census not resolving the link.
    pub live: Option<bool>,
    pub progress: Option<&'a NavalRefitProgressRow>,
    /// Daily metadata never proves liveness.
    pub metadata: Option<&'a ProductionLineRow>,
}

pub fn refit<'a>(
    link: &NavalRefitShipRow,
    progress: Option<&'a [NavalRefitProgressRow]>,
    production: Option<&'a [ProductionLineRow]>,
) -> Refit<'a> {
    let amount = progress.and_then(|rows| link.resolve_line(rows));
    Refit {
        live: progress.map(|_| amount.is_some()),
        progress: amount,
        metadata: production.and_then(|rows| {
            rows.binary_search_by_key(&link.line, |row| row.handle)
                .ok()
                .map(|at| &rows[at])
        }),
    }
}
