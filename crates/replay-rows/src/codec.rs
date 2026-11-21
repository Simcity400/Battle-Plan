
use std::io;

thread_local! {
    static ERROR_DETAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[must_use = "hold the guard until output work completes"]
pub fn error_details_scope() -> impl Drop {
    struct Restore(bool, std::marker::PhantomData<*mut ()>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ERROR_DETAILS.set(self.0);
        }
    }
    Restore(ERROR_DETAILS.replace(true), std::marker::PhantomData)
}

pub fn with_error_details<T>(work: impl FnOnce() -> T) -> T {
    let _restore = error_details_scope();
    work()
}

pub fn error_context(error: io::Error, context: std::fmt::Arguments<'_>) -> io::Error {
    if error.kind() != io::ErrorKind::InvalidData || !ERROR_DETAILS.get() {
        return error;
    }
    match try_format(format_args!("{context}: {error}")) {
        Ok(message) => invalid(message),
        Err(error) => error,
    }
}

pub fn diagnostic_text(text: &str) -> &str {
    let mut end = text.len().min(1024);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub fn try_format(args: std::fmt::Arguments<'_>) -> io::Result<String> {
    struct Writer(String);
    impl std::fmt::Write for Writer {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            engine_allocation::reserve_string(&mut self.0, text.len())
                .map_err(|_| std::fmt::Error)?;
            self.0.push_str(text);
            Ok(())
        }
    }
    let mut writer = Writer(String::new());
    std::fmt::write(&mut writer, args).map_err(|_| engine_allocation::oom())?;
    Ok(writer.0)
}

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn invalid_static(message: &'static str) -> io::Error {
    invalid_for_capture(message, engine_allocation::capture_active())
}

fn invalid_for_capture(message: &'static str, capture_active: bool) -> io::Error {
    if capture_active && !ERROR_DETAILS.get() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else if ERROR_DETAILS.get() {
        match engine_allocation::string(message) {
            Ok(message) => invalid(message),
            Err(error) => error,
        }
    } else {
        invalid(message)
    }
}

#[cfg(test)]
#[test]
fn spool_validation_worker_details_do_not_enable_callback_diagnostics() {
    let growths = engine_allocation::growths();
    assert!(
        invalid_for_capture("callback refusal", true)
            .get_ref()
            .is_none()
    );
    assert_eq!(engine_allocation::growths(), growths);
    let error = {
        let _details = error_details_scope();
        with_error_details(|| with_error_details(|| invalid_for_capture("worker refusal", true)))
    };
    assert_eq!(error.to_string(), "worker refusal");
    assert!(
        invalid_for_capture("callback refusal", true)
            .get_ref()
            .is_none()
    );
}

pub fn put_fixed(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    engine_allocation::reserve(out, bytes.len())?;
    out.extend_from_slice(bytes);
    Ok(())
}

pub fn put_count64(out: &mut Vec<u8>, count: usize) -> io::Result<()> {
    let count = u64::try_from(count).map_err(|_| invalid_static("count exceeds u64"))?;
    put_fixed(out, &count.to_le_bytes())
}

pub fn put_bytes64(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    put_count64(out, bytes.len())?;
    put_fixed(out, bytes)
}

pub fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    let length =
        u32::try_from(bytes.len()).map_err(|_| invalid_static("byte field exceeds u32"))?;
    let additional = bytes
        .len()
        .checked_add(4)
        .ok_or_else(engine_allocation::oom)?;
    engine_allocation::reserve(out, additional)?;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

pub fn put_string(out: &mut Vec<u8>, value: &str) -> io::Result<()> {
    put_bytes(out, value.as_bytes())
}

pub fn put_varint(out: &mut Vec<u8>, mut value: u128) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub struct Decoder<'a> {
    bytes: &'a [u8],
    pub format: u16,
    pub recorded_fields: usize,
}

