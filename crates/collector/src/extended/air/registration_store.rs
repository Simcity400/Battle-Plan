//! AirBaseRegistrations log segments held per state air base between hours
//! ([`crate::air_base_carry_changes`]).
//!
//! A state base's registration subtree (its slot vector and dense list, its
//! containers, their pools and the pools' wings) is read into the shared air
//! bases log every hour unless its held segment carries it: the spans and
//! bytes its latest fresh read logged, which the replay's walk reads exactly
//! as it read them then. A segment is carried only while the base is at the
//! same address, its watch certificate (epoch and sequence) is the one taken
//! before that read, so no hooked writer of anything the subtree reads
//! touched it since, and the hour takes its slots as that read did (dense
//! plan or whole scan). Its container countries' links are not held: the
//! hour resolves each again, as the walk would, and a link that no longer
//! resolves to the held slot reads the base afresh. Host bases, whose carrier
//! fuel changes every hour, are always read.
//!
//! The store follows the AirBases job's loan and holds only the bases the
//! latest complete pass visited, so its size follows live demand. Allocation
//! is advisory: a base whose segment cannot be held is read afresh next hour.
//!
//! Each held segment has an id, unique in the process, that the job reports
//! for every base whose subtree in the hour's log is that segment's (carried,
//! or read afresh and held); a fresh read that a comparison found equal to
//! the held segment keeps its id. The decode worker keeps each base's walked
//! output under its segment's id ([`super::DecodedRegistrations`]).
use crate::raw::Span;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SEGMENT: AtomicU64 = AtomicU64::new(1);

/// One base's segment as its fresh read logged it.
#[derive(Debug)]
struct Held {
    pointer: usize,
    certificate: (u64, u64),
    /// The read took the dense plan's slots, not the whole scan's.
    planned: bool,
    /// Its successful spans but the link takes, `start` into `bytes`.
    spans: Vec<Span>,
    bytes: Vec<u8>,
    /// Each nonzero container country and the slot its link resolved to.
    links: Vec<(u32, u32)>,
    /// Registration rows and wing handles the subtree demanded.
    demand: [usize; 2],
    /// The pass that last visited the base.
    pass: u64,
    segment: u64,
}

/// What one pass did with its state bases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub carried: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
}

#[derive(Debug, Default)]
pub(crate) struct RegistrationStore {
    held: HashMap<u64, Held>,
    pass: u64,
    counts: Counts,
    /// State bases this pass visited, and those the latest complete pass
    /// visited, which size the watches whether or not they were held.
    visited: usize,
    bases: usize,
    /// Allocated bytes as of the last finish, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next pass drops every segment.
    forgotten: bool,
}

/// A held segment a pass may carry.
pub(crate) struct Carried<'a> {
    pub spans: &'a [Span],
    pub bytes: &'a [u8],
    pub links: &'a [(u32, u32)],
    pub demand: [usize; 2],
    pub segment: u64,
}

/// One base's fresh read, as the log holds it.
pub(crate) struct Fresh<'a> {
    pub spans: &'a [Span],
    /// The log's bytes from the first byte the read logged.
    pub bytes: &'a [u8],
    /// The log offset of `bytes[0]`.
    pub start: usize,
    pub links: &'a [(u32, u32)],
    pub demand: [usize; 2],
    /// The span range of the country link table, whose takes are not held.
    pub link_table: std::ops::Range<usize>,
    /// The containers and wings the read reached, whose spans are held as
    /// consumed.
    pub containers: &'a [usize],
    pub wings: &'a [usize],
}

