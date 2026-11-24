use crate::fingerprint::{fingerprint, head};
use crate::generated::{CHECKS, DATA, EDGES, FUNCTIONS, KEYS, MASKS, VTABLES, WITNESSES};
use crate::image::Image;
use crate::{Verified, Withheld};
use crate::{Code, ORIGINAL, Rva};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub(crate) struct Function {
    pub reference: u32,
    pub len: u32,
    pub head: u64,
    pub hash: u64,
    pub masks: (u32, u32),
    pub tail: u32,
    pub tail_hash: u64,
    pub tail_masks: (u32, u32),
    pub pdata: bool,
    pub root: bool,
    pub edges: (u32, u32),
}

pub(crate) enum Edge {
    Field { from: u16, field: u32, end: u32 },
    Slot { vtable: u16, slot: u16 },
    After { function: u16, delta: u32 },
}

pub(crate) struct Vtable {
    pub name: u64,
    pub offset: u32,
    pub slots: u16,
    pub witnesses: (u32, u32),
    pub veto: bool,
}

pub(crate) struct Check {
    pub function: u16,
    pub field: u32,
    pub end: u32,
    pub target: Target,
}

pub(crate) enum Target {
    Function(u16),
    Data(u16),
    Vtable(u16),
    Internal(u32),
    Inside(u16, u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Text,
    Rdata,
    Data,
}

pub(crate) struct Data {
    pub section: Section,
    pub edges: (u32, u32),
}

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Code(u16),
    Vtable(u16),
    Data(u16),
}

pub(crate) struct Key {
    pub rva: u32,
    pub name: &'static str,
    pub kind: Kind,
}

pub(crate) fn key(key: Rva) -> Option<&'static Key> {
    index(key).map(|i| &KEYS[i])
}

fn index(key: Rva) -> Option<usize> {
    let rva = u32::try_from(key.0).ok()?;
    KEYS.binary_search_by_key(&rva, |k| k.rva).ok()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reason {
    Changed,
    Ambiguous,
    Unreached,
    Disagree,
    Section,
    Shape,
    Retargeted,
    Dependent,
    Unwitnessed,
    Unreadable,
    Layout(u16),
}

impl Reason {
    fn text(self) -> String {
        match self {
            Self::Changed => "its code differs from the reference build beyond moved addresses".into(),
            Self::Ambiguous => "its code occurs more than once and no unique reference reaches it".into(),
            Self::Unreached => "no located code references it".into(),
            Self::Disagree => "references from located code disagree".into(),
            Self::Section => "its reference leaves the expected section".into(),
            Self::Shape => "its vtable has a different number of slots".into(),
            Self::Retargeted => {
                "an operand reaches a different function or global than in the reference build".into()
            }
            Self::Dependent => "an operand reaches a function or global that is withheld".into(),
            Self::Unwitnessed => "no constructor witnesses its class layout on this build".into(),
            Self::Unreadable => "part of the running image could not be read".into(),
            Self::Layout(vtable) => format!(
                "struct layouts are unverified on this build: the constructor of {} changed",
                vtable_name(vtable)
            ),
        }
    }
}

fn vtable_name(index: u16) -> &'static str {
    KEYS.iter()
        .find(|k| matches!(k.kind, Kind::Vtable(v) if v == index))
        .map_or("a class", |k| k.name)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Summary {
    pub functions: usize,
    pub functions_found: usize,
    pub keys: usize,
    pub keys_verified: usize,
    pub code_withheld: usize,
    pub vtables_withheld: usize,
    pub data_withheld: usize,
    pub layout_checked: bool,
    pub layout_verified: bool,
    pub elapsed: Duration,
}

struct RvaMap {
    entries: Box<[(u32, u32)]>,
    mask: usize,
    shift: u32,
}

impl RvaMap {
    const EMPTY_KEY: u32 = u32::MAX;

    fn empty() -> Self {
        Self {
            entries: Box::new([]),
            mask: 0,
            shift: 0,
        }
    }

    fn from_pairs(count: usize, pairs: impl Iterator<Item = (u32, u32)>) -> Self {
        if count == 0 {
            return Self::empty();
        }
        let capacity = (count.saturating_mul(2).max(16)).next_power_of_two();
        let mask = capacity - 1;
        let shift = 32 - capacity.trailing_zeros();
        let mut entries = vec![(Self::EMPTY_KEY, 0_u32); capacity].into_boxed_slice();
        for (key, value) in pairs {
            if key == Self::EMPTY_KEY {
                continue;
            }
            let mut slot = (key.wrapping_mul(0x9E37_79B1) >> shift) as usize & mask;
            loop {
                let stored = entries[slot].0;
                if stored == Self::EMPTY_KEY {
                    entries[slot] = (key, value);
                    break;
                }
                if stored == key {
                    break;
                }
                slot = (slot + 1) & mask;
            }
        }
        Self {
            entries,
            mask,
            shift,
        }
    }

    #[inline]
    fn get(&self, key: u32) -> Option<u32> {
        if key == Self::EMPTY_KEY || self.entries.is_empty() {
            return None;
        }
        let mask = self.mask;
        let mut slot = (key.wrapping_mul(0x9E37_79B1) >> self.shift) as usize & mask;
        loop {
            let (stored, value) = self.entries[slot];
            if stored == key {
                return Some(value);
            }
            if stored == Self::EMPTY_KEY {
                return None;
            }
            slot = (slot + 1) & mask;
        }
    }
}

pub struct Located {
    base: usize,
    identity: Option<[u8; 32]>,
    functions: Vec<Result<u32, Reason>>,
    vtables: Vec<Result<u32, Reason>>,
    data: Vec<Result<u32, Reason>>,
    by_live: Vec<(u32, u32, u16)>,
    live_code_min: u32,
    live_code_max: u32,
    globals: Vec<(u32, u32)>,
    resolved_keys: RvaMap,
    globals_map: RvaMap,
    originals: Vec<([u8; ORIGINAL], u8)>,
    layout: Result<(), Reason>,
    messages: Vec<Option<String>>,
    summary: Summary,
}

fn slice<'a>(table: &'a [u32], (start, count): (u32, u32)) -> &'a [u32] {
    &table[start as usize..(start + count) as usize]
}

