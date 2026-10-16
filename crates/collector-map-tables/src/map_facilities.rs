//! Hourly naval-base map inputs beside the bases themselves: the country supply systems'
//! floating-harbour deployment records and the special-facility flags of program provinces.
use crate::{
    clock::qpc,
    table_capture::CaptureRows,
    table_conversion::{table, table_iter, unavailable},
};
use engine::allocation::collect_vec;
use engine::{Memory, ccountry_supply_system, special_facility_flags, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{FacilityMapFlagRow, FloatingHarborDeploymentRow, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::FloatingHarborDeployments,
    TableId::FacilityMapFlags,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    deployments: usize,
    facilities: usize,
}

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let deployments = collector_extended_support::demand::rows(
            ccountry_supply_system::walk_floating_harbor_deployments(memory, base, state, &mut []),
        );
        let facilities = collector_extended_support::demand::rows(special_facility_flags::walk(
            memory,
            base,
            state,
            &mut [],
        ));
        if let Ok(count) = deployments {
            crate::country_scale::raise(&mut self.deployments, count);
        }
        if let Ok(count) = facilities {
            crate::country_scale::raise(&mut self.facilities, count);
        }
        deployments.and(facilities).map(|_| ())
    }
}

pub struct Captured {
    deployments: CaptureRows<ccountry_supply_system::FloatingHarborDeployment>,
    facilities: CaptureRows<special_facility_flags::Row>,
}

impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            deployments: 0,
            facilities: 0,
        })
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            deployments: CaptureRows::new(c.deployments),
            facilities: CaptureRows::new(c.facilities),
        }
    }

    pub fn reset(&mut self) {
        self.deployments.reset();
        self.facilities.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.deployments.allocated_bytes() + self.facilities.allocated_bytes()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        self.deployments.grow(&mut c.deployments);
        self.facilities.grow(&mut c.facilities);
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let begin = qpc();
        self.deployments.outcome = {
            let _reads = crate::read_accounting::table(TABLES[0]);
            ccountry_supply_system::walk_floating_harbor_deployments_growing(
                memory,
                base,
                state,
                &mut self.deployments.rows,
            )
        };
        let middle = qpc();
        self.facilities.outcome = {
            let _reads = crate::read_accounting::table(TABLES[1]);
            special_facility_flags::walk_growing(memory, base, state, &mut self.facilities.rows)
        };
        [
            middle.saturating_sub(begin).max(0) as u64,
            qpc().saturating_sub(middle).max(0) as u64,
        ]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.deployments.outcome, self.facilities.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        let [deployments, facilities] = self.outcomes();
        // Two programs can name one province; its flag is one byte, so the first reading stands.
        let mut flagged: Vec<&special_facility_flags::Row> = match collect_vec(
            self.facilities
                .rows
                .get(..facilities.len())
                .unwrap_or_default()
                .iter(),
        ) {
            Ok(flagged) => flagged,
            Err(_) => {
                return [
                    unavailable("collector allocation failed"),
                    unavailable("collector allocation failed"),
                ];
            }
        };
        // Owned row addresses preserve the first captured occurrence without sort scratch.
        flagged.sort_unstable_by_key(|row| (row.province, std::ptr::from_ref(*row) as usize));
        flagged.dedup_by_key(|row| row.province);
        let facilities = match facilities {
            Outcome::Rows { .. } => Outcome::Rows { len: flagged.len() },
            Outcome::Truncated { unread_rows, .. } => Outcome::Truncated {
                len: flagged.len(),
                unread_rows,
            },
            unavailable => unavailable,
        };
        [
            table::<tables::FloatingHarborDeployments, _>(
                deployments,
                &self.deployments.rows,
                |r| FloatingHarborDeploymentRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    province: r.province,
                    status: r.status,
                },
            ),
            table_iter::<tables::FacilityMapFlags, _>(facilities, flagged.into_iter(), |r| {
                FacilityMapFlagRow {
                    province: r.province,
                    flag: r.flag,
                }
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn repeated_facility_provinces_collapse_and_deployments_convert() {
        let mut capture = Captured::with_capacities(&Capacities {
            deployments: 1,
            facilities: 3,
        });
        capture.deployments.rows[0] = ccountry_supply_system::FloatingHarborDeployment {
            country: 2,
            ordinal: 1,
            province: -1,
            status: 3,
        };
        capture.deployments.outcome = Outcome::Rows { len: 1 };
        for (index, province) in [9_u32, 4, 9].into_iter().enumerate() {
            capture.facilities.rows[index] = special_facility_flags::Row { province, flag: 1 };
        }
        capture.facilities.outcome = Outcome::Rows { len: 3 };
        let [
            TableView::Complete(TableRows::FloatingHarborDeployments(deployments)),
            TableView::Complete(TableRows::FacilityMapFlags(flags)),
        ] = capture.tables()
        else {
            panic!("complete tables");
        };
        assert_eq!((deployments[0].province, deployments[0].status), (-1, 3));
        assert_eq!(
            flags.iter().map(|row| row.province).collect::<Vec<_>>(),
            [4, 9]
        );
        capture.facilities.outcome = Outcome::Unavailable { reason: "test" };
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
    }
    #[test]
    fn repeated_facility_province_keeps_first_captured_flag() {
        let count = 256;
        let mut captured = Captured::with_capacities(&Capacities {
            deployments: 0,
            facilities: count,
        });
        for (index, row) in captured.facilities.rows.iter_mut().enumerate() {
            *row = special_facility_flags::Row {
                province: if index % 3 == 0 { 7 } else { 3 },
                flag: index as u8,
            };
        }
        captured.facilities.outcome = Outcome::Rows { len: count };
        let TableView::Complete(TableRows::FacilityMapFlags(rows)) = &captured.tables()[1] else {
            panic!("facility flags unavailable");
        };
        assert_eq!(
            rows.iter()
                .map(|row| (row.province, row.flag))
                .collect::<Vec<_>>(),
            [(3, 1), (7, 0)]
        );
    }
}