impl Fresh<'_> {
    fn held(&self) -> impl Iterator<Item = &Span> {
        self.spans
            .iter()
            .filter(|span| !(span.len == 4 && self.link_table.contains(&span.addr)))
    }

    /// Appends the bytes of `span` the registration rows consume; bytes the
    /// walk reads but no row consumes, which change without a registration
    /// writer, are held as zero:
    /// - a container's span (`+0xB0..+0xF8`) keeps its country and capacity
    ///   words; its sortie factor, fuel demand and allocation words are
    ///   rewritten every hour (the hourly air sortie update from the hourly
    ///   strategic-air stage, and the hourly air fuel update) and only a host
    ///   base's carrier observer reads them;
    /// - a wing's identity (`+0x00..+0x20`) keeps its two vtables and its
    ///   handle; `+0x08..+0x10` is its `CSelectable` state (the selected
    ///   byte `+0x0C`, set by the selection writer when the player selects
    ///   it), which the walk does not check.
    fn consumed(&self, span: &Span, output: &mut Vec<u8>) {
        use engine::{cair_base::CONTAINER_SPAN, ccountry_air_container as container};
        let start = span.start as usize - self.start;
        let at = output.len();
        output.extend_from_slice(&self.bytes[start..start + span.len as usize]);
        let kept = &mut output[at..];
        let names = |objects: &[usize], offset: usize| {
            objects
                .iter()
                .any(|&pointer| pointer.checked_add(offset) == Some(span.addr))
        };
        if span.len as usize == CONTAINER_SPAN && names(self.containers, container::COUNTRY) {
            let capacity = container::CAPACITY_RAW - container::COUNTRY;
            kept[4..capacity].fill(0);
            kept[capacity + 8..].fill(0);
        } else if span.len == 32 && names(self.wings, 0) {
            kept[8..engine::cair_wing::SECONDARY].fill(0);
        }
    }
}

/// Where a fresh read first differed from its held segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Difference {
    /// What differed: the demand, the links, the span list, or a span or its
    /// bytes.
    pub what: &'static str,
    /// The span's address and length, for a span.
    pub address: usize,
    pub len: u32,
    /// The first differing byte within the span.
    pub byte: usize,
}

