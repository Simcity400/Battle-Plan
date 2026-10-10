//! One frame's borrowed directed-war owner admission for selected movement,
//! attack and transport. Local selection does not select recording demand.
//! Each Owner holds exactly this immutable frame/world/hour, admits once on
//! first reached direct predicate, and cannot be reassigned to a later frame.

use replay::{
    frame::DecodedFrame,
    rows::{DirectedWarContext, ReservedReverseVisibilityInputs, tables},
};
use std::sync::OnceLock;

type Fact<T> = Result<T, String>;

enum Context<'a> {
    Positive(DirectedWarContext<'a>),
    ActualZero(&'a ReservedReverseVisibilityInputs),
}

impl Context<'_> {
    fn pointer_active(&self, target_slot: u32) -> Fact<bool> {
        match self {
            Self::Positive(context) => context.pointer_active(target_slot),
            Self::ActualZero(packet) => {
                let cell=packet.cells.get(target_slot as usize).and_then(Option::as_ref)
                    .ok_or("ReservedCountryNaming.reverse_visibility cell capture refused or outside observed matrix")?;
                // Native source authenticates ordinal + status owner. Active
                // war intentionally returns before target/subject reads, so
                // absent cell.target must not erase a known independent war.
                match cell
                    .war
                    .ok_or("ReservedCountryNaming.reverse_visibility war capture refused")?
                {
                    [0, 0] => Ok(false),
                    [1, retired] => Ok(retired == 0),
                    _ => {
                        Err("ReservedCountryNaming.reverse_visibility war presence invalid".into())
                    }
                }
            }
        }
    }
}

/// A narrowly scoped borrowed admission, not a persistent world/query cache.
/// First direct demand records admission failure as failure for later targets;
/// no retry, hour reassignment or stale carried packet is possible.
pub(super) struct Owner<'a> {
    frame: &'a DecodedFrame,
    actual_raw_id: i32,
    canonical_slot: u32,
    context: OnceLock<Fact<Context<'a>>>,
}

fn canonical(frame: &DecodedFrame, raw: u32) -> Fact<u32> {
    if raw == 0 {
        return Ok(0);
    }
    let mut rows = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.id == raw);
    let row = rows.next().ok_or("directed-war raw country link missing")?;
    if rows.next().is_some() {
        return Err("directed-war raw country link duplicated".into());
    }
    Ok(row.canonical_slot)
}

impl<'a> Owner<'a> {
    pub(super) fn new(frame: &'a DecodedFrame, receiver_raw: u32) -> Fact<Self> {
        let canonical_slot = canonical(frame, receiver_raw)?;
        let actual_raw_id = if canonical_slot == 0 {
            0
        } else {
            let mut rows = frame
                .complete::<tables::Countries>()
                .map_err(|e| e.to_string())?
                .iter()
                .filter(|row| row.canonical_slot == canonical_slot);
            let row = rows
                .next()
                .ok_or("directed-war receiver Countries row missing")?;
            if rows.next().is_some() {
                return Err("directed-war receiver Countries row duplicated".into());
            }
            row.id as i32
        };
        Ok(Self {
            frame,
            actual_raw_id,
            canonical_slot,
            context: OnceLock::new(),
        })
    }

