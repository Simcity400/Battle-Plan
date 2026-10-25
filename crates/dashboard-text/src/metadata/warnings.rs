use serde::{
    Deserialize, Serialize,
    de::{DeserializeSeed, SeqAccess, Visitor},
    ser::SerializeSeq,
};
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    hash::BuildHasher,
    io::{self, Read, Seek, SeekFrom, Write},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

const PREFIX: &str = "Metadata warning storage refused: ";
const SUFFIX: &str = " further distinct warnings were not stored because allocation failed.";

#[derive(Default)]
pub struct Refusals {
    spill: Option<Mutex<Spill>>,
    summary: Mutex<String>,
}

impl Refusals {
    fn new() -> io::Result<Self> {
        let mut summary = String::new();
        engine_allocation::reserve_string(
            &mut summary,
            PREFIX.len() + usize::MAX.ilog10() as usize + 1 + SUFFIX.len(),
        )?;
        Ok(Self {
            spill: Some(Mutex::new(Spill::new()?)),
            summary: Mutex::new(summary),
        })
    }

    pub fn count(&self) -> usize {
        self.spill.as_ref().map_or(0, |spill| {
            spill.lock().unwrap_or_else(|e| e.into_inner()).count
        })
    }

    pub(super) fn copy_wire(&self, values: &mut Vec<String>) -> io::Result<()> {
        let count = self.count();
        if count != 0 {
            engine_allocation::reserve(values, 1)?;
            let mut summary = self.summary.lock().unwrap_or_else(|e| e.into_inner());
            format_summary(&mut summary, count);
            values.push(engine_allocation::string(&summary)?);
        }
        Ok(())
    }

    pub(super) fn append_wire(mut self, values: &mut Vec<String>) -> io::Result<()> {
        let count = self.count();
        if count != 0 {
            engine_allocation::reserve(values, 1)?;
            let summary = self.summary.get_mut().unwrap_or_else(|e| e.into_inner());
            format_summary(summary, count);
            values.push(std::mem::take(summary));
        }
        Ok(())
    }

    fn contains(&mut self, message: &str) -> io::Result<bool> {
        match &mut self.spill {
            Some(spill) => {
                let spill = spill.get_mut().unwrap_or_else(|e| e.into_inner());
                if spill.count == 0 {
                    return Ok(false);
                }
                spill.contains(message)
            }
            None => Ok(false),
        }
    }

    fn insert(&mut self, message: &str) -> io::Result<()> {
        let spill = self
            .spill
            .as_mut()
            .expect("refusal index reserved before collection")
            .get_mut()
            .unwrap_or_else(|e| e.into_inner());
        spill.insert(spill.hasher.hash_one(message), message.as_bytes())?;
        Ok(())
    }

    fn visit(&self, visit: impl FnMut(&str) -> io::Result<()>) -> io::Result<()> {
        if let Some(spill) = &self.spill {
            spill
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .visit(visit)?;
        }
        Ok(())
    }

    fn discard_empty(&mut self) {
        if self.count() == 0 {
            self.spill = None;
        }
    }
}

impl Serialize for Refusals {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.count()))?;
        let mut error = None;
        self.visit(|message| {
            sequence.serialize_element(message).map_err(|e| {
                error = Some(e);
                io::Error::other("Metadata refusal serialization failed")
            })
        })
        .map_err(|e| error.unwrap_or_else(|| serde::ser::Error::custom(e)))?;
        sequence.end()
    }
}

impl<'de> Deserialize<'de> for Refusals {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RefusalList;
        impl<'de> Visitor<'de> for RefusalList {
            type Value = Refusals;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("distinct refused metadata warning identities")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Refusals, A::Error> {
                let mut refusals = Refusals::new().map_err(serde::de::Error::custom)?;
                while seq
                    .next_element_seed(RefusalIdentity(&mut refusals))?
                    .is_some()
                {}
                refusals.discard_empty();
                Ok(refusals)
            }
        }
        deserializer.deserialize_seq(RefusalList)
    }
}

struct RefusalIdentity<'a>(&'a mut Refusals);
impl<'de> DeserializeSeed<'de> for RefusalIdentity<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for RefusalIdentity<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a normalized metadata warning")
    }
    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<(), E> {
        if text.len() > 1024 {
            return Err(E::custom("Oversized metadata refusal identity"));
        }
        self.0.insert(text).map_err(E::custom)
    }
}