impl RegistrationStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Nothing held carries anything: the next pass reads every base.
    /// Allocation-free for the game thread.
    pub(crate) fn forget(&mut self) {
        self.forgotten = true;
    }

    /// As of the last finish, so the game thread need not measure.
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn counts(&self) -> Counts {
        self.counts
    }

    pub(crate) fn begin(&mut self) {
        if std::mem::take(&mut self.forgotten) {
            self.held.clear();
        }
        self.pass += 1;
        self.counts = Counts::default();
        self.visited = 0;
    }

    /// The pass reaches a state base.
    pub(crate) fn visit(&mut self) {
        self.visited += 1;
    }

    /// The segment of `handle` this pass may carry: held from a read of the
    /// base at `pointer` under `certificate`, taking its slots as `planned`
    /// says. Marks the base visited.
    pub(crate) fn carryable(
        &mut self,
        handle: u64,
        pointer: usize,
        certificate: Option<(u64, u64)>,
        planned: bool,
    ) -> Option<Carried<'_>> {
        let pass = self.pass;
        let held = self.held.get_mut(&handle)?;
        if held.pointer != pointer
            || Some(held.certificate) != certificate
            || held.planned != planned
        {
            return None;
        }
        held.pass = pass;
        Some(Carried {
            spans: &held.spans,
            bytes: &held.bytes,
            links: &held.links,
            demand: held.demand,
            segment: held.segment,
        })
    }

    /// The id of the segment this pass held for `handle`, carried or read.
    pub(crate) fn segment(&self, handle: u64) -> Option<u64> {
        self.held
            .get(&handle)
            .filter(|held| held.pass == self.pass)
            .map(|held| held.segment)
    }

    pub(crate) fn carried(&mut self) {
        self.counts.carried += 1;
    }

    /// Holds `fresh` for `handle` under `certificate`, first comparing it
    /// with the segment held under the same certificate when `compare`:
    /// returns where they differ, a write no hook observed. A read with a
    /// failed span, or one without a certificate, is not held.
    pub(crate) fn hold(
        &mut self,
        handle: u64,
        pointer: usize,
        certificate: Option<(u64, u64)>,
        planned: bool,
        fresh: &Fresh<'_>,
        compare: bool,
    ) -> Option<Difference> {
        self.counts.read += 1;
        let previous = self.held.remove(&handle);
        let certificate = certificate.filter(|_| fresh.spans.iter().all(|span| span.ok))?;
        let mut held = Held {
            pointer,
            certificate,
            planned,
            spans: Vec::new(),
            bytes: Vec::new(),
            links: Vec::new(),
            demand: fresh.demand,
            pass: self.pass,
            segment: 0,
        };
        let spans = fresh.held().count();
        let bytes = fresh.held().map(|span| span.len as usize).sum::<usize>();
        let stored = engine::allocation::advisory(|| {
            engine::allocation::reserve_map(&mut self.held, 1)?;
            engine::allocation::reserve(&mut held.spans, spans)?;
            engine::allocation::reserve(&mut held.bytes, bytes)?;
            engine::allocation::reserve(&mut held.links, fresh.links.len())
        });
        if stored.is_err() {
            return None;
        }
        for span in fresh.held() {
            held.spans.push(Span {
                start: held.bytes.len() as u32,
                ..*span
            });
            fresh.consumed(span, &mut held.bytes);
        }
        held.links.extend_from_slice(fresh.links);
        let compared = previous.filter(|previous| {
            compare
                && previous.pointer == pointer
                && previous.certificate == certificate
                && previous.planned == planned
        });
        self.counts.verified += u64::from(compared.is_some());
        let difference = compared
            .as_ref()
            .and_then(|previous| difference(previous, &held));
        self.counts.mismatches += u64::from(difference.is_some());
        // An equal read is the held segment byte for byte as consumed.
        held.segment = match compared {
            Some(previous) if difference.is_none() => previous.segment,
            _ => NEXT_SEGMENT.fetch_add(1, Ordering::Relaxed),
        };
        self.held.insert(handle, held);
        difference
    }

    /// The state bases the latest complete pass visited.
    pub(crate) fn bases(&self) -> usize {
        self.bases
    }

    /// Ends the pass: a complete one keeps the bases it visited, any other
    /// keeps nothing.
    pub(crate) fn finish(&mut self, complete: bool) {
        if complete {
            let pass = self.pass;
            self.held.retain(|_, held| held.pass == pass);
            self.bases = self.visited;
        } else {
            self.held.clear();
        }
        self.bytes = self.held.capacity() * size_of::<(u64, Held)>()
            + self
                .held
                .values()
                .map(|held| {
                    held.spans.capacity() * size_of::<Span>()
                        + held.bytes.capacity()
                        + held.links.capacity() * size_of::<(u32, u32)>()
                })
                .sum::<usize>();
    }
}

/// Where `fresh`, a read held as consumed, first differs from `held`.
fn difference(held: &Held, fresh: &Held) -> Option<Difference> {
    let whole = |what| Difference {
        what,
        address: 0,
        len: 0,
        byte: 0,
    };
    if held.demand != fresh.demand {
        return Some(whole("registration and wing demand"));
    }
    if held.links != fresh.links {
        return Some(whole("container country links"));
    }
    if held.spans.len() != fresh.spans.len() {
        return Some(whole("span list"));
    }
    held.spans.iter().zip(&fresh.spans).find_map(|(old, new)| {
        if (Span { start: 0, ..*old }) != (Span { start: 0, ..*new }) {
            return Some(Difference {
                what: "span",
                address: new.addr,
                len: new.len,
                byte: 0,
            });
        }
        let (old_bytes, new_bytes) = (
            &held.bytes[old.start as usize..][..old.len as usize],
            &fresh.bytes[new.start as usize..][..new.len as usize],
        );
        old_bytes
            .iter()
            .zip(new_bytes)
            .position(|(old, new)| old != new)
            .map(|byte| Difference {
                what: "span bytes",
                address: new.addr,
                len: new.len,
                byte,
            })
    })
}
