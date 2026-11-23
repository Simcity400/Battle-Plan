
use super::*;

fn entry_span(header: [u8; 16]) -> Option<(usize, usize)> {
    let entries = Array::from_header(header, modifier_entry::STRIDE)?;
    let count = (entries.count as usize).min(ENTRY_BATCH);
    (count != 0).then_some((entries.data, count * modifier_entry::STRIDE))
}

fn display_spans(header: [u8; 32]) -> [Option<(usize, usize)>; 2] {
    let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
    let length = word(engine_native_memory::msvc_string::LENGTH);
    let capacity = word(engine_native_memory::msvc_string::CAPACITY);
    if length > capacity || capacity <= engine_native_memory::msvc_string::INLINE_CAPACITY {
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
