//! One declaration per cause: the variant, the slot its counter lives at, and
//! the leaf that slot publishes under.
//!
//! THE DEFECT, which is lane TV's R7
//! (`runs/tv_capture_error_taxonomy/FIX.md` section 10). Each of this crate's
//! cause taxonomies sized a counter array from a hand-written `ALL` and
//! indexed it with the implicit discriminant. A cause appended with its
//! producer and its exhaustive name arm and no `ALL` row compiled: the array
//! stayed one short, and the new cause indexed past it, was dropped by a
//! `get`, or was never published by a consumer's loop over `ALL`.
//!
//! `ALL`, the count and the slot come off one column now. The generated slot
//! match has no wildcard arm, so a variant missing from the column is E0004
//! and a column row that is not a variant does not resolve -- the enum and the
//! column are a bijection at compile time without either being generated from
//! the other, which is the shape lane TV landed for `CaptureError`.
//!
//! The enums stay hand-written for the reason TV's section 2 gives: their
//! per-variant documentation is what a reader of a refusal cause needs, and a
//! macro that carried it would buy nothing the non-exhaustive match does not
//! already give.

/// A cause taxonomy whose counter slot is its own discriminant.
///
/// Four forms. The first is for a taxonomy whose leaves live in the consumer
/// that publishes them. The rest carry the leaf here, under a prefix the whole
/// taxonomy shares, and a leaf must spell its variant.
///
/// `rename` is for a taxonomy where some leaves deliberately do not: it names
/// ONE prefix substitution for the whole taxonomy, and a row marked `renamed`
/// is held to its own name under it. The substitution is per taxonomy rather
/// than per row because a row that could write its own replacement could write
/// another cause's leaf and satisfy the check (Codex round 2). A taxonomy with
/// no `rename` admits no `renamed` row at all -- that row matches no form.
///
/// `leaves` additionally publishes the bare leaves as an array indexed by the
/// slot, for a cross-tab whose column headings are those leaves.
macro_rules! indexed_causes {
    (
        $enum:ident, all = $all:ident, count = $count:ident, slot = $slot:ident;
        $( $variant:ident = $index:literal, )+
    ) => {
        $crate::cause_column::indexed_causes_column!(
            $enum, $all, $count, $slot; $($variant = $index,)+
        );
    };

    (
        $enum:ident, all = $all:ident, count = $count:ident, slot = $slot:ident,
        unit_name = $name_fn:ident, prefix = $prefix:literal;
        $( $variant:ident = $index:literal => $leaf:literal, )+
    ) => {
        $crate::cause_column::indexed_causes_column!(
            $enum, $all, $count, $slot; $($variant = $index,)+
        );
        $crate::cause_column::indexed_causes_names!(
            $enum, $name_fn, $prefix; $($variant => $leaf,)+
        );

        const _: () = {
            $(assert!(
                ::compact_contract::diagnostic_channel::const_str_is_snake_of(
                    $leaf, stringify!($variant),
                ),
                "a cause's leaf does not spell the cause it names",
            );)+
        };
    };

    (
        $enum:ident, all = $all:ident, count = $count:ident, slot = $slot:ident,
        unit_name = $name_fn:ident, prefix = $prefix:literal,
        rename = ($from:literal, $to:literal);
        $( $variant:ident = $index:literal => $leaf:literal $($renamed:ident)?, )+
    ) => {
        $crate::cause_column::indexed_causes_column!(
            $enum, $all, $count, $slot; $($variant = $index,)+
        );
        $crate::cause_column::indexed_causes_names!(
            $enum, $name_fn, $prefix; $($variant => $leaf,)+
        );

        const _: () = {
            $($crate::cause_column::leaf_spells_its_cause!(
                $($renamed)? ($from, $to), stringify!($variant), $leaf
            );)+
        };
    };

    (
        $enum:ident, all = $all:ident, count = $count:ident, slot = $slot:ident,
        unit_name = $name_fn:ident, prefix = $prefix:literal, leaves = $leaves:ident,
        rename = ($from:literal, $to:literal);
        $( $variant:ident = $index:literal => $leaf:literal $($renamed:ident)?, )+
    ) => {
        $crate::cause_column::indexed_causes!(
            $enum, all = $all, count = $count, slot = $slot,
            unit_name = $name_fn, prefix = $prefix, rename = ($from, $to);
            $($variant = $index => $leaf $($renamed)?,)+
        );

        /// The bare leaves, indexed by the slot, for a cross-tab that heads a
        /// column with one.
        pub const $leaves: [&str; $enum::$count] = {
            let mut leaves = [""; $enum::$count];
            $(leaves[$index] = $leaf;)+
            leaves
        };
    };
}