macro_rules! number {
    ($name:ident, $ty:ty) => {
        pub fn $name(&mut self) -> io::Result<$ty> {
            Ok(<$ty>::from_le_bytes(
                self.take(size_of::<$ty>())?.try_into().unwrap(),
            ))
        }
    };
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            format: crate::FORMAT_VERSION,
            recorded_fields: usize::MAX,
        }
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len()
    }
    pub fn rest(&self) -> &'a [u8] {
        self.bytes
    }
    pub fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        if length > self.bytes.len() {
            return Err(invalid_static("truncated field"));
        }
        let (head, tail) = self.bytes.split_at(length);
        self.bytes = tail;
        Ok(head)
    }
    pub fn finish(self) -> io::Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(invalid_static("trailing bytes"))
        }
    }
    number!(i8, i8);
    number!(i16, i16);
    number!(u128, u128);
    number!(i128, i128);
    number!(u8, u8);
    number!(u16, u16);
    number!(u32, u32);
    number!(u64, u64);
    number!(i32, i32);
    number!(i64, i64);
    pub fn varint(&mut self) -> io::Result<u128> {
        if let Some((&byte, rest)) = self.bytes.split_first()
            && byte < 0x80
        {
            self.bytes = rest;
            return Ok(u128::from(byte));
        }
        self.varint_groups()
    }

    fn varint_groups(&mut self) -> io::Result<u128> {
        let mut value = 0_u128;
        for index in 0..19 {
            let byte = self.u8()?;
            let payload = byte & 0x7f;
            if index == 18 && (payload > 3 || byte & 0x80 != 0) {
                return Err(invalid_static("varint exceeds u128"));
            }
            value |= u128::from(payload) << (index * 7);
            if byte & 0x80 == 0 {
                if index != 0 && payload == 0 {
                    return Err(invalid_static("noncanonical overlong varint"));
                }
                return Ok(value);
            }
        }
        Err(invalid_static("varint exceeds u128"))
    }

    pub fn bool(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid_static("invalid boolean")),
        }
    }
    pub fn count(&mut self, minimum_item_bytes: usize) -> io::Result<usize> {
        let n = self.u32()? as usize;
        if minimum_item_bytes == 0 || n > self.remaining() / minimum_item_bytes {
            return Err(invalid_static(
                "collection length exceeds remaining payload",
            ));
        }
        Ok(n)
    }
    pub fn count64(&mut self, minimum_item_bytes: usize) -> io::Result<usize> {
        let n = usize::try_from(self.u64()?)
            .map_err(|_| invalid_static("collection length exceeds address space"))?;
        if minimum_item_bytes == 0 || n > self.remaining() / minimum_item_bytes {
            return Err(invalid_static(
                "collection length exceeds remaining payload",
            ));
        }
        Ok(n)
    }
    pub fn bytes64(&mut self) -> io::Result<Vec<u8>> {
        let n = self.count64(1)?;
        engine_allocation::copy_vec(self.take(n)?)
    }
    pub fn string64(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes64()?).map_err(|_| invalid_static("invalid UTF-8 string"))
    }
    pub fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let n = self.u32()? as usize;
        engine_allocation::copy_vec(self.take(n)?)
    }
    pub fn string(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| invalid_static("invalid UTF-8 string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_lengths_and_tags_fail_before_allocation() {
        assert!(Decoder::new(&u32::MAX.to_le_bytes()).bytes().is_err());
        assert!(Decoder::new(&u32::MAX.to_le_bytes()).count(1).is_err());
        assert!(Decoder::new(&[2]).bool().is_err());
        assert!(Decoder::new(&[0]).finish().is_err());
    }

    #[test]
    fn varints_preserve_full_width_values_with_canonical_bytes() {
        let mut u64_max_bytes = vec![0xff; 9];
        u64_max_bytes.push(1);
        let mut difference_max_bytes = vec![0xff; 9];
        difference_max_bytes.push(3);
        let mut u128_max_bytes = vec![0xff; 18];
        u128_max_bytes.push(3);
        for (value, expected) in [
            (0, vec![0]),
            (127, vec![127]),
            (128, vec![0x80, 1]),
            (u128::from(u64::MAX), u64_max_bytes),
            ((1_u128 << 65) - 1, difference_max_bytes),
            (u128::MAX, u128_max_bytes),
        ] {
            let mut encoded = Vec::new();
            put_varint(&mut encoded, value);
            assert_eq!(encoded, expected);
            encoded.push(0xff);
            let mut decoder = Decoder::new(&encoded);
            assert_eq!(decoder.varint().unwrap(), value);
            assert_eq!(decoder.u8().unwrap(), 0xff);
            decoder.finish().unwrap();
        }
    }

    #[test]
    fn varints_refuse_truncation_overflow_and_redundant_groups() {
        let mut overflow = vec![0xff; 18];
        overflow.push(4);
        let mut continued_final_group = vec![0xff; 18];
        continued_final_group.push(0x83);
        let mut overlong_zero = vec![0x80; 18];
        overlong_zero.push(0);
        for malformed in [
            Vec::new(),
            vec![0x80],
            vec![0xff; 9],
            vec![0x80, 0],
            vec![0x81, 0],
            overflow,
            continued_final_group,
            vec![0x80; 19],
            overlong_zero,
        ] {
            assert_eq!(
                Decoder::new(&malformed).varint().unwrap_err().kind(),
                io::ErrorKind::InvalidData,
                "accepted malformed varint {malformed:02x?}",
            );
        }
    }

    #[test]
    fn single_group_varints_match_the_group_loop_in_value_error_and_position() {
        let mut inputs = vec![Vec::new(), vec![0xff; 9], vec![0x80; 19]];
        for first in 0..=u8::MAX {
            inputs.push(vec![first]);
            for second in [0, 1, 0x7f, 0x80, 0xff] {
                inputs.push(vec![first, second]);
                inputs.push(vec![first, second, 0x7f]);
            }
        }
        for value in [0, 127, 128, 16_383, 16_384, u128::from(u64::MAX), u128::MAX] {
            let mut encoded = Vec::new();
            put_varint(&mut encoded, value);
            encoded.push(5);
            inputs.push(encoded);
        }
        for input in inputs {
            let mut shortcut = Decoder::new(&input);
            let mut groups = Decoder::new(&input);
            let outcome = |result: io::Result<u128>| result.map_err(|e| (e.kind(), e.to_string()));
            assert_eq!(
                outcome(shortcut.varint()),
                outcome(groups.varint_groups()),
                "{input:02x?}"
            );
            assert_eq!(shortcut.remaining(), groups.remaining(), "{input:02x?}");
        }
    }
}
