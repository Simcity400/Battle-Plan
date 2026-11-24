use crate::{
    codec::{Decoder, invalid_static as invalid},
    rows::RowCodec,
};
use engine_allocation as allocation;
use std::io;

pub trait Row: RowCodec {
    fn encode_checkpoint_fields(&self, output: &mut Vec<u8>) -> io::Result<()>;
    fn decode_checkpoint_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self>;
    fn encode_endpoint_fields(&self, mask: u64, output: &mut Vec<u8>) -> io::Result<()>;
    fn decode_endpoint_fields(key: u128, mask: u64, input: &mut Decoder<'_>) -> io::Result<Self>;
    fn install_endpoint_fields(&mut self, next: Self, mask: u64) -> io::Result<()>;
}

fn check_key<T: RowCodec>(key: u128) -> io::Result<()> {
    if T::KEY_BITS < 128 && key >> T::KEY_BITS != 0 {
        Err(invalid("checkpoint key exceeds table width"))
    } else {
        Ok(())
    }
}

pub fn encode_rows<T: Row>(rows: &[T], output: &mut Vec<u8>) -> io::Result<()> {
    let count =
        u32::try_from(rows.len()).map_err(|_| invalid("checkpoint row count exceeds u32"))?;
    allocation::reserve(output, 4)?;
    output.extend_from_slice(&count.to_le_bytes());
    for row in rows {
        row.check_key_fields()?;
        check_key::<T>(row.key())?;
        allocation::reserve(output, 16)?;
        output.extend_from_slice(&row.key().to_le_bytes());
        row.encode_checkpoint_fields(output)?;
    }
    Ok(())
}

pub fn decode_rows<T: Row>(input: &mut Decoder<'_>) -> io::Result<Vec<T>> {
    let count = input.count(16)?;
    let mut rows = allocation::vec(count)?;
    for _ in 0..count {
        let key = u128::from_le_bytes(input.take(16)?.try_into().unwrap());
        check_key::<T>(key)?;
        rows.push(T::decode_checkpoint_fields(key, input)?);
    }
    Ok(rows)
}

#[macro_export]
#[doc(hidden)]
macro_rules! row {
    ($ty:ty, |$key_name:ident| $initial:expr, { $(($($field:tt)+): $kind:ident),+ $(,)? }) => {
        impl $crate::checkpoint::rows::Row for $ty {
            fn encode_checkpoint_fields(&self, output: &mut Vec<u8>) -> std::io::Result<()> {
                $($crate::rows::checkpoint::Native::encode(&self.$($field)+, output)?;)+
                Ok(())
            }
            #[inline(never)]
            fn encode_endpoint_fields(&self, mask: u64, output: &mut Vec<u8>) -> std::io::Result<()> {
                let mut bit = 1u64;
                $(if mask & bit != 0 { $crate::rows::checkpoint::Native::encode(&self.$($field)+, output)?; }
                  bit = bit.checked_shl(1).unwrap_or(0);)+
                let _ = bit;
                Ok(())
            }
            #[inline(never)]
            fn decode_endpoint_fields(key: u128, mask: u64, input: &mut $crate::codec::Decoder<'_>) -> std::io::Result<Self> {
                let $key_name = key;
                let mut row = $initial;
                let mut bit = 1u64;
                $(if mask & bit != 0 { row.$($field)+ = $crate::rows::checkpoint::Native::decode($crate::rows::Kind::$kind, input)?; }
                  bit = bit.checked_shl(1).unwrap_or(0);)+
                let _ = bit;
                Ok(row)
            }
            #[inline(never)]
            fn install_endpoint_fields(&mut self, next: Self, mask: u64) -> std::io::Result<()> {
                let mut bit = 1u64;
                $(if mask & bit != 0 { self.$($field)+ = next.$($field)+; }
                  bit = bit.checked_shl(1).unwrap_or(0);)+
                let _ = bit;
                Ok(())
            }
            fn decode_checkpoint_fields(key: u128, input: &mut $crate::codec::Decoder<'_>) -> std::io::Result<Self> {
                let $key_name = key;
                let mut row = $initial;
                $(row.$($field)+ = $crate::rows::checkpoint::Native::decode($crate::rows::Kind::$kind, input)?;)+
                $crate::rows::RowCodec::check_key_fields(&row)?;
                Ok(row)
            }
        }
    };
}