/// `ALL`, the count and the slot, with the guards that make them one table.
macro_rules! indexed_causes_column {
    (
        $enum:ident, $all:ident, $count:ident, $slot:ident;
        $( $variant:ident = $index:literal, )+
    ) => {
        impl $enum {
            pub const $count: usize = [$($index,)+].len();

            pub const $all: [Self; Self::$count] = [$(Self::$variant,)+];

            pub const fn $slot(self) -> usize {
                match self {
                    $(Self::$variant => $index,)+
                }
            }
        }

        const _: () = {
            assert!(
                ::compact_contract::diagnostic_channel::indices_are_a_bijection(
                    &[$($index,)+],
                    $enum::$count,
                ),
                "two causes share a counter slot, or one indexes past the array",
            );
            $(assert!(
                $enum::$variant as usize == $index,
                "a cause's declared slot is not its own discriminant",
            );)+
            let mut i = 0;
            while i < $enum::$count {
                assert!(
                    $enum::$all[i].$slot() == i,
                    "a cause is not at the slot its counter is read from",
                );
                i += 1;
            }
        };
    };
}

/// The published name, which is the shared prefix and the row's own leaf.
macro_rules! indexed_causes_names {
    (
        $enum:ident, $name_fn:ident, $prefix:literal;
        $( $variant:ident => $leaf:literal, )+
    ) => {
        impl $enum {
            pub const fn $name_fn(self) -> &'static str {
                match self {
                    $(Self::$variant => concat!($prefix, "_", $leaf),)+
                }
            }
        }

        const _: () = {
            assert!(
                ::compact_contract::diagnostic_channel::const_names_are_distinct(
                    &[$(concat!($prefix, "_", $leaf),)+],
                ),
                "two causes are published under the same unit name",
            );
        };
    };
}

/// Distinctness inside a taxonomy is not distinctness on the wire. Two of the
/// prefixes here nest -- `domain53_operative_skip_rostered` is
/// `domain53_operative_skip` and a leaf -- so a leaf added to the shorter one
/// can spell a whole name of the longer, and the two counters accumulate into
/// one published unit. A cross-tab cell is a name of the same kind, so the
/// bracketed list takes any const array of published names as well. Every
/// source of a published name in this crate is listed here once; one left out
/// is not cross-checked.
macro_rules! columns_publish_distinct_names {
    (
        $( $enum:ident, all = $all:ident, count = $count:ident,
           unit_name = $name_fn:ident; )+
        [ $( $extra:ident, )* ]
    ) => {
        const _: () = {
            const TOTAL: usize = 0 $( + $enum::$count )+ $( + $extra.len() )*;
            let mut names = [""; TOTAL];
            let mut n = 0;
            $(
                let mut i = 0;
                while i < $enum::$count {
                    names[n] = $enum::$all[i].$name_fn();
                    n += 1;
                    i += 1;
                }
            )+
            $(
                let mut i = 0;
                while i < $extra.len() {
                    names[n] = $extra[i];
                    n += 1;
                    i += 1;
                }
            )*
            assert!(
                ::compact_contract::diagnostic_channel::const_names_are_distinct(&names),
                "two published unit names in this crate are the same string",
            );
        };
    };
}

/// The half a bijection cannot see: distinct slots say every cause has its own
/// counter, and nothing about the counter carrying that cause's name.
///
/// An ordinary row's leaf spells its variant. A row marked `renamed` is held
/// to its own name under the taxonomy's ONE substitution, so two renamed rows
/// cannot exchange leaves either. The substitution must also actually rename:
/// one that leaves the leaf spelling its variant is refused, which is what
/// stops the marker becoming a place to park an ordinary cause past the check.
macro_rules! leaf_spells_its_cause {
    ( ($from:literal, $to:literal), $variant:expr, $leaf:expr ) => {
        assert!(
            ::compact_contract::diagnostic_channel::const_str_is_snake_of($leaf, $variant),
            "a cause's leaf does not spell the cause it names",
        );
    };
    ( renamed ($from:literal, $to:literal), $variant:expr, $leaf:expr ) => {
        assert!(
            $crate::cause_column::renamed_leaf_spells_its_cause($leaf, $variant, $from, $to),
            "a renamed cause's leaf is not its own name under the taxonomy's substitution",
        );
        assert!(
            !::compact_contract::diagnostic_channel::const_str_is_snake_of($leaf, $variant),
            "a substitution that leaves the leaf spelling its own variant renames nothing",
        );
    };
}