impl Located {
    pub fn locate(image: &Image<'_>, layout_checked: bool) -> Self {
        let started = Instant::now();
        image.begin();
        let pdata = pdata(image);
        let ends: HashMap<u32, u32> = pdata.iter().copied().collect();
        let mut functions: Vec<Result<u32, Reason>> = vec![Err(Reason::Unreached); FUNCTIONS.len()];

        let mut wanted: HashMap<u32, Vec<u16>> = HashMap::new();
        for (index, f) in FUNCTIONS.iter().enumerate() {
            if f.root {
                wanted.entry(f.len).or_default().push(index as u16);
            }
        }
        let mut hits: Vec<(u32, u32)> = vec![(0, 0); FUNCTIONS.len()];
        for &(begin, end) in &pdata {
            let Some(candidates) = wanted.get(&(end - begin)) else { continue };
            let Some(bytes) = image.bytes(begin, (end - begin) as usize) else { continue };
            for &index in candidates {
                let f = &FUNCTIONS[index as usize];
                let mask = slice(&MASKS, f.masks);
                if head(&bytes, mask) == f.head && fingerprint(&bytes, mask) == f.hash {
                    let hit = &mut hits[index as usize];
                    hit.0 = begin;
                    hit.1 += 1;
                }
            }
        }
        for (index, f) in FUNCTIONS.iter().enumerate() {
            if f.root {
                functions[index] = match hits[index] {
                    (start, 1) => Ok(start),
                    (_, 0) => Err(Reason::Changed),
                    _ => Err(Reason::Ambiguous),
                };
            }
        }

        let mut vtables = rtti(image);
        let by_rtti = vtables.clone();
        for (index, v) in VTABLES.iter().enumerate() {
            if let Ok(table) = vtables[index]
                && slot_count(image, table) != u32::from(v.slots)
            {
                vtables[index] = Err(Reason::Shape);
            }
        }

        let roots = functions;
        let mut banned: Vec<Option<Reason>> = vec![None; FUNCTIONS.len()];
        let field = |from: u16, at: u32, end: u32, functions: &[Result<u32, Reason>]| -> Option<u32> {
            let start = *functions[from as usize].as_ref().ok()?;
            let disp = image.i32(start.checked_add(at)?)?;
            start.checked_add(end)?.checked_add_signed(disp)
        };
        let follow = |edge: &Edge, functions: &[Result<u32, Reason>]| -> Option<u32> {
            match *edge {
                Edge::After { function, delta } => functions[function as usize].as_ref().ok()?.checked_add(delta),
                Edge::Field { from, field: at, end } => field(from, at, end, functions),
                Edge::Slot { vtable, slot } => {
                    let table = *vtables[vtable as usize].as_ref().ok()?;
                    image.pointer(table.checked_add(8 * u32::from(slot))?)
                }
            }
        };
        let (functions, data) = loop {
            let mut functions: Vec<Result<u32, Reason>> = roots
                .iter()
                .zip(&banned)
                .map(|(root, ban)| ban.map_or(*root, Err))
                .collect();
            let mut changed = true;
            while changed {
                changed = false;
                for (index, f) in FUNCTIONS.iter().enumerate() {
                    if functions[index].is_ok()
                        || f.edges.1 == 0
                        || functions[index] == Err(Reason::Changed)
                        || banned[index].is_some()
                    {
                        continue;
                    }
                    let mut found = None;
                    let mut disagree = false;
                    for edge in &EDGES[f.edges.0 as usize..(f.edges.0 + f.edges.1) as usize] {
                        let Some(target) = follow(edge, &functions) else { continue };
                        if f.pdata && ends.get(&target) != Some(&(target + f.len)) {
                            continue;
                        }
                        let Some(bytes) = image.bytes(target, f.len as usize) else { continue };
                        if fingerprint(&bytes, slice(&MASKS, f.masks)) != f.hash {
                            continue;
                        }
                        match found {
                            None => found = Some(target),
                            Some(t) if t != target => disagree = true,
                            _ => {}
                        }
                    }
                    if disagree {
                        functions[index] = Err(Reason::Disagree);
                    } else if let Some(target) = found {
                        functions[index] = Ok(target);
                        changed = true;
                    }
                }
            }

            let mut failed = false;
            for (index, f) in FUNCTIONS.iter().enumerate() {
                let Ok(start) = functions[index] else { continue };
                if f.root || banned[index].is_some() {
                    continue;
                }
                let elsewhere = EDGES[f.edges.0 as usize..(f.edges.0 + f.edges.1) as usize].iter().any(|edge| {
                    follow(edge, &functions).is_some_and(|target| {
                        target != start
                            && (!f.pdata || ends.get(&target) == Some(&(target + f.len)))
                            && image
                                .bytes(target, f.len as usize)
                                .is_some_and(|bytes| fingerprint(&bytes, slice(&MASKS, f.masks)) == f.hash)
                    })
                });
                if elsewhere {
                    banned[index] = Some(Reason::Disagree);
                    failed = true;
                }
            }

            let data: Vec<Result<u32, Reason>> = DATA
                .iter()
                .map(|d| {
                    let mut found = None;
                    for edge in &EDGES[d.edges.0 as usize..(d.edges.0 + d.edges.1) as usize] {
                        let Some(target) = follow(edge, &functions) else { continue };
                        match found {
                            None => found = Some(target),
                            Some(t) if t != target => return Err(Reason::Disagree),
                            _ => {}
                        }
                    }
                    let target = found.ok_or(Reason::Unreached)?;
                    let section = match image.section_of(target) {
                        Some(b".text") => Some(Section::Text),
                        Some(b".rdata") => Some(Section::Rdata),
                        Some(b".data") => Some(Section::Data),
                        _ => None,
                    };
                    if section == Some(d.section) { Ok(target) } else { Err(Reason::Section) }
                })
                .collect();

            for check in CHECKS {
                let source = check.function as usize;
                let Ok(start) = functions[source] else { continue };
                let operand = || field(check.function, check.field, check.end, &functions);
                let verdict = match check.target {
                    Target::Internal(offset) => {
                        let stored = start.checked_add(check.field).and_then(|at| image.u32(at));
                        (stored != start.checked_add(offset)).then_some(Reason::Retargeted)
                    }
                    Target::Function(f) => reached(functions[f as usize], operand()),
                    Target::Data(d) => reached(data[d as usize], operand()),
                    Target::Vtable(v) => reached(by_rtti[v as usize], operand()),
                    Target::Inside(f, offset) => reached(
                        functions[f as usize].and_then(|start| start.checked_add(offset).ok_or(Reason::Unreached)),
                        operand(),
                    ),
                };
                if let Some(reason) = verdict
                    && banned[source].is_none()
                {
                    banned[source] = Some(reason);
                    failed = true;
                }
            }
            if !failed {
                break (functions, data);
            }
        };

        let mut layout = Ok(());
        if !layout_checked {
            for (index, v) in VTABLES.iter().enumerate() {
                let witnesses = slice(&WITNESSES, v.witnesses);
                if witnesses.is_empty() && vtables[index].is_ok() {
                    vtables[index] = Err(Reason::Unwitnessed);
                }
                if witnesses.iter().any(|&w| functions[w as usize].is_err()) {
                    if !v.veto && vtables[index].is_ok() {
                        vtables[index] = Err(Reason::Layout(index as u16));
                    } else if v.veto && layout.is_ok() {
                        layout = Err(Reason::Layout(index as u16));
                    }
                }
            }
        }

        let mut by_live: Vec<(u32, u32, u16)> = functions
            .iter()
            .enumerate()
            .filter_map(|(i, f)| f.ok().map(|s| (s, s + FUNCTIONS[i].len, i as u16)))
            .collect();
        by_live.sort_unstable();
        let originals = KEYS
            .iter()
            .map(|k| {
                let Kind::Code(f) = k.kind else { return ([0; ORIGINAL], 0) };
                let Ok(start) = functions[f as usize] else { return ([0; ORIGINAL], 0) };
                let function = &FUNCTIONS[f as usize];
                let offset = k.rva - function.reference;
                let tail = image
                    .bytes(start + function.len, function.tail as usize)
                    .filter(|bytes| function.tail > 0 && fingerprint(bytes, slice(&MASKS, function.tail_masks)) == function.tail_hash)
                    .map_or(0, |_| function.tail);
                let verified = (function.len + tail - offset).min(ORIGINAL as u32) as usize;
                let mut original = [0; ORIGINAL];
                if let Some(bytes) = image.bytes(start + offset, verified) {
                    original[..verified].copy_from_slice(&bytes);
                    (original, verified as u8)
                } else {
                    ([0; ORIGINAL], 0)
                }
            })
            .collect::<Vec<_>>();

        let (mut functions, mut data, mut originals) = (functions, data, originals);
        if image.unreadable() {
            functions.fill(Err(Reason::Unreadable));
            vtables.fill(Err(Reason::Unreadable));
            data.fill(Err(Reason::Unreadable));
            by_live.clear();
            originals.fill(([0; ORIGINAL], 0));
            layout = Err(Reason::Unreadable);
        }
        let live_code_min = by_live.first().map_or(u32::MAX, |&(start, _, _)| start);
        let live_code_max = by_live.iter().map(|&(_, end, _)| end).max().unwrap_or(0);
        let mut located = Self {
            base: image.base,
            identity: None,
            functions,
            vtables,
            data,
            by_live,
            live_code_min,
            live_code_max,
            globals: Vec::new(),
            resolved_keys: RvaMap::empty(),
            globals_map: RvaMap::empty(),
            originals,
            layout,
            messages: Vec::new(),
            summary: Summary::default(),
        };
        located.globals = KEYS
            .iter()
            .filter_map(|k| {
                let live = match k.kind {
                    Kind::Vtable(v) => located.vtables[v as usize],
                    Kind::Data(d) => located.data[d as usize],
                    Kind::Code(_) => return None,
                };
                live.ok().map(|live| (live, k.rva))
            })
            .collect();
        located.globals.sort_unstable();
        located.globals_map = RvaMap::from_pairs(
            located.globals.len(),
            located.globals.iter().copied(),
        );
        if located.layout.is_ok() {
            let pairs = KEYS.iter().filter_map(|k| {
                let live = match k.kind {
                    Kind::Code(f) => located.code_address(f, k.rva),
                    Kind::Vtable(v) => located.vtables[v as usize].ok(),
                    Kind::Data(d) => located.data[d as usize].ok(),
                }?;
                Some((k.rva, live))
            });
            located.resolved_keys = RvaMap::from_pairs(KEYS.len(), pairs);
        }
        located.messages = KEYS
            .iter()
            .map(|k| located.why(k).map(|r| Withheld::new(k.name, r.text()).to_string()))
            .collect();
        let mut summary = Summary {
            functions: FUNCTIONS.len(),
            functions_found: located.functions.iter().filter(|f| f.is_ok()).count(),
            keys: KEYS.len(),
            layout_checked,
            layout_verified: located.layout.is_ok(),
            ..Summary::default()
        };
        for k in KEYS {
            match (located.why(&k), k.kind) {
                (None, _) => summary.keys_verified += 1,
                (Some(_), Kind::Code(_)) => summary.code_withheld += 1,
                (Some(_), Kind::Vtable(_)) => summary.vtables_withheld += 1,
                (Some(_), Kind::Data(_)) => summary.data_withheld += 1,
            }
        }
        summary.elapsed = started.elapsed();
        located.summary = summary;
        located
    }

