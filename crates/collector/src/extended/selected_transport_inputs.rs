//! Pure projections of hourly native operands. No live pointers survive here.
use engine::selected_transport_units as native;
use replay::rows::{UnitTransferInputs, UnitTransportInputs};

pub(super) fn transfer(value: native::Transfer) -> UnitTransferInputs {
    match value {
        native::Transfer::Absent => UnitTransferInputs::Absent,
        native::Transfer::Present {
            handle,
            status,
            source,
        } => UnitTransferInputs::Present {
            transfer: handle,
            status: status.ok(),
            source: source.ok(),
        },
    }
}

pub(super) fn project(row: &native::Row) -> UnitTransportInputs {
    UnitTransportInputs {
        sample_hour: row.sample_hour,
        weight: row.weight.ok(),
        transfer: row.transfer.ok().map(transfer),
        coastal_template: row.coastal.ok(),
    }
}

#[derive(Default)]
pub(super) struct Work {
    captured: u64,
    refusals: Vec<(&'static str, u64)>,
}
impl Work {
    fn refuse(&mut self, reason: &'static str) -> std::io::Result<()> {
        let index = match self.refusals.binary_search_by_key(&reason, |entry| entry.0) {
            Ok(index) => index,
            Err(index) => {
                engine::allocation::reserve(&mut self.refusals, 1)?;
                self.refusals.insert(index, (reason, 0));
                index
            }
        };
        self.refusals[index].1 = self.refusals[index].1.saturating_add(1);
        Ok(())
    }
    pub(super) fn observe(&mut self, packet: &native::Packet) -> std::io::Result<()> {
        if let engine::walk::Outcome::Unavailable { reason } = packet.outcome() {
            return self.refuse(reason);
        }
        for row in packet.rows() {
            self.observe_row(row)?;
        }
        Ok(())
    }
    fn observe_row(&mut self, row: &native::Row) -> std::io::Result<()> {
        for failure in [row.weight.err(), row.coastal.err(), row.transfer.err()]
            .into_iter()
            .flatten()
        {
            self.refuse(failure)?;
        }
        if let Ok(native::Transfer::Present { status, source, .. }) = row.transfer {
            if let Err(reason) = status {
                self.refuse(reason)?;
            }
            if status == Ok(0)
                && let Err(reason) = source
            {
                self.refuse(reason)?;
            }
        }
        self.captured = self.captured.saturating_add(1);
        Ok(())
    }
    pub(super) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({ "captured": self.captured,
            "refusals": crate::status::map(self.refusals.iter().map(|&(reason, count)| (reason, count))),
            "scope": "same-pass hourly selected-unit transport inputs; independent failures retain the established movement prefix" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(status: native::Fact<i32>, source: native::Fact<u32>) -> native::Row {
        native::Row {
            unit: u64::MAX,
            sample_hour: -17,
            weight: Err("original weight refused"),
            coastal: Ok(false),
            transfer: Ok(native::Transfer::Present {
                handle: u64::MAX,
                status,
                source,
            }),
        }
    }
    #[test]
    fn projection_keeps_independent_failures_exact_signed_status_and_null() {
        assert_eq!(
            project(&row(Ok(-3), Err("unreached source"))),
            UnitTransportInputs {
                sample_hour: -17,
                weight: None,
                coastal_template: Some(false),
                transfer: Some(UnitTransferInputs::Present {
                    transfer: u64::MAX,
                    status: Some(-3),
                    source: None
                }),
            }
        );
        let mut value = row(Err("status refused"), Err("unreached source"));
        assert_eq!(
            project(&value).transfer,
            Some(UnitTransferInputs::Present {
                transfer: u64::MAX,
                status: None,
                source: None
            })
        );
        value.transfer = Ok(native::Transfer::Absent);
        assert_eq!(project(&value).transfer, Some(UnitTransferInputs::Absent));
        value.transfer = Err("identity refused");
        assert_eq!(project(&value).transfer, None);
        assert_eq!(project(&value).coastal_template, Some(false));
    }
    #[test]
    fn named_counts_include_only_reached_siblings_and_repeated_failed_occurrences() {
        let mut work = Work::default();
        work.observe_row(&row(Ok(-3), Err("unreached source")))
            .unwrap();
        work.observe_row(&row(Err("status refused"), Err("unreached source")))
            .unwrap();
        work.observe_row(&row(Ok(0), Err("source refused")))
            .unwrap();
        assert_eq!(work.captured, 3);
        assert_eq!(
            work.refusals,
            vec![
                ("original weight refused", 3),
                ("source refused", 1),
                ("status refused", 1)
            ]
        );
        work.observe(&native::Packet::vacant()).unwrap();
        assert_eq!(work.captured, 3);
        assert!(work.refusals.iter().any(|(reason, count)| *reason
            == "selected transport units not acquired"
            && *count == 1));
    }
}