/// `leaf` is `variant` with the leading `from` replaced by `to`, the rest
/// spelled in snake case.
///
/// A substitution that consumes the WHOLE variant is refused. It would leave
/// nothing for the compiler to derive -- the leaf would be pinned only by the
/// replacement written beside it, which is the second table this column exists
/// to remove, and two causes could then exchange leaves by each declaring the
/// other's (Codex round 2). A substitution renames a PREFIX, and the rest of
/// the name is the variant's own.
pub(crate) const fn renamed_leaf_spells_its_cause(
    leaf: &str,
    variant: &str,
    from: &str,
    to: &str,
) -> bool {
    let (leaf, variant) = (leaf.as_bytes(), variant.as_bytes());
    let (from, to) = (from.as_bytes(), to.as_bytes());
    if from.is_empty() || from.len() >= variant.len() || to.len() >= leaf.len() {
        return false;
    }
    let mut i = 0;
    while i < from.len() {
        if variant[i] != from[i] {
            return false;
        }
        i += 1;
    }
    let mut i = 0;
    while i < to.len() {
        if leaf[i] != to[i] {
            return false;
        }
        i += 1;
    }
    let mut v = from.len();
    let mut l = to.len();
    if leaf[l] != b'_' {
        return false;
    }
    l += 1;
    let mut first = true;
    while v < variant.len() {
        let byte = variant[v];
        if byte.is_ascii_uppercase() {
            if !first {
                if l >= leaf.len() || leaf[l] != b'_' {
                    return false;
                }
                l += 1;
            }
            if l >= leaf.len() || leaf[l] != byte.to_ascii_lowercase() {
                return false;
            }
        } else if l >= leaf.len() || leaf[l] != byte {
            return false;
        }
        first = false;
        v += 1;
        l += 1;
    }
    l == leaf.len()
}

/// Space, tab, carriage return or newline: what may sit between a site and the
/// name it gives.
const fn is_layout_byte(byte: u8) -> bool {
    byte == b' ' || byte == b'\t' || byte == b'\r' || byte == b'\n'
}

/// A byte an identifier may continue with, which is what tells a whole name
/// from a prefix of a longer one.
const fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Past layout AND comments, because a comment between a variant and its
/// argument list is invisible to the compiler and was invisible to the first
/// form of this guard (Codex round 14).
const fn past_layout(src: &[u8], mut at: usize) -> usize {
    loop {
        while at < src.len() && is_layout_byte(src[at]) {
            at += 1;
        }
        if at + 1 < src.len() && src[at] == b'/' && src[at + 1] == b'*' {
            at += 2;
            while at + 1 < src.len() && !(src[at] == b'*' && src[at + 1] == b'/') {
                at += 1;
            }
            at = if at + 1 < src.len() { at + 2 } else { src.len() };
        } else if at + 1 < src.len() && src[at] == b'/' && src[at + 1] == b'/' {
            while at < src.len() && src[at] != b'\n' {
                at += 1;
            }
        } else {
            return at;
        }
    }
}

/// How many times `needle` appears in `source` before `stop_at`.
///
/// The count is the guard a lexical allowlist cannot be: an added occurrence of
/// the variant's own name, in ANY spelling or spacing, changes it. What the
/// allowlist then adds is that the sanctioned occurrences are the right ones.
/// Comments and string literals do not count: prose naming the variant is not
/// a site, and neither is an assert message that quotes it. This crate has no
/// raw or byte string literals, so a plain `"` with backslash escapes is the
/// whole of the string syntax it has to walk.
pub(crate) const fn const_str_count(source: &str, stop_at: usize, needle: &str) -> usize {
    let (src, needle) = (source.as_bytes(), needle.as_bytes());
    if needle.is_empty() || needle.len() > src.len() {
        return 0;
    }
    let mut last = src.len() - needle.len();
    if stop_at < last {
        last = stop_at;
    }
    let (mut at, mut seen) = (0, 0);
    while at <= last {
        if src[at] == b'/' && at + 1 < src.len() && (src[at + 1] == b'*' || src[at + 1] == b'/') {
            at = past_layout(src, at);
            continue;
        }
        if src[at] == b'"' {
            at += 1;
            while at < src.len() && src[at] != b'"' {
                at += if src[at] == b'\\' { 2 } else { 1 };
            }
            at += 1;
            continue;
        }
        if src[at] == needle[0] {
            let mut i = 1;
            while i < needle.len() && src[at + i] == needle[i] {
                i += 1;
            }
            if i == needle.len() {
                seen += 1;
            }
        }
        at += 1;
    }
    seen
}