fn format_summary(summary: &mut String, mut count: usize) {
    let mut digits = [0u8; usize::BITS as usize];
    let mut first = digits.len();
    loop {
        first -= 1;
        digits[first] = b'0' + (count % 10) as u8;
        count /= 10;
        if count == 0 {
            break;
        }
    }
    let digits = std::str::from_utf8(&digits[first..]).unwrap();
    assert!(summary.capacity() >= PREFIX.len() + digits.len() + SUFFIX.len());
    summary.clear();
    summary.push_str(PREFIX);
    summary.push_str(digits);
    summary.push_str(SUFFIX);
}

pub struct Warnings {
    values: Vec<String>,
    index: HashSet<String>,
    refused: Refusals,
}

impl Warnings {
    pub fn new() -> io::Result<Self> {
        #[cfg(any(test, feature = "test-support"))]
        if FAIL_INITIALIZATION.get() {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "Metadata warning refusal index allocation failed",
            ));
        }
        Ok(Self {
            values: engine_allocation::vec(1)?,
            index: HashSet::new(),
            refused: Refusals::new()?,
        })
    }

    pub fn import(&mut self, values: &[String], refused: &Refusals) -> io::Result<()> {
        for value in values {
            self.warn(value)?;
        }
        self.import_refusals(refused)
    }

    pub fn import_refusals(&mut self, refused: &Refusals) -> io::Result<()> {
        refused.visit(|message| {
            if !self.index.contains(message) {
                self.refused.insert(message)?;
            }
            Ok(())
        })
    }

    pub fn warn(&mut self, message: &str) -> io::Result<()> {
        let mut end = message.len().min(1024);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        let message = &message[..end];
        if self.index.contains(message) || self.refused.contains(message)? {
            return Ok(());
        }
        // S3 will charge resident diagnostics to the shared memory owner once it exists.
        let stored = (|| {
            storage_attempt()?;
            engine_allocation::reserve(&mut self.values, 2)?;
            engine_allocation::reserve_set(&mut self.index, 1)?;
            let value = engine_allocation::string(message)?;
            let key = engine_allocation::string(message)?;
            self.index.insert(key);
            self.values.push(value);
            Ok::<_, io::Error>(())
        })();
        match stored {
            Ok(()) => Ok(()),
            Err(error) if engine_allocation::is_allocation_error(&error) => {
                self.refused.insert(message)
            }
            Err(error) => Err(error),
        }
    }

    pub fn finish(mut self) -> (Vec<String>, Refusals) {
        self.refused.discard_empty();
        (self.values, self.refused)
    }
}

fn storage_attempt() -> io::Result<()> {
    #[cfg(any(test, feature = "test-support"))]
    if FAIL_AFTER.with(|remaining| match remaining.get() {
        Some(0) => true,
        Some(n) => {
            remaining.set(Some(n - 1));
            false
        }
        None => false,
    }) {
        return Err(engine_allocation::oom());
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static FAIL_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static FAIL_INITIALIZATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_warning_initialization_failure<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            FAIL_INITIALIZATION.set(self.0);
        }
    }
    let _reset = Reset(FAIL_INITIALIZATION.replace(true));
    run()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_storage_failure_after<T>(accepted: usize, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<usize>);
    impl Drop for Reset {
        fn drop(&mut self) {
            FAIL_AFTER.set(self.0);
        }
    }
    let _reset = Reset(FAIL_AFTER.replace(Some(accepted)));
    run()
}

// Overflow identities live on disk so counting distinct refusals does not grow the heap.
struct Spill {
    file: Option<File>,
    root: u64,
    end: u64,
    count: usize,
    hasher: std::collections::hash_map::RandomState,
}

