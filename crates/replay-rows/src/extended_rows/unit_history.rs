use super::*;
use std::sync::Arc;

pub type SharedDivisionHistoryRow = Arc<DivisionHistoryRow>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionHistoryRow {
    pub division: u64,
    pub list: u8,
    pub ordinal: u32,
    pub available: u8,
    pub reason: Vec<u8>,
    pub amount: i32,
    pub name: Vec<u8>,
    pub custom_name: Vec<u8>,
    pub kind: i32,
    pub target_country: u32,
    pub hour: i32,
    pub medal_token: Option<i32>,
    pub medal_definition: Option<i32>,
    pub medal_enabled: u8,
    pub medal_count: u8,
    pub inherit: u8,
    pub multiplier: i64,
    pub orders: i32,
    pub provinces: Vec<u32>,
    pub sunk_ship: Option<SunkShipEntry>,
}
impl DivisionHistoryRow {
    pub const SUMMARY: u8 = 0;
    pub const CAREER: u8 = 1;
    pub const AWARDS: u8 = 2;
}
typed_codec!(DivisionHistoryRow, 98, |row| ((row.division as u128) << 34) | ((row.list as u128) << 32) | row.ordinal as u128,
    |key| Self { division: (key >> 34) as u64, list: ((key >> 32) & 3) as u8, ordinal: key as u32, ..Default::default() }, {
        0 => (available): U8, 1 => (reason): Bytes, 2 => (amount): I32, 3 => (name): Bytes,
        4 => (custom_name): Bytes, 5 => (kind): I32, 6 => (target_country): U32, 7 => (hour): I32,
        8 => (medal_token): OptionalI32, 9 => (medal_definition): OptionalI32, 10 => (medal_count): U8,
        11 => (inherit): U8, 12 => (multiplier): I64, 13 => (orders): I32,
        14 => (provinces): U32List, 15 => (sunk_ship): Opaque, 16 => (medal_enabled): U8
    }, |_row| Ok(()));