/// Does every `site` in `source` name one of `allowed` next?
///
/// For a guard that has to read source text, which is the only way to say "no
/// site here writes this name by hand" when the payload's own type cannot say
/// it. An ALLOWLIST, not a search for one bad spelling: the weaker form looked
/// for a literal and a `stringify!` walked past it (Codex round 11). Whatever
/// follows the site, after layout, must begin with a declaration's name, so a
/// literal, a `stringify!`, a `concat!` or anything else is refused by not
/// being on the list.
///
/// The first byte is compared before the inner walk is entered at all: over a
/// source file of this size the naive form outruns const eval's step budget.
/// Where `marker` first appears, or the end of `source`. One pass, so a caller
/// that needs the bound for several scans pays for it once.
pub(crate) const fn const_str_find(source: &str, marker: &str) -> usize {
    let (src, marker) = (source.as_bytes(), marker.as_bytes());
    if marker.is_empty() || marker.len() > src.len() {
        return src.len();
    }
    let mut at = 0;
    let last = src.len() - marker.len();
    while at <= last {
        if src[at] == marker[0] {
            let mut i = 1;
            while i < marker.len() && src[at + i] == marker[i] {
                i += 1;
            }
            if i == marker.len() {
                return at;
            }
        }
        at += 1;
    }
    src.len()
}

/// `stop_at` bounds the scan to production: a test may build an error from a
/// string it scanned or generated, and what it hands to a constructor is not a
/// name production publishes.
pub(crate) const fn every_site_names_a_declaration(
    source: &str,
    stop_at: usize,
    site: &str,
    open: u8,
    allowed: &[&str],
) -> bool {
    let (src, site) = (source.as_bytes(), site.as_bytes());
    if site.is_empty() || site.len() > src.len() {
        return false;
    }
    let mut last = src.len() - site.len();
    if stop_at < last {
        last = stop_at;
    }
    let mut start = 0;
    while start <= last {
        // Comments and string literals are not sites, for the same reason they
        // are not counted: prose that names the variant is not a raise.
        if src[start] == b'/'
            && start + 1 < src.len()
            && (src[start + 1] == b'*' || src[start + 1] == b'/')
        {
            start = past_layout(src, start);
            continue;
        }
        if src[start] == b'"' {
            start += 1;
            while start < src.len() && src[start] != b'"' {
                start += if src[start] == b'\\' { 2 } else { 1 };
            }
            start += 1;
            continue;
        }
        if src[start] == site[0] {
            let mut i = 1;
            while i < site.len() && src[start + i] == site[i] {
                i += 1;
            }
            if i == site.len() {
                // Not part of a longer identifier: `InvalidRootish(` is not a
                // site, and the argument list may sit behind layout or a
                // comment (Codex round 14).
                if is_name_byte(src[start + site.len()]) {
                    start += 1;
                    continue;
                }
                let mut at = past_layout(src, start + site.len());
                if at >= src.len() || src[at] != open {
                    start += 1;
                    continue;
                }
                at = past_layout(src, at + 1);
                let mut named = false;
                let mut a = 0;
                while a < allowed.len() {
                    let want = allowed[a].as_bytes();
                    if at + want.len() <= src.len() {
                        let mut j = 0;
                        while j < want.len() && src[at + j] == want[j] {
                            j += 1;
                        }
                        // The whole expression, not a prefix of one: an
                        // allowlist that matched prefixes accepted any
                        // identifier that merely started like a sanctioned one
                        // (Codex round 12).
                        if j == want.len()
                            && (at + want.len() == src.len()
                                || !is_name_byte(src[at + want.len()]))
                        {
                            named = true;
                        }
                    }
                    a += 1;
                }
                if !named {
                    return false;
                }
            }
        }
        start += 1;
    }
    true
}

pub(crate) use columns_publish_distinct_names;
pub(crate) use indexed_causes;
pub(crate) use indexed_causes_column;
pub(crate) use indexed_causes_names;
pub(crate) use leaf_spells_its_cause;
