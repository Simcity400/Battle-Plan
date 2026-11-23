use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRepairCountryRow {
    pub country: u32,
    pub max_factories_for_repair: i32,
    pub max_allowed_dockyards_to_repair: i32,
    pub num_used_dockyards: i32,
}
typed_codec!(NavalRepairCountryRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (max_factories_for_repair): I32, 1 => (max_allowed_dockyards_to_repair): I32, 2 => (num_used_dockyards): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRepairFleetRow {
    pub fleet: u64,
    pub automated_homebase: u8,
}
typed_codec!(NavalRepairFleetRow, 64, |row| row.fleet as u128, |key| Self { fleet: key as u64, ..Default::default() },
 { 0 => (automated_homebase): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRepairTaskForceRow {
    pub task_force: u64,
    pub repair_mode: i32,
    pub repair_split: u8,
    pub repair_target: u32,
    pub auto_reinforcement: u8,
    pub refit_variant: u64,
    pub refit_after_repair: u64,
    pub composition_token: u32,
    pub merge_after_repair: u64,
}
typed_codec!(NavalRepairTaskForceRow, 64, |row| row.task_force as u128, |key| Self { task_force: key as u64, ..Default::default() },
 { 0 => (repair_mode): I32, 1 => (repair_split): U8, 2 => (repair_target): U32, 3 => (auto_reinforcement): U8, 4 => (refit_variant): U64, 5 => (refit_after_repair): U64, 6 => (composition_token): U32, 7 => (merge_after_repair): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalTaskForceCompositionRow {
    pub task_force: u64,
    pub kind: u8,
    pub ordinal: u32,
    pub sub_unit: i32,
    pub role: i32,
    pub definition: i32,
    pub amount: i32,
}
typed_codec!(NavalTaskForceCompositionRow, 104, |row| (row.task_force as u128) << 40 | (row.kind as u128) << 32 | row.ordinal as u128, |key| Self { task_force: (key >> 40) as u64, kind: (key >> 32) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (sub_unit): I32, 1 => (role): I32, 2 => (definition): I32, 3 => (amount): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRepairBaseRow {
    pub province: u32,
    pub priority: i32,
    pub allocated_dockyards: i32,
}
typed_codec!(NavalRepairBaseRow, 32, |row| row.province as u128, |key| Self { province: key as u32, ..Default::default() },
 { 0 => (priority): I32, 1 => (allocated_dockyards): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRepairQueueRow {
    pub province: u32,
    pub ordinal: u32,
    pub ship: u64,
}
typed_codec!(NavalRepairQueueRow, 64, |row| (row.province as u128) << 32 | row.ordinal as u128, |key| Self { province: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (ship): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRefitShipRow {
    pub ship: u64,
    pub line: u64,
}
typed_codec!(NavalRefitShipRow, 64, |row| row.ship as u128, |key| Self { ship: key as u64, ..Default::default() },
 { 0 => (line): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRefitProgressRow {
    pub line: u64,
    pub progress_raw: i64,
}
typed_codec!(NavalRefitProgressRow, 64, |row| row.line as u128, |key| Self { line: key as u64, ..Default::default() },
 { 0 => (progress_raw): I64 }, |_row| Ok(()));

impl NavalRefitShipRow {
    pub fn resolve_line<'a>(
        &self,
        lines: &'a [NavalRefitProgressRow],
    ) -> Option<&'a NavalRefitProgressRow> {
        if self.line == 0 {
            return None;
        }
        lines
            .binary_search_by_key(&self.line, |row| row.line)
            .ok()
            .map(|index| &lines[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn round_trip<T: RowCodec + std::fmt::Debug + PartialEq>(row: T) {
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let decoded = T::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded, row);
    }
    #[test]
    fn refit_resolution_uses_complete_hourly_production_identity() {
        let link = NavalRefitShipRow {
            ship: 1,
            line: 71 | (7_u64 << 32),
        };
        let live = [NavalRefitProgressRow {
            line: link.line,
            progress_raw: 0,
        }];
        assert_eq!(link.resolve_line(&live), Some(&live[0]));
        assert_eq!(link.resolve_line(&[]), None);
        for replacement in [link.line + 1, link.line + (1_u64 << 32)] {
            assert_eq!(
                link.resolve_line(&[NavalRefitProgressRow {
                    line: replacement,
                    progress_raw: 123
                }]),
                None
            );
        }
        assert_eq!(
            NavalRefitShipRow { ship: 1, line: 0 }.resolve_line(&live),
            None
        );
    }

    #[test]
    fn naval_management_codecs_preserve_native_widths_and_order_keys() {
        round_trip(NavalRepairCountryRow {
            country: u32::MAX,
            max_factories_for_repair: i32::MIN,
            max_allowed_dockyards_to_repair: i32::MIN,
            num_used_dockyards: i32::MIN,
        });
        round_trip(NavalRepairFleetRow {
            fleet: u64::MAX,
            automated_homebase: u8::MAX,
        });
        round_trip(NavalRepairTaskForceRow {
            task_force: u64::MAX,
            repair_mode: i32::MIN,
            repair_split: u8::MAX,
            repair_target: u32::MAX,
            auto_reinforcement: u8::MAX,
            refit_variant: u64::MAX,
            refit_after_repair: u64::MAX,
            composition_token: u32::MAX,
            merge_after_repair: u64::MAX,
        });
        round_trip(NavalTaskForceCompositionRow {
            task_force: u64::MAX,
            kind: u8::MAX,
            ordinal: u32::MAX,
            sub_unit: i32::MIN,
            role: i32::MIN,
            definition: i32::MIN,
            amount: i32::MIN,
        });
        round_trip(NavalRepairBaseRow {
            province: u32::MAX,
            priority: i32::MIN,
            allocated_dockyards: i32::MIN,
        });
        round_trip(NavalRepairQueueRow {
            province: u32::MAX,
            ordinal: u32::MAX,
            ship: u64::MAX,
        });
        round_trip(NavalRefitShipRow {
            ship: u64::MAX,
            line: u64::MAX,
        });
        round_trip(NavalRepairChildRow {
            task_force: u64::MAX,
            ordinal: u32::MAX,
            child: u64::MAX,
        });
        round_trip(NavalRefitProgressRow {
            line: u64::MAX,
            progress_raw: i64::MIN,
        });
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRepairChildRow {
    pub task_force: u64,
    pub ordinal: u32,
    pub child: u64,
}
typed_codec!(NavalRepairChildRow, 96, |row| (row.task_force as u128) << 32 | row.ordinal as u128,
    |key| Self { task_force: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
    { 0 => (child): U64 }, |_row| Ok(()));
