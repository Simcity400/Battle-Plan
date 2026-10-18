//! DivisionTemplates derived every hour from the template icon walk
//! ([`crate::template_changes`]).
//!
//! A dictionary row records a template's handle, roster owner, literal name, fallback name key and
//! unit_type. The hourly icon walk ([`native::walk_icons_with_names_growing`]) visits the same
//! rosters in the same order and reads, for every template, its handle, owner, literal and
//! unit-type operands (the unit-type override and the definition category [`native::walk`]
//! derives), and the key behind an empty literal. Only a key behind a nonempty literal is not read
//! every hour: it is carried from the held capture when the template object and handle are the held
//! ones and either the family's quiet token is the one taken before that capture or no hooked
//! writer touched the template since it registered (every run-time writer of the key is a hooked
//! site, [`engine_mutations::template_mutations`]). Every other template's names are read afresh. A capture
//! is held only when it completed and the quiet token stood still across it. The carryable keys of
//! the hour's [`crate::sampled_verify`] slice (by template id) are read afresh instead and compared
//! with the held ones. After all name reads and an acquire fence, an unchanged final certificate
//! permits a mismatch verdict; an overlapping writer withdraws the capture without poisoning. A key
//! mismatch poisons the key domain. Allocation is advisory: a capture that cannot be held reads
//! every such key again next hour.
use crate::{
    sampled_verify::Check,
    template_changes::{Changes, Counts},
};
use engine_allocation as allocation;
use engine_native::{creferenced_division_template as native, walk::Outcome};
use std::{
    ops::Range,
    sync::atomic::{Ordering, fence},
};

/// One template of the held capture.
#[derive(Clone, Debug)]
struct HeldKey {
    pointer: usize,
    template: u64,
    /// The template's certificate; `None` when it could not be registered.
    certificate: Option<(u64, u64)>,
    /// The key's bytes in the store's arena.
    key: Range<usize>,
}

/// The latest complete capture's keys, with the quiet token taken before it.
#[derive(Default)]
pub(crate) struct KeyStore {
    quiet: Option<(u64, u64)>,
    held: Vec<HeldKey>,
    keys: Vec<u8>,
    /// `held` indices by template pointer, built when a capture's order
    /// departs from the held one.
    sorted: Vec<usize>,
    /// This capture's templates and keys, swapped in when it is held.
    next: Vec<HeldKey>,
    next_keys: Vec<u8>,
}

impl KeyStore {
    /// Nothing held certifies anything.
    pub(crate) fn forget(&mut self) {
        self.quiet = None;
        self.held.clear();
        self.keys.clear();
        self.sorted.clear();
        self.next.clear();
        self.next_keys.clear();
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        (self.held.capacity() + self.next.capacity()) * size_of::<HeldKey>()
            + self.keys.capacity()
            + self.next_keys.capacity()
            + self.sorted.capacity() * size_of::<usize>()
    }
}

/// Finds a template's held entry: the next in held order, else by pointer.
struct Finder<'a> {
    held: &'a [HeldKey],
    sorted: &'a mut Vec<usize>,
    cursor: usize,
}

impl<'a> Finder<'a> {
    fn find(&mut self, pointer: usize, template: u64) -> Option<&'a HeldKey> {
        let held = self.held;
        let matches = |entry: &HeldKey| (entry.pointer, entry.template) == (pointer, template);
        if held.get(self.cursor).is_some_and(matches) {
            self.cursor += 1;
            return Some(&held[self.cursor - 1]);
        }
        if self.sorted.len() != held.len() {
            self.sorted.clear();
            allocation::advisory(|| allocation::reserve(self.sorted, held.len())).ok()?;
            self.sorted.extend(0..held.len());
            self.sorted
                .sort_unstable_by_key(|&index| held[index].pointer);
        }
        let found = self
            .sorted
            .binary_search_by_key(&pointer, |&index| held[index].pointer)
            .ok()
            .map(|at| self.sorted[at])
            .filter(|&index| matches(&held[index]))?;
        self.cursor = found + 1;
        Some(&held[found])
    }
}

/// `arena[span]`.
fn text(arena: &[u8], span: native::Span) -> Option<&[u8]> {
    arena.get(span.start..span.start.checked_add(span.len)?)
}

/// Appends `value` to `bytes`, returning its span.
fn append(bytes: &mut Vec<u8>, value: &[u8]) -> Result<native::Span, &'static str> {
    allocation::reserve(bytes, value.len()).map_err(|_| "template byte arena allocation failed")?;
    let start = bytes.len();
    bytes.extend_from_slice(value);
    Ok(native::Span {
        start,
        len: value.len(),
    })
}

