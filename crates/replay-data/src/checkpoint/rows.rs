//! Typed prepared rows use fixed keys and raw decoded fields, never wire deltas.
use crate::{
    codec::{Decoder, invalid_static as invalid},
    rows::RowCodec,
};
use engine_allocation as allocation;
use std::io;

pub(crate) trait Row: RowCodec {
    fn encode_checkpoint_fields(&self, output: &mut Vec<u8>) -> io::Result<()>;
    fn decode_checkpoint_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self>;
}

fn check_key<T: RowCodec>(key: u128) -> io::Result<()> {
    if T::KEY_BITS < 128 && key >> T::KEY_BITS != 0 {
        Err(invalid("checkpoint key exceeds table width"))
    } else {
        Ok(())
    }
}

pub(crate) fn encode_rows<T: Row>(rows: &[T], output: &mut Vec<u8>) -> io::Result<()> {
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

pub(crate) fn decode_rows<T: Row>(input: &mut Decoder<'_>) -> io::Result<Vec<T>> {
    let count = input.count(16)?;
    let mut rows = allocation::vec(count)?;
    for _ in 0..count {
        let key = u128::from_le_bytes(input.take(16)?.try_into().unwrap());
        check_key::<T>(key)?;
        rows.push(T::decode_checkpoint_fields(key, input)?);
    }
    Ok(rows)
}

macro_rules! row {
    ($ty:ty, |$key_name:ident| $initial:expr, { $(($($field:tt)+): $kind:ident),+ $(,)? }) => {
        impl $crate::checkpoint::rows::Row for $ty {
            fn encode_checkpoint_fields(&self, output: &mut Vec<u8>) -> std::io::Result<()> {
                $($crate::rows::checkpoint::Native::encode(&self.$($field)+, output)?;)+
                Ok(())
            }
            #[allow(clippy::field_reassign_with_default)]
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
pub(crate) use row;

macro_rules! tables {
    ($($table:ident => $row:ty),+ $(,)?) => {
        impl $crate::rows::TableId {
            pub fn decode_checkpoint_rows(self, input: &mut $crate::codec::Decoder<'_>) -> std::io::Result<$crate::rows::TableRows> {
                Ok(match self { $(Self::$table => $crate::rows::TableRows::$table($crate::checkpoint::rows::decode_rows::<$row>(input)?)),+ })
            }
        }
        impl $crate::rows::TableRows {
            pub fn encode_checkpoint_rows(&self, output: &mut Vec<u8>) -> std::io::Result<()> {
                match self { $(Self::$table(rows) => $crate::checkpoint::rows::encode_rows(rows, output)),+ }
            }
        }
    };
}
pub(crate) use tables;