    pub fn summary(&self) -> Summary {
        self.summary
    }

    pub(crate) fn identity(&self) -> Option<&[u8; 32]> {
        self.identity.as_ref()
    }

    pub(crate) fn set_identity(&mut self, sha256: [u8; 32]) {
        self.identity = Some(sha256);
    }

    pub fn base(&self) -> usize {
        self.base
    }

    pub fn withheld(&self) -> impl Iterator<Item = (Rva, Withheld)> + '_ {
        KEYS.iter().filter_map(|k| {
            self.why(k)
                .map(|reason| (Rva(k.rva as usize), Withheld::new(k.name, reason.text())))
        })
    }

    pub fn moved(&self) -> impl Iterator<Item = (Rva, u32)> + '_ {
        KEYS.iter().filter_map(|k| {
            let live = match k.kind {
                Kind::Code(f) => self.code_address(f, k.rva),
                Kind::Vtable(v) => self.vtables[v as usize].ok(),
                Kind::Data(d) => self.data[d as usize].ok(),
            }?;
            (live != k.rva).then_some((Rva(k.rva as usize), live))
        })
    }

    pub fn withheld_key(&self, key: Rva) -> Option<Withheld> {
        match crate::locate::key(key) {
            Some(k) => self.why(k).map(|reason| Withheld::new(k.name, reason.text())),
            None => self.resolve(self.base, key).err(),
        }
    }

    fn why(&self, k: &Key) -> Option<Reason> {
        match k.kind {
            Kind::Code(f) => self.functions[f as usize].err().or(self.layout.err()),
            Kind::Vtable(v) => self.vtables[v as usize].err().or(self.layout.err()),
            Kind::Data(d) => self.data[d as usize].err().or(self.layout.err()),
        }
    }

    #[inline]
    pub(crate) fn resolve(&self, base: usize, key: Rva) -> Verified<usize> {
        if base == self.base
            && let Ok(rva) = u32::try_from(key.0)
            && let Some(live) = self.resolved_keys.get(rva)
        {
            return Ok(self.base + live as usize);
        }
        self.resolve_slow(base, key)
    }

    #[cold]
    #[inline(never)]
    fn resolve_slow(&self, base: usize, key: Rva) -> Verified<usize> {
        if base != self.base {
            return Err(Withheld::new(crate::name(key), "image base differs from the located module"));
        }
        let rva = u32::try_from(key.0)
            .map_err(|_| Withheld::new(crate::name(key), "not a position in the reference build"))?;
        let live = match crate::locate::key(key) {
            Some(k) => {
                if let Some(reason) = self.why(k) {
                    return Err(Withheld::new(k.name, reason.text()));
                }
                match k.kind {
                    Kind::Code(f) => self.code_address(f, rva),
                    Kind::Vtable(v) => self.vtables[v as usize].ok(),
                    Kind::Data(d) => self.data[d as usize].ok(),
                }
            }
            None => {
                let i = FUNCTIONS.partition_point(|f| f.reference <= rva).checked_sub(1);
                match i {
                    Some(i) if rva < FUNCTIONS[i].reference + FUNCTIONS[i].len => {
                        if let Err(reason) = self.functions[i] {
                            return Err(Withheld::new(crate::name(key), reason.text()));
                        }
                        self.code_address(i as u16, rva)
                    }
                    _ => {
                        return Err(Withheld::new(
                            crate::name(key),
                            "not a generated site; run tools/sites.py generate",
                        ));
                    }
                }
            }
        };
        live.map(|rva| self.base + rva as usize)
            .ok_or_else(|| Withheld::new(crate::name(key), "unresolved"))
    }

    fn code_address(&self, function: u16, rva: u32) -> Option<u32> {
        let start = self.functions[function as usize].ok()?;
        Some(start + (rva - FUNCTIONS[function as usize].reference))
    }

    pub(crate) fn code(&self, base: usize, key: Rva) -> Verified<Code> {
        let address = self.resolve(base, key)?;
        let k = crate::locate::key(key)
            .ok_or_else(|| Withheld::new(crate::name(key), "not a generated code site"))?;
        let Kind::Code(f) = k.kind else {
            return Err(Withheld::new(k.name, "not a code site"));
        };
        let (original, verified) = index(key).map_or(([0; ORIGINAL], 0), |i| self.originals[i]);
        let function = self.base + self.functions[f as usize].unwrap_or(0) as usize;
        Ok(Code {
            address,
            function,
            end: function + FUNCTIONS[f as usize].len as usize,
            original,
            verified: usize::from(verified),
        })
    }

    pub fn layouts(&self) -> Verified<()> {
        self.layout
            .map_err(|reason| Withheld::new("struct layouts", reason.text()))
    }

    pub fn message(&self, key: Rva) -> Option<&str> {
        let i = index(key).or_else(|| {
            let rva = u32::try_from(key.0).ok()?;
            let f = FUNCTIONS.partition_point(|f| f.reference <= rva).checked_sub(1)?;
            (rva < FUNCTIONS[f].reference + FUNCTIONS[f].len).then_some(())?;
            KEYS.iter().position(|k| matches!(k.kind, Kind::Code(c) if usize::from(c) == f))
        })?;
        self.messages[i].as_deref()
    }

    #[inline]
    pub(crate) fn reference(&self, base: usize, address: usize) -> Option<Rva> {
        let rva = u32::try_from(address.checked_sub(base)?).ok()?;
        if base != self.base || self.layout.is_err() {
            return None;
        }
        if rva >= self.live_code_min && rva < self.live_code_max {
            let i = self.by_live.partition_point(|&(s, _, _)| s <= rva).checked_sub(1);
            if let Some(i) = i {
                let (start, end, f) = self.by_live[i];
                if rva < end {
                    return Some(Rva((FUNCTIONS[f as usize].reference + (rva - start)) as usize));
                }
            }
        }
        self.globals_map.get(rva).map(|key| Rva(key as usize))
    }
}