/// One hour's DivisionTemplates into `rows` and `bytes`, from this hour's
/// icon walk (`icons`, `outcome`, its display-name arena `names`), reading
/// only the names the store and the domain do not certify. Records the
/// hour's counts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn derive(
    memory: &impl engine_native::Memory,
    icons: &[native::IconRow],
    outcome: Outcome,
    names: &[u8],
    store: &mut KeyStore,
    changes: &dyn Changes,
    check: Check,
    rows: &mut Vec<native::Row>,
    bytes: &mut Vec<u8>,
) -> Outcome {
    rows.clear();
    bytes.clear();
    let before = changes.quiet();
    let quiet = before.is_some() && before == store.quiet;
    let mut counts = Counts {
        quiet,
        ..Counts::default()
    };
    let mut withdrawn = false;
    let derived = (|| -> Result<(), &'static str> {
        if let Outcome::Unavailable { reason } = outcome {
            return Err(reason);
        }
        let KeyStore {
            held,
            keys,
            sorted,
            next,
            next_keys,
            ..
        } = &mut *store;
        next.clear();
        next_keys.clear();
        let mut finder = Finder {
            held,
            sorted,
            cursor: 0,
        };
        allocation::reserve(rows, icons.len()).map_err(|_| "template row allocation failed")?;
        for icon in icons {
            if let Some(reason) = icon.unit_type_failure {
                return Err(reason);
            }
            let kept = finder.find(icon.pointer, icon.template).filter(|kept| {
                kept.certificate.is_some()
                    && !withdrawn
                    && (quiet || kept.certificate == changes.sequence(icon.pointer))
            });
            let display = icon.display_name.and_then(|display| {
                let literal = text(names, display.literal)?;
                match display.fallback_key {
                    Some(key) => Some((literal, Some(text(names, key)?))),
                    None => Some((literal, None)),
                }
            });
            // Quiet tokens spare ordinary carry lookups, but a sampled read
            // needs its own certificate immediately before acquiring names.
            let sampled = kept.is_some() && check.covers(icon.template >> 32);
            let verify_before = kept.filter(|_| sampled).and_then(|kept| {
                changes
                    .sequence(icon.pointer)
                    .filter(|certificate| Some(*certificate) == kept.certificate)
            });
            if sampled && verify_before.is_none() {
                withdrawn = true;
            }
            let (name, name_key, certificate) = match (display, kept) {
                // The walk read both names this hour.
                (Some((literal, Some(key))), _) => (
                    append(bytes, literal)?,
                    append(bytes, key)?,
                    changes.sequence(icon.pointer),
                ),
                (Some((literal, None)), Some(kept)) if !sampled => {
                    counts.reused += 1;
                    let key = &keys[kept.key.clone()];
                    (
                        append(bytes, literal)?,
                        append(bytes, key)?,
                        kept.certificate,
                    )
                }
                _ => {
                    counts.read += 1;
                    let start = bytes.len();
                    let names = native::copy_names_growing(memory, icon.pointer, bytes, start);
                    // The reads precede the fenced final certificate: a writer
                    // during any of them, failed reads included, withdraws
                    // rather than poisons.
                    fence(Ordering::Acquire);
                    let certificate = changes.sequence(icon.pointer);
                    if let Some(before) = verify_before {
                        counts.verified += 1;
                        if certificate != Some(before) {
                            withdrawn = true;
                        } else if match &names {
                            Ok([_, key]) => {
                                text(bytes, *key) != Some(&keys[kept.unwrap().key.clone()])
                            }
                            // A failed acquisition already refuses the key
                            // table and forgets the store. It supplies no key
                            // bytes to compare (allocation failure included).
                            Err(_) => false,
                        } {
                            counts.mismatches += 1;
                            changes.mismatch();
                        }
                    }
                    let [name, name_key] = names?;
                    (name, name_key, certificate)
                }
            };
            let key = text(bytes, name_key).ok_or("template byte arena invalid")?;
            allocation::advisory(|| {
                allocation::reserve(next, 1)?;
                allocation::reserve(next_keys, key.len())
            })
            .map_err(|_| "template key store allocation failed")?;
            let start = next_keys.len();
            next_keys.extend_from_slice(key);
            next.push(HeldKey {
                pointer: icon.pointer,
                template: icon.template,
                certificate,
                key: start..next_keys.len(),
            });
            rows.push(native::Row {
                template: icon.template,
                owner: icon.owner,
                name,
                name_key,
                unit_type: icon.unit_type,
            });
        }
        Ok(())
    })();
    let derived = match derived {
        Ok(()) => outcome,
        Err(reason) => {
            rows.clear();
            Outcome::Unavailable { reason }
        }
    };
    // Held only when the capture completed and no hooked writer ran across it.
    fence(Ordering::Acquire);
    match (derived, before) {
        (Outcome::Rows { .. }, Some(before)) if !withdrawn && changes.quiet() == Some(before) => {
            std::mem::swap(&mut store.held, &mut store.next);
            std::mem::swap(&mut store.keys, &mut store.next_keys);
            store.sorted.clear();
            store.quiet = Some(before);
        }
        _ => store.forget(),
    }
    crate::template_changes::record(
        &counts,
        matches!(derived, Outcome::Rows { .. }).then_some(store.held.len()),
    );
    derived
}

#[cfg(test)]
mod tests;
