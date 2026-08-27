//! One declaration per cause, for the d81 taxonomies whose wire ordinal and
//! wire name were two hand-written matches.
//!
//! THE DEFECT. `d81_plan_refusal_ordinal` and `d81_plan_refusal_unit_name` were
//! separate matches over separate tables, so a cause given an ordinal another
//! cause already resolved published under that cause's name; the gap pair had
//! the same hole, and the failure stage and reason names could carry a number
//! that was not the enum's own discriminant. All four are the shape lane TV
//! closed for d90's subsites (Codex round 2, findings 4 and 5).
//!
//! The counter names spell their ordinal, which is what makes a manifest
//! readable, and is a second place for the number to be wrong. Both macros
//! refuse a spelling whose digits are not the ordinal, and refuse two causes
//! spelled alike -- distinct ordinals are not distinct names, and the manifest
//! aggregates by name.

/// `name` is `ordinal` in two digits, an underscore, then `variant` in snake
/// case.
pub(super) const fn is_ordinal_then_snake_of(name: &str, ordinal: u32, variant: &str) -> bool {
    compact_contract::diagnostic_channel::const_str_has_ordinal_prefix(name, ordinal)
        && is_snake_from(name, 3, variant)
}

/// `name` is `variant` in snake case.
pub(super) const fn is_snake_spelling_of(name: &str, variant: &str) -> bool {
    is_snake_from(name, 0, variant)
}

const fn is_snake_from(name: &str, offset: usize, variant: &str) -> bool {
    let (name, variant) = (name.as_bytes(), variant.as_bytes());
    let mut i = 0;
    let mut j = offset;
    while i < variant.len() {
        let byte = variant[i];
        if byte.is_ascii_uppercase() {
            if i > 0 {
                if j >= name.len() || name[j] != b'_' {
                    return false;
                }
                j += 1;
            }
            if j >= name.len() || name[j] != byte.to_ascii_lowercase() {
                return false;
            }
        } else {
            if j >= name.len() || name[j] != byte {
                return false;
            }
        }
        i += 1;
        j += 1;
    }
    j == name.len()
}

/// A fieldless enum whose discriminant is its wire code, with one counter name
/// per variant.
///
/// The enum keeps its own declaration -- the variants carry long reasons for
/// their existence -- and this pins the names to it: the match is exhaustive so
/// a new variant must appear here, the declared code must BE the variant's
/// discriminant, and the name must spell that code and that variant.
macro_rules! d81_coded_unit_names {
    (
        $enum:ident => fn $name_fn:ident, prefix = $prefix:literal;
        $( $variant:ident = $code:literal => $leaf:literal, )+
    ) => {
        fn $name_fn(value: $enum) -> &'static str {
            match value {
                $($enum::$variant => concat!($prefix, $leaf),)+
            }
        }

        const _: () = {
            $(assert!(
                $enum::$variant as u32 == $code,
                "a d81 counter name is declared against a code that is not the variant's",
            );)+
            $(assert!(
                crate::attach_log::ordinal_taxonomy::is_ordinal_then_snake_of(
                    $leaf,
                    $code,
                    stringify!($variant),
                ),
                "a d81 counter name does not spell its own code and cause",
            );)+
            assert!(
                compact_contract::diagnostic_channel::const_names_are_distinct(&[$($leaf,)+]),
                "two d81 causes carry the same counter name",
            );
        };
    };
}