impl Spill {
    fn new() -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "hoi4-metadata-warnings-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    DELETE, FILE_FLAG_DELETE_ON_CLOSE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
                };
                // Delete-on-close avoids allocating a Windows path during OOM cleanup.
                options
                    .access_mode(FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE)
                    .custom_flags(FILE_FLAG_DELETE_ON_CLOSE);
            }
            match options.open(&path) {
                Ok(file) => {
                    #[cfg(not(windows))]
                    std::fs::remove_file(&path)?;
                    return Ok(Self {
                        file: Some(file),
                        root: 0,
                        end: 0,
                        count: 0,
                        hasher: Default::default(),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(io::Error::other(format!(
                        "Metadata warning refusal index unavailable: {error}"
                    )));
                }
            }
        }
    }

    fn insert(&mut self, hash: u64, message: &[u8]) -> io::Result<bool> {
        let file = self.file.as_mut().unwrap();
        let mut node = self.root;
        let mut parent = None;
        while node != 0 {
            let offset = node - 1;
            file.seek(SeekFrom::Start(offset))?;
            let mut header = [0; 32];
            file.read_exact(&mut header)?;
            let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
            let length = usize::try_from(word(24))
                .ok()
                .filter(|&length| length <= 1024)
                .ok_or_else(|| io::Error::other("Invalid metadata warning refusal index"))?;
            let mut order = hash.cmp(&word(16));
            if order.is_eq() {
                let mut text = [0; 1024];
                file.read_exact(&mut text[..length])?;
                order = message.cmp(&text[..length]);
            }
            if order.is_eq() {
                return Ok(false);
            }
            let side = if order.is_lt() { 0 } else { 8 };
            parent = Some(offset + side as u64);
            node = word(side);
        }
        let node = self
            .end
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Metadata warning refusal index overflow"))?;
        file.seek(SeekFrom::Start(self.end))?;
        file.write_all(&[0; 16])?;
        file.write_all(&hash.to_le_bytes())?;
        file.write_all(&(message.len() as u64).to_le_bytes())?;
        file.write_all(message)?;
        self.end = self
            .end
            .checked_add(32 + message.len() as u64)
            .ok_or_else(|| io::Error::other("Metadata warning refusal index overflow"))?;
        if let Some(parent) = parent {
            file.seek(SeekFrom::Start(parent))?;
            file.write_all(&node.to_le_bytes())?;
        } else {
            self.root = node;
        }
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Metadata warning refusal count overflow"))?;
        Ok(true)
    }

    fn contains(&mut self, message: &str) -> io::Result<bool> {
        let hash = self.hasher.hash_one(message);
        let file = self.file.as_mut().unwrap();
        let mut node = self.root;
        while node != 0 {
            file.seek(SeekFrom::Start(node - 1))?;
            let mut header = [0; 32];
            file.read_exact(&mut header)?;
            let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
            let length = usize::try_from(word(24))
                .ok()
                .filter(|&n| n <= 1024)
                .ok_or_else(|| io::Error::other("Invalid metadata warning refusal index"))?;
            let mut order = hash.cmp(&word(16));
            if order.is_eq() {
                let mut text = [0; 1024];
                file.read_exact(&mut text[..length])?;
                order = message.as_bytes().cmp(&text[..length]);
            }
            if order.is_eq() {
                return Ok(true);
            }
            node = word(if order.is_lt() { 0 } else { 8 });
        }
        Ok(false)
    }

    fn visit(&mut self, mut visit: impl FnMut(&str) -> io::Result<()>) -> io::Result<()> {
        let file = self.file.as_mut().unwrap();
        file.seek(SeekFrom::Start(0))?;
        let mut offset = 0;
        while offset < self.end {
            let mut header = [0; 32];
            file.read_exact(&mut header)?;
            let length = usize::try_from(u64::from_le_bytes(header[24..32].try_into().unwrap()))
                .ok()
                .filter(|&n| n <= 1024)
                .ok_or_else(|| io::Error::other("Invalid metadata warning refusal index"))?;
            let mut text = [0; 1024];
            file.read_exact(&mut text[..length])?;
            let text = std::str::from_utf8(&text[..length]).map_err(io::Error::other)?;
            visit(text)?;
            offset += 32 + length as u64;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_vector_index_and_both_string_growth_failures_keep_a_reserved_refusal_slot() {
        let mut healthy = Warnings::new().unwrap();
        let (stored, allocations) =
            crate::test_allocator::deny_after(usize::MAX, || healthy.warn("distinct"));
        stored.unwrap();
        assert!(allocations > 0);
        for allowed in 0..allocations {
            let mut warnings = Warnings::new().unwrap();
            let (result, _) =
                crate::test_allocator::deny_after(allowed, || warnings.warn("distinct"));
            result.unwrap();
            let (mut values, refused) = warnings.finish();
            assert!(values.is_empty());
            assert_eq!(refused.count(), 1);
            let (wire, attempts) =
                crate::test_allocator::deny_after(0, || refused.append_wire(&mut values));
            wire.unwrap();
            assert_eq!(attempts, 0);
            assert_eq!(
                values,
                [
                    "Metadata warning storage refused: 1 further distinct warnings were not stored because allocation failed."
                ]
            );
        }
    }

    #[test]
    fn actual_denied_growth_counts_distinct_refusals_and_materializes_the_reserved_summary() {
        let mut warnings = Warnings::new().unwrap();
        warnings.warn("retained").unwrap();
        let (result, attempts) = crate::test_allocator::deny_after(0, || {
            for text in ["refused one", "refused two", "refused one", "refused three"] {
                warnings.warn(text)?;
            }
            Ok::<_, io::Error>(())
        });
        result.unwrap();
        assert!(attempts >= 3);
        let (mut values, refused) = warnings.finish();
        assert_eq!(refused.count(), 3);
        assert!(values.capacity() > values.len());
        let (result, attempts) =
            crate::test_allocator::deny_after(0, || refused.append_wire(&mut values));
        result.unwrap();
        assert_eq!(attempts, 0);
        assert_eq!(
            values,
            [
                "retained",
                "Metadata warning storage refused: 3 further distinct warnings were not stored because allocation failed."
            ]
        );
    }

    #[test]
    fn reserved_summary_formats_the_largest_exact_count_without_growth() {
        let mut refusals = Refusals::new().unwrap();
        let summary = refusals.summary.get_mut().unwrap();
        let ((), attempts) =
            crate::test_allocator::deny_after(0, || format_summary(summary, usize::MAX));
        assert_eq!(attempts, 0);
        assert_eq!(summary, &format!("{PREFIX}{}{SUFFIX}", usize::MAX));
    }

    #[test]
    fn many_distinct_warnings_keep_first_order_and_exact_allocation_refusals() {
        const COUNT: usize = 100_000;
        for accepted in [COUNT, 2] {
            let mut warnings = Warnings::new().unwrap();
            with_storage_failure_after(accepted, || {
                for hour in 0..COUNT {
                    let message = format!("unreadable synthetic block at hour {hour}");
                    warnings.warn(&message).unwrap();
                    warnings.warn(&message).unwrap();
                }
                for hour in (0..COUNT).rev() {
                    warnings
                        .warn(&format!("unreadable synthetic block at hour {hour}"))
                        .unwrap();
                }
            });
            let (mut values, refused) = warnings.finish();
            assert_eq!(values.len(), accepted);
            for (hour, warning) in values.iter().enumerate() {
                assert_eq!(
                    warning,
                    &format!("unreadable synthetic block at hour {hour}")
                );
            }
            assert_eq!(refused.count(), COUNT - accepted);
            refused.append_wire(&mut values).unwrap();
            if accepted < COUNT {
                assert_eq!(
                    values.last().unwrap(),
                    &format!("{PREFIX}{}{SUFFIX}", COUNT - accepted)
                );
            }
        }
    }

    #[test]
    fn allocation_recovery_keeps_admitted_diagnostics_and_stable_refusal_identity() {
        let mut warnings = Warnings::new().unwrap();
        warnings.warn("first").unwrap();
        with_storage_failure_after(0, || warnings.warn("refused").unwrap());
        warnings.warn("second").unwrap();
        warnings.warn("refused").unwrap();
        let (values, refused) = warnings.finish();
        assert_eq!(values, ["first", "second"]);
        assert_eq!(refused.count(), 1);
    }

    #[test]
    fn refused_hash_collisions_compare_complete_text() {
        let mut spill = Spill::new().unwrap();
        for text in [b"second".as_slice(), b"first", b"third"] {
            assert!(spill.insert(7, text).unwrap());
            assert!(!spill.insert(7, text).unwrap());
        }
        assert_eq!(spill.count, 3);
    }

    #[test]
    fn exhausted_allocation_restore_and_append_keep_one_exact_wire_refusal() {
        let mut original = Warnings::new().unwrap();
        original.warn("retained").unwrap();
        let message = "\u{e9}".repeat(512) + "ignored suffix";
        with_storage_failure_after(0, || {
            original.warn(&message).unwrap();
            original.warn("another refusal").unwrap();
        });
        let (values, refused) = original.finish();
        let encoded = serde_json::to_vec(&refused).unwrap();
        let restored: Refusals = serde_json::from_slice(&encoded).unwrap();
        let mut reopened = Warnings::new().unwrap();
        reopened.import(&values, &restored).unwrap();
        with_storage_failure_after(0, || {
            reopened.warn(&message).unwrap();
            reopened
                .warn(&("\u{e9}".repeat(512) + "different suffix"))
                .unwrap();
            reopened.warn("another refusal").unwrap();
            reopened.warn("new refusal").unwrap();
        });
        let (mut values, refused) = reopened.finish();
        assert_eq!(values, ["retained"]);
        assert_eq!(refused.count(), 3);
        refused.append_wire(&mut values).unwrap();
        assert_eq!(
            values,
            ["retained".to_owned(), format!("{PREFIX}3{SUFFIX}")]
        );
    }
}
