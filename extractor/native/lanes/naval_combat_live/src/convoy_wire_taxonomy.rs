//! One declaration per wire cause, for the d90 taxonomies that used to be
//! several hand-maintained tables.
//!
//! THE DEFECT THESE MACROS CLOSE. A `CaptureError` variant given a subsite
//! index another variant already held compiled, passed, and counted its losses
//! under the other cause's name: `subsite_index`'s exhaustive match forced a
//! new variant to get AN index but not a free one, and the test that would have
//! caught the collision listed the variants by hand, so a variant left out of
//! that list was never handed to it (lane TS, `runs/ts_d90_combat_sink/FIX.md`
//! section 8). `RouteSourceError::refusal_index` had the same shape with no
//! test at all.
//!
//! WHAT REPLACES IT. The declaration carries a cause's index, its slot and its
//! leaf spelling once, and the uses are generated from it:
//!
//! * the index a variant reports is the declared one, via a match with no
//!   wildcard arm -- so a variant MISSING from the declaration makes the match
//!   non-exhaustive, and a declared name that is not a variant does not
//!   resolve. The enum stays hand-written and cannot drift from the table;
//! * two variants sharing an index, or an index past the wire, is a const
//!   assert (`indices_are_a_bijection`) rather than a run-time surprise;
//! * the wire leaf is `concat!` of the table's prefix and the slot's own
//!   identifier, so no spelling is written twice;
//! * the slot's identifier must be the variant's name in snake case, so a
//!   variant cannot be counted under a leaf that names something else;
//! * the leaf must already be declared to the diagnostic channel with the
//!   accumulation the table needs, which is what makes a new cause land in the
//!   manifest as the kind of number it is.
//!
//! The wire index is WRITTEN, never taken from declaration position, for the
//! reason `convoy_combat_refusal_causes!` writes its own: reordering the list
//! must not renumber the causes recorded captures carry.

/// `field` is `variant` spelled in snake case.
///
/// BOTH ARMS are exercised at compile time and neither has a test: the
/// `errors` group asserts this holds for every ordinary cause and the
/// `renamed_errors` group asserts it does NOT for every deliberately renamed
/// one, so a rubber-stamp `true` and a rubber-stamp `false` each fail to build
/// (Codex round 2, finding 7).
pub(crate) const fn is_snake_case_spelling_of(variant: &str, field: &str) -> bool {
    let (variant, field) = (variant.as_bytes(), field.as_bytes());
    let mut i = 0;
    let mut j = 0;
    while i < variant.len() {
        let byte = variant[i];
        if byte.is_ascii_uppercase() {
            if i > 0 {
                if j >= field.len() || field[j] != b'_' {
                    return false;
                }
                j += 1;
            }
            if j >= field.len() || field[j] != byte.to_ascii_lowercase() {
                return false;
            }
        } else {
            if j >= field.len() || field[j] != byte {
                return false;
            }
        }
        i += 1;
        j += 1;
    }
    j == field.len()
}

