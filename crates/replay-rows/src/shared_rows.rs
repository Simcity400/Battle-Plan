use super::*;
use crate::checkpoint::rows::Row;
use std::sync::Arc;

pub trait SharedRow: RowCodec {
    const KEY_ERROR: &'static str;
}

impl<T: SharedRow + Fields> Fields for Arc<T> {
    const SCHEMA: &'static [Kind] = T::SCHEMA;
    const STEPS: &'static [Option<Step>] = T::STEPS;
    const NAMES: &'static [&'static str] = T::NAMES;
}
impl<T: SharedRow> RowCodec for Arc<T> {
    const KEY_BITS: u32 = T::KEY_BITS;
    fn try_clone(&self) -> io::Result<Self> {
        self.check_key_fields()?;
        if self.key() >> Self::KEY_BITS != 0 {
            return Err(invalid(T::KEY_ERROR));
        }
        Ok(Arc::clone(self))
    }
    fn try_clone_validated(&self) -> io::Result<Self> {
        Ok(Arc::clone(self))
    }
    fn heap_bytes(&self) -> usize {
        std::mem::size_of::<T>() + self.as_ref().heap_bytes()
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
    fn visit_wire_masked(
        &self,
        mask: u64,
        visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.as_ref().visit_wire_masked(mask, visit)
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
        T::decode_fields(key, input).map(Arc::new)
    }
    fn from_wire<'a>(
        key: u128,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<Self> {
        T::from_wire(key, wire).map(Arc::new)
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
        T::apply_delta(delta, previous).map(Arc::new)
    }
    fn has_display_steps(&self) -> bool {
        false
    }
    fn round_to_display(&mut self) {}
}
impl<T: SharedRow + Row> Row for Arc<T> {
    #[inline(never)]
    fn encode_endpoint_fields(&self, mask: u64, output: &mut Vec<u8>) -> io::Result<()> {
        Row::encode_endpoint_fields(self.as_ref(), mask, output)
    }
    #[inline(never)]
    fn decode_endpoint_fields(key: u128, mask: u64, input: &mut Decoder<'_>) -> io::Result<Self> {
        <T as Row>::decode_endpoint_fields(
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
        Row::install_endpoint_fields(
            Arc::get_mut(self).unwrap(),
            next,
            mask,
        )
    }
    fn encode_checkpoint_fields(&self, output: &mut Vec<u8>) -> io::Result<()> {
        Row::encode_checkpoint_fields(self.as_ref(), output)
    }
    fn decode_checkpoint_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self> {
        <T as Row>::decode_checkpoint_fields(key, input)
            .map(Arc::new)
    }
}