    fn admit(&self) -> Fact<Context<'a>> {
        if self.canonical_slot == 0 {
            let rows = self
                .frame
                .complete::<tables::ReservedCountryNaming>()
                .map_err(|e| e.to_string())?;
            let [row] = rows else {
                return Err(
                    "directed-war actual-zero naming singleton missing or duplicated".into(),
                );
            };
            let packet = row
                .reverse_visibility
                .require("ReservedCountryNaming.reverse_visibility")?
                .as_ref()
                .ok_or("ReservedCountryNaming.reverse_visibility capture refused")?;
            if packet.sample_hour != self.frame.hour {
                return Err("ReservedCountryNaming.reverse_visibility sample hour mismatch".into());
            }
            if packet.cells.len() > 65_536 {
                return Err(
                    "ReservedCountryNaming.reverse_visibility matrix domain invalid".into(),
                );
            }
            return Ok(Context::ActualZero(packet));
        }
        let mut rows = self
            .frame
            .complete::<tables::CountryCounterInputs>()
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|row| row.country == self.actual_raw_id as u32);
        let row = rows
            .next()
            .ok_or("directed-war receiver CountryCounterInputs row missing")?;
        if rows.next().is_some() {
            return Err("directed-war receiver CountryCounterInputs row duplicated".into());
        }
        let packet = row
            .directed_wars
            .require("CountryCounterInputs.directed_wars")?
            .as_ref()
            .ok_or("CountryCounterInputs.directed_wars capture refused")?;
        packet
            .admit(self.frame.hour, self.actual_raw_id, self.canonical_slot)
            .map(Context::Positive)
    }

    /// Direct relation-byte predicate without positive-receiver or self veto.
    /// Callers with an already resolved exact target use pointer_active_slot.
    pub(super) fn pointer_active(&self, target_raw: u32) -> Fact<bool> {
        self.pointer_active_slot(canonical(self.frame, target_raw)?)
    }

    pub(super) fn pointer_active_slot(&self, target_slot: u32) -> Fact<bool> {
        match self.context.get_or_init(|| self.admit()) {
            Ok(context) => context.pointer_active(target_slot),
            Err(reason) => Err(reason.clone()),
        }
    }

    /// Raw identity, then both-nonzero alias identity.
    pub(super) fn same_country(&self, target_raw: u32) -> Fact<bool> {
        if target_raw as i32 == self.actual_raw_id {
            return Ok(true);
        }
        Ok(target_raw != 0
            && self.actual_raw_id != 0
            && canonical(self.frame, target_raw)? == self.canonical_slot)
    }

    pub(super) fn slot(&self) -> u32 {
        self.canonical_slot
    }

    /// Raw-self, both-nonzero alias-self, then the signed-positive gate.
    /// Those proved false arms cannot acquire unrelated packet admission.
    pub(super) fn at_war(&self, target_raw: u32) -> Fact<bool> {
        if self.same_country(target_raw)? {
            return Ok(false);
        }
        if self.actual_raw_id <= 0 {
            return Ok(false);
        }
        self.pointer_active(target_raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{DirectedWars, ReverseZeroVisibilityInputs, WarCell};
    #[test]
    fn actual_zero_active_cell_survives_native_skipped_target_read() {
        let packet = ReservedReverseVisibilityInputs {
            sample_hour: 12,
            cells: vec![
                Some(ReverseZeroVisibilityInputs {
                    target: None,
                    war: Some([1, 0]),
                    ..Default::default()
                }),
                None,
                Some(ReverseZeroVisibilityInputs {
                    war: Some([0, 0]),
                    ..Default::default()
                }),
            ],
        };
        let context = Context::ActualZero(&packet);
        assert_eq!(context.pointer_active(0), Ok(true));
        assert!(context.pointer_active(1).is_err());
        assert_eq!(context.pointer_active(2), Ok(false));
        assert!(context.pointer_active(3).is_err());
    }
    #[test]
    fn direct_positive_alias_to_zero_remains_a_direct_cell() {
        let packet = DirectedWars {
            sample_hour: 12,
            receiver_raw_id: 99,
            receiver_slot: 1,
            matrix_count: 3,
            cells: vec![WarCell {
                target_slot: 0,
                active: true,
            }],
            ..Default::default()
        };
        let context = Context::Positive(packet.admit(12, 99, 1).unwrap());
        assert_eq!(context.pointer_active(0), Ok(true));
        assert!(context.pointer_active(2).is_err());
    }
    #[test]
    fn corrupt_actual_zero_presence_never_becomes_clear_war() {
        for war in [[0, 8], [2, 0]] {
            let packet = ReservedReverseVisibilityInputs {
                sample_hour: 12,
                cells: vec![Some(ReverseZeroVisibilityInputs {
                    war: Some(war),
                    ..Default::default()
                })],
            };
            assert!(Context::ActualZero(&packet).pointer_active(0).is_err());
        }
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            CountryCounterInputRow, CountryLinkRow, CountryRow, DirectedWars, Recorded,
            ReservedCountryNamingRow, ReverseZeroVisibilityInputs, TableId, TableRows, WarCell,
        },
    };

    fn frame(packet: Recorded<Option<DirectedWars>>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 12,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "undemanded fixture table".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        };
        frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 7,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 12,
                    canonical_slot: 0,
                },
                CountryLinkRow {
                    id: 20,
                    canonical_slot: 2,
                },
                CountryLinkRow {
                    id: 99,
                    canonical_slot: 1,
                },
            ]))
            .into();
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 99,
                canonical_slot: 1,
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 99,
                    directed_wars: packet,
                    ..Default::default()
                },
            ]))
            .into();
        frame
    }

    fn packet(hour: i32, active: bool) -> DirectedWars {
        DirectedWars {
            sample_hour: hour,
            receiver_raw_id: 99,
            receiver_slot: 1,
            matrix_count: 3,
            cells: vec![WarCell {
                target_slot: 2,
                active,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn owner_resolves_actual_raw_row_and_raw_self_gates_skip_admission() {
        let frame = frame(Recorded::Value(Some(packet(12, true))));
        let owner = Owner::new(&frame, 7).unwrap();
        assert_eq!(owner.actual_raw_id, 99);
        assert_eq!(owner.canonical_slot, 1);
        assert_eq!(owner.at_war(99), Ok(false));
        assert_eq!(owner.at_war(7), Ok(false));
        assert!(owner.context.get().is_none());
        assert_eq!(owner.at_war(20), Ok(true));
        assert!(matches!(
            owner.context.get(),
            Some(Ok(Context::Positive(_)))
        ));
    }

    #[test]
    fn owner_zero_alias_uses_existing_independent_war_and_skips_country_rows() {
        let mut frame = frame(Recorded::Value(None));
        frame.tables[TableId::Countries.index()] = TableView::Unavailable {
            reason: "zero must not demand positive Countries".into(),
        }
        .into();
        frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    reverse_visibility: Recorded::Value(Some(ReservedReverseVisibilityInputs {
                        sample_hour: 12,
                        cells: vec![Some(ReverseZeroVisibilityInputs {
                            target: None,
                            war: Some([1, 0]),
                            ..Default::default()
                        })],
                    })),
                    ..Default::default()
                },
            ]))
            .into();
        let owner = Owner::new(&frame, 12).unwrap();
        assert_eq!(owner.actual_raw_id, 0);
        // The signed receiver gate returns false without resolving an unknown target.
        assert_eq!(owner.at_war(123_456), Ok(false));
        assert!(owner.context.get().is_none());
        assert_eq!(owner.pointer_active_slot(0), Ok(true));
        assert!(owner.pointer_active_slot(1).is_err());
    }

    #[test]
    fn owner_retains_first_stale_packet_failure_for_later_direct_queries() {
        let frame = frame(Recorded::Value(Some(packet(11, true))));
        let owner = Owner::new(&frame, 7).unwrap();
        let first = owner.pointer_active_slot(2).unwrap_err();
        let cached = match owner.context.get().unwrap() {
            Err(reason) => reason as *const String,
            Ok(_) => panic!("stale packet unexpectedly admitted"),
        };
        assert_eq!(owner.pointer_active_slot(0), Err(first));
        let later = match owner.context.get().unwrap() {
            Err(reason) => reason as *const String,
            Ok(_) => panic!("failed admission unexpectedly replaced"),
        };
        assert_eq!(cached, later);
    }

    #[test]
    fn owner_keeps_historical_absence_and_current_refusal_unknown() {
        for packet in [Recorded::NotRecorded, Recorded::Value(None)] {
            let frame = frame(packet);
            let owner = Owner::new(&frame, 7).unwrap();
            assert_eq!(owner.at_war(99), Ok(false));
            assert!(owner.context.get().is_none());
            assert!(owner.pointer_active_slot(2).is_err());
            assert!(matches!(owner.context.get(), Some(Err(_))));
        }
    }

    #[test]
    fn owner_rejects_packet_receiver_identity_mismatch() {
        for (receiver_raw_id, receiver_slot) in [(7, 1), (99, 2)] {
            let mut packet = packet(12, true);
            packet.receiver_raw_id = receiver_raw_id;
            packet.receiver_slot = receiver_slot;
            let frame = frame(Recorded::Value(Some(packet)));
            let owner = Owner::new(&frame, 7).unwrap();
            assert!(owner.pointer_active_slot(2).is_err());
        }
    }

    #[test]
    fn new_owner_after_backward_seek_does_not_reuse_other_frames_admission() {
        let current = frame(Recorded::Value(Some(packet(12, true))));
        let current_owner = Owner::new(&current, 7).unwrap();
        assert_eq!(current_owner.pointer_active_slot(2), Ok(true));
        let mut previous = frame(Recorded::Value(Some(packet(11, false))));
        previous.hour = 11;
        previous.branch = 2;
        let previous_owner = Owner::new(&previous, 7).unwrap();
        assert!(previous_owner.context.get().is_none());
        assert_eq!(previous_owner.pointer_active_slot(2), Ok(false));
        assert_eq!(current_owner.pointer_active_slot(2), Ok(true));
    }
}