/// A fieldless enum whose discriminant is its BIN INDEX, published as one
/// named unit per bin.
///
/// Same rule as [`d81_coded_unit_names`], plus the array the publisher indexes
/// so no site writes a bin's name beside a bin's number. The codes are a
/// bijection over `0..COUNT`, which is what makes `COUNT` the enum's own size
/// rather than a number written beside it (Codex round 3, finding 4).
macro_rules! d81_indexed_unit_names {
    (
        $enum:path as $alias:ident => $names:ident, fn $proof:ident, count = $count:expr,
        prefix = $prefix:literal;
        $( $variant:ident = $code:literal => $leaf:literal, )+
    ) => {
        pub(super) const $names: [&str; $count] = {
            let mut names = [""; $count];
            $(names[$code] = concat!($prefix, $leaf);)+
            names
        };

        /// Exhaustive, so a variant that is not declared above has no bin and
        /// does not compile.
        #[allow(dead_code)]
        pub(super) fn $proof(value: $enum) -> &'static str {
            match value {
                $($alias::$variant => $names[$code],)+
            }
        }

        const _: () = {
            assert!(
                compact_contract::diagnostic_channel::indices_are_a_bijection(
                    &[$($code,)+],
                    $count,
                ),
                "two d81 bins share an index, or one indexes past the histogram",
            );
            $(assert!(
                $alias::$variant as usize == $code,
                "a d81 bin is declared against an index that is not the variant's",
            );)+
            $(assert!(
                crate::attach_log::ordinal_taxonomy::is_snake_spelling_of(
                    $leaf,
                    stringify!($variant),
                ),
                "a d81 bin's name does not spell the variant it counts",
            );)+
            assert!(
                compact_contract::diagnostic_channel::const_names_are_distinct(&[$($leaf,)+]),
                "two d81 bins carry the same name",
            );
        };
    };
}
/// A histogram over an enum this crate does not own, where the bins are
/// ordinals from 1 and `0` is the unknown arm.
///
/// Three groups. `causes` is the ordinary one: the bin's name spells its
/// ordinal and then its variant, and the pattern is that variant ignoring its
/// payload. `renamed` is a variant whose name deliberately does not spell it,
/// with the reason; a `renamed` bin whose name DOES spell its variant is
/// refused, so the group cannot become a way past the check. `nested` is for a
/// bin that is not one variant at all -- a case inside a payload -- where
/// there is no variant name to hold the spelling to, and the whole pattern is
/// written out (Codex round 3, finding 3).
macro_rules! d81_plan_ordinal_bins {
    (
        $subject:path as $alias:ident => fn $ordinal_fn:ident, fn $name_fn:ident,
        prefix = $prefix:literal, unknown = $unknown:literal;
        causes { $( $variant:ident = $ordinal:literal => $leaf:literal, )* }
        renamed { $( $(#[$rd:meta])* $renamed:ident = $renamed_ordinal:literal => $renamed_leaf:literal, )* }
        nested { $( $(#[$nd:meta])* $pattern:pat = $nested_ordinal:literal => $nested_leaf:literal, )* }
    ) => {
        pub fn $ordinal_fn(value: &$subject) -> u32 {
            match value {
                $($alias::$variant { .. } => $ordinal,)*
                $($alias::$renamed { .. } => $renamed_ordinal,)*
                $($pattern => $nested_ordinal,)*
            }
        }

        pub fn $name_fn(ordinal: u32) -> &'static str {
            match ordinal {
                $($ordinal => concat!($prefix, $leaf),)*
                $($renamed_ordinal => concat!($prefix, $renamed_leaf),)*
                $($nested_ordinal => concat!($prefix, $nested_leaf),)*
                _ => concat!($prefix, $unknown),
            }
        }

        const _: () = {
            assert!(
                compact_contract::diagnostic_channel::indices_are_a_bijection(
                    &[$($ordinal - 1,)* $($renamed_ordinal - 1,)* $($nested_ordinal - 1,)*],
                    [$($ordinal,)* $($renamed_ordinal,)* $($nested_ordinal,)*].len(),
                ),
                "two d81 plan bins share an ordinal, or one is outside 1..=count",
            );
            $(assert!(
                crate::attach_log::ordinal_taxonomy::is_ordinal_then_snake_of(
                    $leaf,
                    $ordinal,
                    stringify!($variant),
                ),
                "a d81 plan bin's name does not spell its own ordinal and cause; \
                 if the spelling is deliberate the bin belongs in renamed",
            );)*
            $(assert!(
                !crate::attach_log::ordinal_taxonomy::is_ordinal_then_snake_of(
                    $renamed_leaf,
                    $renamed_ordinal,
                    stringify!($renamed),
                ) && compact_contract::diagnostic_channel::const_str_has_ordinal_prefix(
                    $renamed_leaf,
                    $renamed_ordinal,
                ),
                "a renamed d81 plan bin either spells its own variant, in which \
                 case it belongs in causes, or does not spell its ordinal",
            );)*
            $(assert!(
                compact_contract::diagnostic_channel::const_str_has_ordinal_prefix(
                    $nested_leaf,
                    $nested_ordinal,
                ),
                "a nested d81 plan bin's name does not spell its own ordinal",
            );)*
            assert!(
                compact_contract::diagnostic_channel::const_names_are_distinct(
                    &[$($leaf,)* $($renamed_leaf,)* $($nested_leaf,)* $unknown,]
                ),
                "two d81 plan bins carry the same name",
            );
        };
    };
}
/// A histogram whose bins are addressed by NUMBER at the sites that fill
/// them, published under one name per bin.
///
/// The number and the name were two tables and the sites wrote raw integers,
/// so moving a cause published its losses under its neighbour's name and
/// nothing saw it (Codex round 4, finding 1). One declaration now: each row
/// mints a named constant in a module of its own, the wire name is that same
/// identifier under the table's prefix, and a site says which cause it means
/// instead of which slot.
macro_rules! numbered_cause_bins {
    (
        mod $module:ident, names = $names:ident, count = $count:ident,
        prefix = $prefix:literal, base = $base:literal;
        $( $(#[$doc:meta])* $cause:ident = $index:literal, )+
    ) => {
        #[allow(non_upper_case_globals)]
        mod $module {
            $($(#[$doc])* pub const $cause: usize = $index;)+
        }

        const $count: usize = [$($index,)+].len();

        const $names: [&str; $count] = {
            let mut names = [""; $count];
            $(names[$index - $base] = concat!($prefix, stringify!($cause));)+
            names
        };

        const _: () = {
            assert!(
                compact_contract::diagnostic_channel::indices_are_a_bijection(
                    &[$($index - $base,)+],
                    $count,
                ),
                "two causes share a bin, or one indexes past the histogram",
            );
            assert!(
                compact_contract::diagnostic_channel::const_names_are_distinct(&$names),
                "two causes are published under the same name",
            );
        };
    };
}
/// [`numbered_cause_bins`] for a table whose wire names carry their own
/// ordinal, which is a second place for the number to be wrong.
macro_rules! ordinal_named_cause_bins {
    (
        mod $module:ident, names = $names:ident, count = $count:ident,
        prefix = $prefix:literal;
        $( $(#[$doc:meta])* $cause:ident = $index:literal => $leaf:literal, )+
    ) => {
        #[allow(non_upper_case_globals)]
        pub(crate) mod $module {
            $($(#[$doc])* pub const $cause: u32 = $index;)+
        }

        pub const $count: usize = [$($index,)+].len();

        pub const $names: [&str; $count] = {
            let mut names = [""; $count];
            $(names[$index as usize] = concat!($prefix, $leaf);)+
            names
        };

        const _: () = {
            assert!(
                compact_contract::diagnostic_channel::indices_are_a_bijection(
                    &[$($index as usize,)+],
                    $count,
                ),
                "two causes share a bin, or one indexes past the histogram",
            );
            $(assert!(
                compact_contract::diagnostic_channel::const_str_is_ordinal_then(
                    $leaf,
                    $index,
                    stringify!($cause),
                ),
                "a bin's name does not spell its own ordinal and cause",
            );)+
            assert!(
                compact_contract::diagnostic_channel::const_names_are_distinct(&$names),
                "two causes are published under the same name",
            );
        };
    };
}
pub(super) use d81_coded_unit_names;
pub(super) use d81_indexed_unit_names;
pub(super) use numbered_cause_bins;
pub(super) use ordinal_named_cause_bins;
pub(super) use d81_plan_ordinal_bins;

#[cfg(test)]
mod tests {
    use super::{is_ordinal_then_snake_of, is_snake_spelling_of};

    /// The two spelling guards these macros const-assert with.
    ///
    /// Unlike the naval route taxonomy, nothing here asserts the FALSE arm at
    /// compile time -- the `renamed` and `nested` groups are held to their
    /// ordinal only -- so a rubber stamp would be invisible and every bin
    /// could be named after another cause (Codex round 4, finding 2).
    ///
    /// MUTATION: either returns `true` unconditionally. The tree still
    /// compiles and this fails.
    #[test]
    fn the_spelling_guards_refuse_a_bin_named_after_another_cause() {
        assert!(is_ordinal_then_snake_of("01_depth_exceeded", 1, "DepthExceeded"));
        assert!(!is_ordinal_then_snake_of("01_byte_limit", 1, "DepthExceeded"));
        assert!(!is_ordinal_then_snake_of("02_depth_exceeded", 1, "DepthExceeded"));
        assert!(!is_ordinal_then_snake_of(
            "01_depth_exceeded_twice",
            1,
            "DepthExceeded"
        ));
        assert!(!is_ordinal_then_snake_of("01_depth", 1, "DepthExceeded"));

        assert!(is_snake_spelling_of("interval_count", "IntervalCount"));
        assert!(is_snake_spelling_of("vector", "Vector"));
        assert!(!is_snake_spelling_of("interval_count", "IntervalClosing"));
        assert!(!is_snake_spelling_of("intervalcount", "IntervalCount"));
    }
}