impl Fields for SharedDivisionHistoryRow {
    const SCHEMA: &'static [Kind] = DivisionHistoryRow::SCHEMA;
    const STEPS: &'static [Option<Step>] = DivisionHistoryRow::STEPS;
    const NAMES: &'static [&'static str] = DivisionHistoryRow::NAMES;
}
impl RowCodec for SharedDivisionHistoryRow {
    const KEY_BITS: u32 = DivisionHistoryRow::KEY_BITS;
    fn try_clone(&self) -> io::Result<Self> {
        self.check_key_fields()?;
        if self.key() >> Self::KEY_BITS != 0 {
            return Err(invalid("division history key exceeds table width"));
        }
        Ok(Arc::clone(self))
    }
    fn try_clone_validated(&self) -> io::Result<Self> {
        Ok(Arc::clone(self))
    }
    fn heap_bytes(&self) -> usize {
        std::mem::size_of::<DivisionHistoryRow>() + self.as_ref().heap_bytes()
    }
    fn key(&self) -> u128 {
        self.as_ref().key()
    }
    fn encoded_bound(&self) -> io::Result<usize> {
        self.as_ref().encoded_bound()
    }
    fn snapshot_bytes(&self) -> io::Result<usize> {
        self.as_ref().snapshot_bytes()
    }
    fn check_key_fields(&self) -> io::Result<()> {
        self.as_ref().check_key_fields()
    }
    fn not_recorded_fields(&self) -> u64 {
        self.as_ref().not_recorded_fields()
    }
    fn differing_fields(&self, previous: &Self) -> u64 {
        if Arc::ptr_eq(self, previous) {
            0
        } else {
            self.as_ref().differing_fields(previous)
        }
    }
    fn visit_nested_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
        self.as_ref().visit_nested_field_gaps(visit)
    }
    fn visit_wire(
        &self,
        visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.as_ref().visit_wire(visit)
    }
    fn recorded_fields_bytes(&self) -> io::Result<usize> {
        self.as_ref().recorded_fields_bytes()
    }
    fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>> {
        self.as_ref().delta(previous.map(AsRef::as_ref))
    }
    fn encode_change_reserved(
        &self,
        previous: Option<&Self>,
        previous_key: u128,
        output: &mut Vec<u8>,
    ) -> io::Result<bool> {
        self.as_ref()
            .encode_change_reserved(previous.map(AsRef::as_ref), previous_key, output)
    }
    fn encode_fields_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.as_ref().encode_fields_reserved(output)
    }
    fn decode_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self> {
        DivisionHistoryRow::decode_fields(key, input).map(Arc::new)
    }
    fn from_wire<'a>(
        key: u128,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<Self> {
        DivisionHistoryRow::from_wire(key, wire).map(Arc::new)
    }
    fn patch_from_wire<'a>(
        &mut self,
        mask: u64,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<bool> {
        if let Some(row) = Arc::get_mut(self) {
            return row.patch_from_wire(mask, wire);
        }
        let mut row = self.as_ref().try_clone()?;
        let patched = row.patch_from_wire(mask, wire)?;
        if patched {
            *self = Arc::new(row);
        }
        Ok(patched)
    }
    fn apply_delta(delta: &DeltaRow, previous: Option<Self>) -> io::Result<Self> {
        let previous = previous
            .map(|row| match Arc::try_unwrap(row) {
                Ok(row) => Ok(row),
                Err(row) => row.as_ref().try_clone(),
            })
            .transpose()?;
        DivisionHistoryRow::apply_delta(delta, previous).map(Arc::new)
    }
    fn has_display_steps(&self) -> bool {
        false
    }
    fn round_to_display(&mut self) {}
}
impl crate::checkpoint::rows::Row for SharedDivisionHistoryRow {
    #[inline(never)]
    fn encode_endpoint_fields(&self, mask: u64, output: &mut Vec<u8>) -> io::Result<()> {
        crate::checkpoint::rows::Row::encode_endpoint_fields(self.as_ref(), mask, output)
    }
    #[inline(never)]
    fn decode_endpoint_fields(key: u128, mask: u64, input: &mut Decoder<'_>) -> io::Result<Self> {
        <DivisionHistoryRow as crate::checkpoint::rows::Row>::decode_endpoint_fields(
            key, mask, input,
        )
        .map(Arc::new)
    }
    #[inline(never)]
    fn install_endpoint_fields(&mut self, next: Self, mask: u64) -> io::Result<()> {
        let next = match Arc::try_unwrap(next) {
            Ok(row) => row,
            Err(row) => RowCodec::try_clone_validated(row.as_ref())?,
        };
        if Arc::get_mut(self).is_none() {
            *self = Arc::new(RowCodec::try_clone_validated(self.as_ref())?);
        }
        crate::checkpoint::rows::Row::install_endpoint_fields(
            Arc::get_mut(self).unwrap(),
            next,
            mask,
        )
    }
    fn encode_checkpoint_fields(&self, output: &mut Vec<u8>) -> io::Result<()> {
        crate::checkpoint::rows::Row::encode_checkpoint_fields(self.as_ref(), output)
    }
    fn decode_checkpoint_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self> {
        <DivisionHistoryRow as crate::checkpoint::rows::Row>::decode_checkpoint_fields(key, input)
            .map(Arc::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experience_history_shared_rows_round_trip_and_delta_without_changing_held_entries() {
        let first = Arc::new(DivisionHistoryRow {
            division: u64::MAX,
            list: DivisionHistoryRow::AWARDS,
            ordinal: u32::MAX,
            available: 1,
            name: b"stored subject".to_vec(),
            medal_token: Some(17),
            medal_enabled: 1,
            multiplier: -123_456,
            provinces: vec![7, 3, 7],
            ..Default::default()
        });
        let held = RowCodec::try_clone(&first).unwrap();
        assert!(Arc::ptr_eq(&first, &held));
        let mut bytes = Vec::new();
        first.encode_fields_reserved(&mut bytes).unwrap();
        let decoded =
            SharedDivisionHistoryRow::decode_fields(first.key(), &mut Decoder::new(&bytes))
                .unwrap();
        assert_eq!(decoded, first);
        let changed = Arc::new(DivisionHistoryRow {
            medal_count: 2,
            ..first.as_ref().clone()
        });
        let delta = changed.delta(Some(&first)).unwrap().unwrap();
        assert_eq!(
            SharedDivisionHistoryRow::apply_delta(&delta, Some(first)).unwrap(),
            changed
        );
        assert_eq!(held.medal_count, 0);
        assert_eq!(held.provinces, [7, 3, 7]);
        bytes.clear();
        crate::checkpoint::rows::Row::encode_checkpoint_fields(&held, &mut bytes).unwrap();
        let decoded =
            <SharedDivisionHistoryRow as crate::checkpoint::rows::Row>::decode_checkpoint_fields(
                held.key(),
                &mut Decoder::new(&bytes),
            )
            .unwrap();
        assert_eq!(decoded, held);
    }
}

pub use game_model::modifier_breakdown::{
    SourceNode as DivisionPanelSourceNode, SourceValue as DivisionPanelSourceValue,
};
value_struct!(DivisionPanelSourceValue { key: u32, raw: i64 });
value_struct!(DivisionPanelSourceNode { children: u32, name: Vec<u8>, values: Vec<DivisionPanelSourceValue> });
opaque_value!(Option<DivisionPanelSourceNode>);
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionPanelSourceRow {
    pub kind: u8,
    pub owner: u64,
    pub ordinal: u32,
    pub node: Option<DivisionPanelSourceNode>,
    pub reason: Vec<u8>,
}
pub type SharedDivisionPanelSourceRow = Arc<DivisionPanelSourceRow>;
typed_codec!(DivisionPanelSourceRow,104,|row|((row.kind as u128)<<96)|((row.owner as u128)<<32)|row.ordinal as u128,
    |key|Self {kind:(key>>96) as u8,owner:(key>>32) as u64,ordinal:key as u32,..Default::default()}, {
        0=>(node):Opaque,1=>(reason):Bytes
    },|_row|Ok(()));
impl Fields for SharedDivisionPanelSourceRow {
    const SCHEMA: &'static [Kind] = DivisionPanelSourceRow::SCHEMA;
    const STEPS: &'static [Option<Step>] = DivisionPanelSourceRow::STEPS;
    const NAMES: &'static [&'static str] = DivisionPanelSourceRow::NAMES;
}
impl RowCodec for SharedDivisionPanelSourceRow {
    const KEY_BITS: u32 = DivisionPanelSourceRow::KEY_BITS;
    fn try_clone(&self) -> io::Result<Self> {
        self.check_key_fields()?;
        if self.key() >> Self::KEY_BITS != 0 {
            return Err(invalid("division panel source key exceeds table width"));
        }
        Ok(Arc::clone(self))
    }
    fn try_clone_validated(&self) -> io::Result<Self> {
        Ok(Arc::clone(self))
    }
    fn heap_bytes(&self) -> usize {
        std::mem::size_of::<DivisionPanelSourceRow>() + self.as_ref().heap_bytes()
    }
    fn key(&self) -> u128 {
        self.as_ref().key()
    }
    fn encoded_bound(&self) -> io::Result<usize> {
        self.as_ref().encoded_bound()
    }
    fn snapshot_bytes(&self) -> io::Result<usize> {
        self.as_ref().snapshot_bytes()
    }
    fn check_key_fields(&self) -> io::Result<()> {
        self.as_ref().check_key_fields()
    }
    fn not_recorded_fields(&self) -> u64 {
        self.as_ref().not_recorded_fields()
    }
    fn differing_fields(&self, previous: &Self) -> u64 {
        if Arc::ptr_eq(self, previous) {
            0
        } else {
            self.as_ref().differing_fields(previous)
        }
    }
    fn visit_nested_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
        self.as_ref().visit_nested_field_gaps(visit)
    }
    fn visit_wire(
        &self,
        visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.as_ref().visit_wire(visit)
    }
    fn recorded_fields_bytes(&self) -> io::Result<usize> {
        self.as_ref().recorded_fields_bytes()
    }
    fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>> {
        self.as_ref().delta(previous.map(AsRef::as_ref))
    }
    fn encode_change_reserved(
        &self,
        previous: Option<&Self>,
        previous_key: u128,
        output: &mut Vec<u8>,
    ) -> io::Result<bool> {
        self.as_ref()
            .encode_change_reserved(previous.map(AsRef::as_ref), previous_key, output)
    }
    fn encode_fields_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.as_ref().encode_fields_reserved(output)
    }
    fn decode_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self> {
        DivisionPanelSourceRow::decode_fields(key, input).map(Arc::new)
    }
    fn from_wire<'a>(
        key: u128,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<Self> {
        DivisionPanelSourceRow::from_wire(key, wire).map(Arc::new)
    }
    fn patch_from_wire<'a>(
        &mut self,
        mask: u64,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<bool> {
        if let Some(row) = Arc::get_mut(self) {
            return row.patch_from_wire(mask, wire);
        }
        let mut row = self.as_ref().try_clone()?;
        let patched = row.patch_from_wire(mask, wire)?;
        if patched {
            *self = Arc::new(row);
        }
        Ok(patched)
    }
    fn apply_delta(delta: &DeltaRow, previous: Option<Self>) -> io::Result<Self> {
        let previous = previous
            .map(|row| match Arc::try_unwrap(row) {
                Ok(row) => Ok(row),
                Err(row) => row.as_ref().try_clone(),
            })
            .transpose()?;
        DivisionPanelSourceRow::apply_delta(delta, previous).map(Arc::new)
    }
    fn has_display_steps(&self) -> bool {
        false
    }
    fn round_to_display(&mut self) {}
}
impl crate::checkpoint::rows::Row for SharedDivisionPanelSourceRow {
    #[inline(never)]
    fn encode_endpoint_fields(&self, mask: u64, output: &mut Vec<u8>) -> io::Result<()> {
        crate::checkpoint::rows::Row::encode_endpoint_fields(self.as_ref(), mask, output)
    }
    #[inline(never)]
    fn decode_endpoint_fields(key: u128, mask: u64, input: &mut Decoder<'_>) -> io::Result<Self> {
        <DivisionPanelSourceRow as crate::checkpoint::rows::Row>::decode_endpoint_fields(
            key, mask, input,
        )
        .map(Arc::new)
    }
    #[inline(never)]
    fn install_endpoint_fields(&mut self, next: Self, mask: u64) -> io::Result<()> {
        let next = match Arc::try_unwrap(next) {
            Ok(row) => row,
            Err(row) => RowCodec::try_clone_validated(row.as_ref())?,
        };
        if Arc::get_mut(self).is_none() {
            *self = Arc::new(RowCodec::try_clone_validated(self.as_ref())?);
        }
        crate::checkpoint::rows::Row::install_endpoint_fields(
            Arc::get_mut(self).unwrap(),
            next,
            mask,
        )
    }
    fn encode_checkpoint_fields(&self, output: &mut Vec<u8>) -> io::Result<()> {
        crate::checkpoint::rows::Row::encode_checkpoint_fields(self.as_ref(), output)
    }
    fn decode_checkpoint_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self> {
        <DivisionPanelSourceRow as crate::checkpoint::rows::Row>::decode_checkpoint_fields(
            key, input,
        )
        .map(Arc::new)
    }
}