/// A block of per-cause u64 counters that travels as a contiguous window of
/// wire words, one word per declared slot.
///
/// The counters are LIFETIME totals, so every leaf must be declared cumulative
/// to the diagnostic channel; a table of some other accumulation would need its
/// own macro rather than a parameter, because the choice is a property of what
/// the numbers mean.
macro_rules! convoy_subsite_table {
    (
        $(#[$table_doc:meta])*
        $table:ident, unit_prefix = $prefix:literal, count = $count_vis:vis $count:ident;
        $( $(#[$slot_doc:meta])* $field:ident = $index:literal, )+
    ) => {
        $count_vis const $count: usize = [$($index,)+].len();

        $(#[$table_doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct $table {
            $($(#[$slot_doc])* pub $field: u64,)+
        }

        impl $table {
            /// Wire order: slot `i` is the leaf whose declared index is `i`.
            pub const FIELDS: [&'static str; $count] = {
                let mut names = [""; $count];
                $(names[$index] = concat!($prefix, stringify!($field));)+
                names
            };

            pub fn total(&self) -> u64 {
                self.as_array()
                    .into_iter()
                    .fold(0u64, |total, value| total.wrapping_add(value))
            }

            pub fn as_array(&self) -> [u64; $count] {
                let mut values = [0u64; $count];
                $(values[$index] = self.$field;)+
                values
            }

            pub fn from_array(values: [u64; $count]) -> Self {
                Self { $($field: values[$index],)+ }
            }

            /// Each slot's wire index, for a hot-path counter array that has
            /// to be addressed rather than named. Spelled like the field
            /// because it IS the field's slot, and a second spelling would be
            /// a second thing to keep in step.
            $(
                #[allow(non_upper_case_globals)]
                pub const $field: usize = $index;
            )+
        }

        /// A subsite table's DECLARATION ORDER IS ITS WIRE ORDER, which is
        /// stronger than the bijection the route causes get and is what lets
        /// the hot-path counters be one indexed array. Indices are still
        /// written, so the wire identity is on the page; writing one that is
        /// not the row's own position moves a v10 byte's meaning and is
        /// refused here rather than renumbered silently.
        const _: () = {
            let declared = [$($index,)+];
            assert!(
                ::compact_contract::diagnostic_channel::indices_are_a_bijection(
                    &declared,
                    $count,
                ),
                "two slots share a wire index, or a slot indexes past the window",
            );
            let mut position = 0;
            while position < $count {
                assert!(
                    declared[position] == position,
                    "a subsite slot's declared index is not its position, so the \
                     table and the wire disagree about which word is which",
                );
                position += 1;
            }
        };

        const _: () = {
            let mut i = 0;
            while i < $count {
                assert!(
                    ::compact_contract::diagnostic_channel::declares_cumulative_unit(
                        $table::FIELDS[i],
                    ),
                    "a subsite leaf is not declared cumulative to the diagnostic channel",
                );
                i += 1;
            }
        };
    };
}

/// The `CaptureError` window: a subsite table whose slots are enum variants.
///
/// One row per variant. The enum itself stays where its payload types live and
/// is written by hand; this is what pins it to the wire.
macro_rules! convoy_capture_error_subsites {
    (
        $(#[$table_doc:meta])*
        $error:path => $table:ident, unit_prefix = $prefix:literal, count = $count_vis:vis $count:ident;
        $( $variant:ident = $index:literal => $field:ident, )+
    ) => {
        crate::convoy_wire_taxonomy::convoy_subsite_table! {
            $(#[$table_doc])*
            $table, unit_prefix = $prefix, count = $count_vis $count;
            $( $field = $index, )+
        }

        impl $error {
            /// Index of this variant's counter in
            /// [`ConvoyTransportAttackCaptureErrorSubsites`].
            pub fn subsite_index(&self) -> usize {
                match self {
                    $(Self::$variant { .. } => $index,)+
                }
            }
        }

        const _: () = {
            $(assert!(
                crate::convoy_wire_taxonomy::is_snake_case_spelling_of(
                    stringify!($variant),
                    stringify!($field),
                ),
                "a CaptureError variant's wire leaf names something else",
            );)+
        };
    };
}

/// The per-frame route-refusal leaves, whose causes are a `RouteSourceError`
/// plus the ones no `RouteSourceError` produces.
///
/// Not a [`convoy_subsite_table`]: these are window-fold leaves the sweep
/// republishes every frame rather than lifetime counters, they travel as a name
/// array and per-cause bins rather than a struct, and three of the causes are
/// decided outside the error enum. What is the same is the part that desynced.
///
/// The snake-case coupling applies here too, and the causes whose leaf
/// deliberately does not spell their variant go in `renamed_errors` with the
/// reason. Today that is one: `UnsupportedProducerVtable` publishes as
/// `unsupported_producer` because `_vtable` is a `RAW_WORD_SUFFIXES` entry and
/// that spelling would be folded as a last-value raw word instead of summed as
/// the member count it is -- which the per-frame assert below refuses.
macro_rules! convoy_route_refusal_causes {
    (
        $(#[$table_doc:meta])*
        $error:path => $names:ident, unit_prefix = $prefix:literal,
        row_units = $row_names:ident, row_unit_prefix = $row_prefix:literal,
        count = $count:ident;
        errors { $( $variant:ident = $error_index:literal => $error_field:ident, )+ }
        renamed_errors { $( $(#[$renamed_doc:meta])* $renamed:ident = $renamed_index:literal => $renamed_field:ident, )+ }
        unattributed { $( $(#[$slot_doc:meta])* $slot_const:ident = $slot_index:literal => $slot_field:ident, )+ }
    ) => {
        pub const $count: usize =
            [$($error_index,)+ $($renamed_index,)+ $($slot_index,)+].len();

        $(#[$table_doc])*
        pub const $names: [&str; $count] = {
            let mut names = [""; $count];
            $(names[$error_index] = concat!($prefix, stringify!($error_field));)+
            $(names[$renamed_index] = concat!($prefix, stringify!($renamed_field));)+
            $(names[$slot_index] = concat!($prefix, stringify!($slot_field));)+
            names
        };

        /// The PER-ROW leaves the fold publishes for the same causes.
        ///
        /// Derived here rather than sliced out of the sweep's leaf at the fold
        /// site, and const-asserted declared, because a cause added to this
        /// declaration otherwise reaches the fold as an undeclared unit and
        /// fails promotion on a capture instead of failing the build (Codex
        /// round 6, finding 1).
        pub const $row_names: [&str; $count] = {
            let mut names = [""; $count];
            $(names[$error_index] = concat!($row_prefix, stringify!($error_field));)+
            $(names[$renamed_index] = concat!($row_prefix, stringify!($renamed_field));)+
            $(names[$slot_index] = concat!($row_prefix, stringify!($slot_field));)+
            names
        };

        $($(#[$slot_doc])* pub const $slot_const: usize = $slot_index;)+

        /// A slot constant names the cause it indexes. Nothing else ties the
        /// two: the index bijection, the distinct names and the v4 pin all
        /// survive exchanging two constants over their rows, and each cause
        /// then publishes under the other (Codex round 9).
        const _: () = {
            $(assert!(
                ::compact_contract::diagnostic_channel::const_str_eq_ignore_ascii_case(
                    stringify!($slot_const),
                    concat!("convoy_route_refusal_", stringify!($slot_field), "_index"),
                ),
                "an unattributed route slot constant names another cause",
            );)+
        };

        impl $error {
            /// Index of this variant's counter in [`CONVOY_ROUTE_REFUSAL_UNIT_NAMES`].
            pub fn refusal_index(&self) -> usize {
                match self {
                    $(Self::$variant { .. } => $error_index,)+
                    $(Self::$renamed { .. } => $renamed_index,)+
                }
            }
        }

        const _: () = assert!(
            ::compact_contract::diagnostic_channel::indices_are_a_bijection(
                &[$($error_index,)+ $($renamed_index,)+ $($slot_index,)+],
                $count,
            ),
            "two route causes share a wire index, or a cause indexes past the array",
        );

        /// Distinct INDICES are not distinct NAMES, and the manifest
        /// aggregates by name: two causes spelled alike are summed together
        /// and neither can be read (Codex round 2, finding 1).
        const _: () = assert!(
            ::compact_contract::diagnostic_channel::const_names_are_distinct(&$names),
            "two route causes carry the same leaf spelling",
        );

        const _: () = {
            $(assert!(
                crate::convoy_wire_taxonomy::is_snake_case_spelling_of(
                    stringify!($variant),
                    stringify!($error_field),
                ),
                "a route cause's leaf names another cause; if the spelling is \
                 deliberate the cause belongs in renamed_errors with its reason",
            );)+
            $(assert!(
                !crate::convoy_wire_taxonomy::is_snake_case_spelling_of(
                    stringify!($renamed),
                    stringify!($renamed_field),
                ),
                "a renamed_errors cause spells its own variant, so it is not \
                 renamed and belongs in errors",
            );)+
        };

        const _: () = {
            let mut i = 0;
            while i < $count {
                assert!(
                    ::compact_contract::diagnostic_channel::declares_per_frame_unit($names[i])
                        && ::compact_contract::diagnostic_channel::declares_per_frame_unit(
                            $row_names[i],
                        ),
                    "a route refusal leaf, or its per-row twin, is declared as \
                     something other than a per-frame count",
                );
                i += 1;
            }
            assert!(
                ::compact_contract::diagnostic_channel::const_names_are_distinct(&$row_names),
                "two route causes carry the same per-row leaf spelling",
            );
        };
    };
}

/// A telemetry counter's published name, derived from the field it counts.
///
/// The lane held the names in one literal and the host held two more, so a
/// counter added to one and not the others was published under a neighbour's
/// name or not at all -- and the two lists had ALREADY drifted by two counters
/// (lane TV, Codex round 12). The name is `concat!(prefix, field)` now, so it
/// cannot name another field, and every consumer takes the array.
macro_rules! convoy_named_counters {
    (
        $struct:ident, prefix = $prefix:literal, all = $all_fn:ident;
        $( $(#[$group_doc:meta])* $group_fn:ident { $( $field:ident, )+ } )+
    ) => {
        $crate::convoy_wire_taxonomy::convoy_named_counters! {
            $struct, prefix = $prefix;
            $( $(#[$group_doc])* $group_fn { $( $field, )+ } )+
        }

        impl $struct {
            pub fn $all_fn(&self) -> [(&'static str, u64); [$($(stringify!($field),)+)+].len()] {
                [$($((concat!($prefix, stringify!($field)), self.$field),)+)+]
            }
        }
    };
    (
        $struct:ident, prefix = $prefix:literal;
        $( $(#[$group_doc:meta])* $group_fn:ident { $( $field:ident, )+ } )+
    ) => {
        impl $struct {
            $(
                $(#[$group_doc])*
                pub fn $group_fn(&self) -> [(&'static str, u64); [$(stringify!($field),)+].len()] {
                    [$((concat!($prefix, stringify!($field)), self.$field),)+]
                }
            )+
        }

        const _: () = {
            assert!(
                compact_contract::diagnostic_channel::const_names_are_distinct(&[
                    $($(concat!($prefix, stringify!($field)),)+)+
                ]),
                "two telemetry counters publish the same name",
            );
        };
    };
}

pub(crate) use convoy_capture_error_subsites;
pub(crate) use convoy_named_counters;
pub(crate) use convoy_route_refusal_causes;
pub(crate) use convoy_subsite_table;
