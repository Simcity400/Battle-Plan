//! Guarded, exact-field batches of the serial stability walk's operands. Values
//! still pass the serial vtable, owner, array and string checks before use.
//! Filtered modifier addition, child cloning and modifier copying establish the
//! owned entry arrays and display strings.

use super::*;

/// One original nonzero() entry window, never an enclosing allocation span.
fn entry_span(header: [u8; 16]) -> Option<(usize, usize)> {
    let entries = Array::from_header(header, modifier_entry::STRIDE)?;
    let count = (entries.count as usize).min(ENTRY_BATCH);
    (count != 0).then_some((entries.data, count * modifier_entry::STRIDE))
}

/// The original display_digest() heap terminator and first payload window.
/// Inline text is owned by the acquired header and needs no native read.
fn display_spans(header: [u8; 32]) -> [Option<(usize, usize)>; 2] {
    let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
    let length = word(crate::msvc_string::LENGTH);
    let capacity = word(crate::msvc_string::CAPACITY);
    if length > capacity || capacity <= crate::msvc_string::INLINE_CAPACITY {
        return [None; 2];
    }
    let Some(source) = usize::try_from(word(0)).ok().filter(|&p| p != 0) else {
        return [None; 2];
    };
    let Ok(len) = usize::try_from(length) else {
        return [None; 2];
    };
    let Some(end) = source.checked_add(len) else {
        return [None; 2];
    };
    [
        Some((end, 1)),
        (len != 0).then_some((source, len.min(1024))),
    ]
}

fn source_head(modifier: usize, display: bool) -> [Option<(usize, usize)>; 2] {
    [
        modifier
            .checked_add(cmodifier::ACCUMULATOR)
            .map(|p| (p, 16)),
        display
            .then(|| {
                modifier
                    .checked_add(cmodifier::DISPLAY_NAME)
                    .map(|p| (p, 32))
            })
            .flatten(),
    ]
}

fn source_payload(
    memory: &impl Memory,
    modifier: usize,
    display: bool,
) -> [Option<(usize, usize)>; 3] {
    let entries = modifier
        .checked_add(cmodifier::ACCUMULATOR)
        .and_then(|p| read::<[u8; 16]>(memory, p))
        .and_then(entry_span);
    let text = if display {
        modifier
            .checked_add(cmodifier::DISPLAY_NAME)
            .and_then(|p| read::<[u8; 32]>(memory, p))
            .map(display_spans)
            .unwrap_or([None; 2])
    } else {
        [None; 2]
    };
    [entries, text[0], text[1]]
}

/// Fixed source blocks across objects, then dependent exact payload windows.
/// Static modifier binding authenticates Context's shared slots; Scaled static
/// modifier addition reads those same CModifier fields.
pub(super) fn sources(
    memory: &Observed<'_, impl Memory>,
    modifiers: &[usize],
    display: bool,
    shared: bool,
) {
    memory.gather(
        modifiers
            .iter()
            .flat_map(|&p| source_head(p, display))
            .flatten(),
        shared,
    );
    let spans = modifiers
        .iter()
        .flat_map(|&p| source_payload(&memory.prepared(), p, display))
        .flatten();
    memory.gather(spans, shared);
}

/// Heads and source fields belong to the same reached native object. Advisor
/// contributions use the advisor modifier offset; political contributions use
/// the idea and leader/ideology offsets. No record spans an object's gaps or
/// another allocation.
pub(super) fn objects(
    memory: &Observed<'_, impl Memory>,
    pointers: &[usize],
    head_len: usize,
    modifier_offset: usize,
    display: bool,
) {
    memory.gather(
        pointers.iter().flat_map(|&p| {
            let fields = p
                .checked_add(modifier_offset)
                .map(|m| source_head(m, display))
                .unwrap_or([None; 2]);
            [Some((p, head_len)), fields[0], fields[1]]
                .into_iter()
                .flatten()
        }),
        false,
    );
}

/// Child heads were acquired by the original page gather. Payload selection
/// uses only complete, authenticated heads and individually bounded headers.
pub(super) fn children(
    memory: &Observed<'_, impl Memory>,
    pointers: &[usize],
    heads: &[u8],
    complete: usize,
    vtable: u64,
) {
    memory.gather(
        pointers
            .iter()
            .take(complete)
            .enumerate()
            .flat_map(|(index, _)| {
                let head = &heads[index * CHILD_HEAD_BYTES..(index + 1) * CHILD_HEAD_BYTES];
                if u64::from_le_bytes(head[..8].try_into().unwrap()) != vtable {
                    return [None; 3];
                }
                let entries = entry_span(head[8..24].try_into().unwrap());
                let text = display_spans(head[24..56].try_into().unwrap());
                [entries, text[0], text[1]]
            })
            .flatten(),
        false,
    );
}

/// Pointer pages preserve null/overflow refusal order. The serial object
/// readers visit the validated prefix even when a later pointer is invalid.
pub(super) fn pointer_pages(
    memory: &impl Memory,
    array: Array,
    mut visit: impl FnMut(&[usize]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut bytes = [0; 64 * 8];
    let mut pointers = [0; 64];
    let mut first = 0;
    while first < array.count {
        let count = (array.count - first).min(64) as usize;
        if !read_slice(
            memory,
            array.element(first, 8).ok_or("pointer page overflow")?,
            &mut bytes[..count * 8],
        ) {
            return Err("country stability pointers unreadable");
        }
        let mut valid = 0;
        let mut invalid = None;
        for (target, bytes) in pointers.iter_mut().zip(bytes[..count * 8].chunks_exact(8)) {
            match pointer_from(bytes) {
                Ok(pointer) => {
                    *target = pointer;
                    valid += 1;
                }
                Err(reason) => {
                    invalid = Some(reason);
                    break;
                }
            }
        }
        visit(&pointers[..valid])?;
        if let Some(reason) = invalid {
            return Err(reason);
        }
        first += count as u32;
    }
    Ok(())
}