fn reached(target: Result<u32, Reason>, operand: Option<u32>) -> Option<Reason> {
    match target {
        Err(_) => Some(Reason::Dependent),
        Ok(target) => (operand != Some(target)).then_some(Reason::Retargeted),
    }
}

fn pdata(image: &Image<'_>) -> Vec<(u32, u32)> {
    let Some((_, bytes)) = image.section(b".pdata") else { return Vec::new() };
    let mut out: Vec<(u32, u32)> = bytes
        .chunks_exact(12)
        .map(|e| {
            (
                u32::from_le_bytes([e[0], e[1], e[2], e[3]]),
                u32::from_le_bytes([e[4], e[5], e[6], e[7]]),
            )
        })
        .take_while(|&(b, _)| b != 0)
        .filter(|&(b, e)| e > b)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn rtti(image: &Image<'_>) -> Vec<Result<u32, Reason>> {
    let mut want: HashMap<(u64, u32), u16> = HashMap::with_capacity(VTABLES.len());
    for (i, v) in VTABLES.iter().enumerate() {
        want.insert((v.name, v.offset), i as u16);
    }
    let mut out = vec![Err(Reason::Changed); VTABLES.len()];
    let Some((rdata, bytes)) = image.section(b".rdata") else { return out };
    let mut locators: HashMap<u32, u16> = HashMap::new();
    let mut counts = vec![0_u32; VTABLES.len()];
    for (i, w) in bytes.chunks_exact(4).enumerate() {
        if w != [1, 0, 0, 0] {
            continue;
        }
        let at = rdata + 4 * i as u32;
        if image.u32(at + 20) != Some(at) {
            continue;
        }
        let (Some(descriptor), Some(offset)) = (image.u32(at + 12), image.u32(at + 4)) else { continue };
        let Some(name) = image.cstring(descriptor + 16) else { continue };
        if let Some(&index) = want.get(&(fingerprint(&name, &[]), offset)) {
            locators.insert(at, index);
        }
    }
    let first = (8 - rdata % 8) % 8;
    for (i, q) in bytes[first as usize..].chunks_exact(8).enumerate() {
        let value = u64::from_le_bytes(q.try_into().unwrap_or([0; 8]));
        let Some(target) = value.checked_sub(image.pointer_base()) else { continue };
        let Ok(target) = u32::try_from(target) else { continue };
        if let Some(&index) = locators.get(&target) {
            counts[index as usize] += 1;
            out[index as usize] = Ok(rdata + first + 8 * i as u32 + 8);
        }
    }
    for (i, c) in counts.iter().enumerate() {
        if *c > 1 {
            out[i] = Err(Reason::Ambiguous);
        }
    }
    out
}

fn slot_count(image: &Image<'_>, table: u32) -> u32 {
    let mut n = 0;
    while let Some(target) = image.pointer(table + 8 * n) {
        if image.section_of(target) != Some(b".text") {
            break;
        }
        n += 1;
        if n > 4096 {
            break;
        }
    }
    n
}
