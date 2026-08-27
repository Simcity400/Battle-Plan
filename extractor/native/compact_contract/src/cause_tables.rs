//! One declaration per cause id, for the tables whose numeric constant and
//! wire name were two lists and whose exhaustiveness was a third, written by
//! hand in a test.
//!
//! THE DEFECT. A new id given an index another id already holds, and left out
//! of the test's inventory, compiled and published its count under the other
//! id's name. Nothing could see it: the constants are free `usize`s, the names
//! are an array indexed by them, and the test repeated both by hand (lane TV,
//! Codex round 5).
//!
//! The declaration carries the constant, its index and its leaf once. The
//! constant's own spelling is held to the leaf, case for case, so an id cannot
//! be named after another cause; the indices are held to a bijection over the
//! count; and the names are held distinct, because the manifest aggregates by
//! name.

/// A table of `usize` cause ids with one wire unit each.
macro_rules! numbered_cause_ids {
    (
        count = $count:ident, names = $names:ident,
        unit_prefix = $unit_prefix:literal, id_prefix = $id_prefix:literal;
        $( $(#[$doc:meta])* $id:ident = $index:literal => $leaf:ident, )+
    ) => {
        $($(#[$doc])* pub const $id: usize = $index;)+

        pub const $count: usize = [$($index,)+].len();

        pub const $names: [&str; $count] = {
            let mut names = [""; $count];
            $(names[$index] = concat!($unit_prefix, stringify!($leaf));)+
            names
        };

        const _: () = {
            assert!(
                crate::diagnostic_channel::indices_are_a_bijection(&[$($index,)+], $count),
                "two cause ids share an index, or one indexes past the table",
            );
            $(assert!(
                crate::diagnostic_channel::const_str_eq_ignore_ascii_case(
                    stringify!($id),
                    concat!($id_prefix, stringify!($leaf)),
                ),
                "a cause id's constant and its wire leaf name different causes",
            );)+
            assert!(
                crate::diagnostic_channel::const_names_are_distinct(&$names),
                "two causes are published under the same unit name",
            );
        };
    };
}

/// A table of `usize` cause ids with a COUNT unit and a SAMPLE unit each.
macro_rules! numbered_cause_id_pairs {
    (
        count = $count:ident, names = $names:ident, unit_prefix = $unit_prefix:literal,
        count_suffix = $count_suffix:literal, sample_suffix = $sample_suffix:literal,
        id_prefix = $id_prefix:literal;
        $( $(#[$doc:meta])* $id:ident = $index:literal => $leaf:ident, )+
    ) => {
        $($(#[$doc])* pub const $id: usize = $index;)+

        pub const $count: usize = [$($index,)+].len();

        pub const $names: [(&str, &str); $count] = {
            let mut names = [("", ""); $count];
            $(names[$index] = (
                concat!($unit_prefix, stringify!($leaf), $count_suffix),
                concat!($unit_prefix, stringify!($leaf), $sample_suffix),
            );)+
            names
        };

        const _: () = {
            assert!(
                crate::diagnostic_channel::indices_are_a_bijection(&[$($index,)+], $count),
                "two cause ids share an index, or one indexes past the table",
            );
            $(assert!(
                crate::diagnostic_channel::const_str_eq_ignore_ascii_case(
                    stringify!($id),
                    concat!($id_prefix, stringify!($leaf)),
                ),
                "a cause id's constant and its wire leaf name different causes",
            );)+
            let mut i = 0;
            while i < $count {
                let mut j = i + 1;
                while j < $count {
                    assert!(
                        !crate::diagnostic_channel::const_str_eq($names[i].0, $names[j].0)
                            && !crate::diagnostic_channel::const_str_eq($names[i].1, $names[j].1),
                        "two causes are published under the same unit name",
                    );
                    j += 1;
                }
                i += 1;
            }
        };
    };
}

/// A wire cause with a code, a name, and both directions of each.
///
/// The encoder's `match` and the decoder's `match` were two tables over one
/// mapping, so exchanging the codes two causes return compiled and
/// reconstructed each loss as the other (Codex round 8). Both directions and
/// both spellings come from one row now, the code is asserted to be the
/// variant's own discriminant where it has one, and the name must spell the
/// variant.
macro_rules! wire_coded_causes {
    (
        $enum:ident : $repr:ty, all = $all:ident, code = $code_fn:ident,
        from_code = $from_code:ident, wire_name = $name_fn:ident,
        from_wire_name = $from_name:ident, discriminant_is_code = $pinned:literal;
        $( $variant:ident = $code:path => $name:literal, )+
    ) => {
        impl $enum {
            pub const $all: [Self; [$($code,)+].len()] = [$(Self::$variant,)+];

            pub const fn $code_fn(self) -> $repr {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            /// `None` for a code this build does not carry. Callers that owe a
            /// named error wrap it; there is one mapping either way.
            pub const fn $from_code(code: $repr) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)+
                    _ => None,
                }
            }

            pub const fn $name_fn(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }

            pub fn $from_name(name: &str) -> Option<Self> {
                $(if crate::diagnostic_channel::const_str_eq(name, $name) {
                    return Some(Self::$variant);
                })+
                None
            }
        }

        const _: () = {
            $(assert!(
                crate::diagnostic_channel::const_str_is_snake_of($name, stringify!($variant)),
                "a wire name does not spell the cause it names",
            );)+
            assert!(
                crate::diagnostic_channel::const_names_are_distinct(&[$($name,)+]),
                "two causes are published under the same wire name",
            );
            let codes = [$($code,)+];
            let mut i = 0;
            while i < codes.len() {
                let mut j = i + 1;
                while j < codes.len() {
                    assert!(codes[i] != codes[j], "two causes share a wire code");
                    j += 1;
                }
                i += 1;
            }
            $(assert!(
                !$pinned || $enum::$variant as $repr == $code,
                "a cause's declared code is not its own discriminant",
            );)+
        };
    };
}
/// A stable-ABI enum whose code, decoder and label were three tables.
///
/// The discriminant is the encoder and a literal `match` was the decoder, so
/// exchanging two discriminants compiled and decoded each value as the other
/// (Codex round 10). The row carries the variant, its code and its label; the
/// code is asserted to BE the discriminant, the decoder is generated from the
/// same column, and the label match is exhaustive so a variant absent from the
/// declaration does not compile.
///
/// `reserved` is for a variant that has a label but is NOT decodable -- a
/// sentinel a reader must never mint from the wire.
#[macro_export]
macro_rules! wire_coded_variants {
    (
        $enum:ident : $repr:ty, all = $all:ident, code = $code_fn:ident,
        from_code = $from_code:ident, label = $label_fn:ident;
        codes { $( $variant:ident = $code:literal => $label:literal, )+ }
        reserved { $( $rvariant:ident = $rcode:literal => $rlabel:literal, )* }
    ) => {
        impl $enum {
            pub const $all: [Self; [$($code,)+ $($rcode,)*].len()] =
                [$(Self::$variant,)+ $(Self::$rvariant,)*];

            pub const fn $code_fn(self) -> $repr {
                match self {
                    $(Self::$variant => $code,)+
                    $(Self::$rvariant => $rcode,)*
                }
            }

            pub const fn $from_code(code: $repr) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)+
                    _ => None,
                }
            }

            pub const fn $label_fn(self) -> &'static str {
                match self {
                    $(Self::$variant => $label,)+
                    $(Self::$rvariant => $rlabel,)*
                }
            }
        }

        const _: () = {
            $(assert!(
                $enum::$variant as $repr == $code,
                "a value's declared code is not its own discriminant",
            );)+
            $(assert!(
                $enum::$rvariant as $repr == $rcode,
                "a reserved value's declared code is not its own discriminant",
            );)*
            let codes = [$($code,)+ $($rcode,)*];
            let mut i = 0;
            while i < codes.len() {
                let mut j = i + 1;
                while j < codes.len() {
                    assert!(codes[i] != codes[j], "two values share a wire code");
                    j += 1;
                }
                i += 1;
            }
            assert!(
                $crate::diagnostic_channel::const_names_are_distinct(
                    &[$($label,)+ $($rlabel,)*],
                ),
                "two values carry the same label",
            );
        };
    };
}
/// A `#[repr]` wire enum whose ONE code column is both the encoder and the
/// decoder.
///
/// The discriminant was the encoder -- `kind as u16` -- and a hand-written
/// literal `match` the decoder, so exchanging two discriminants compiled and
/// decoded each value as the other, and a variant added without a decoder arm
/// compiled and emitted a code its own decoder refuses (lane TV, Codex round
/// 11). Both directions are generated from the column below: the encoder as an
/// exhaustive match, so an omitted variant is E0004, and the decoder from the
/// same literals, with the declared code asserted to BE the discriminant.
///
/// `$decode` is `Ok` or `Some`; `$fallback` is evaluated with `$arg` bound to
/// the undecodable code. A code is an expression, not a pattern, so a named
/// constant can be the code without a second copy of its value.
/// `escape` is for a taxonomy with one member the column cannot hold: a
/// payload-carrying variant. It comes in the only two shapes that can be proven
/// at compile time, because a free-form guard could not be (Codex round 14):
/// `reserved CODE { V with PAYLOAD }`, one code that joins the distinctness
/// array, where the variant is named ONCE and drives both directions -- with
/// `{ V => never }` for a member the encoder may spell but the wire may not
/// carry (Codex round 15: taking a pattern and a value separately let the two
/// directions name different variants under one code) -- and
/// `masked V, flag = F, payload = P`, where the code is a passthrough value
/// beside a flag bit, `F` and `P` partition the repr, and no ordinary code
/// carries `F`. In both, ONE spelling drives encode and decode.
///
/// `no_discriminant` is for an enum whose wire code is not a discriminant at
/// all -- no `#[repr]`, or codes that are not the `0..` ordinals. Everything
/// else is the same law; only the code-is-the-discriminant assert drops out,
/// because there is no discriminant for it to hold.
#[macro_export]
macro_rules! wire_coded_codes {
    (
        no_discriminant $enum:ident : $repr:ty,
        code = $cvis:vis $code_fn:ident,
        $decode:ident = $dvis:vis $from_fn:ident($arg:ident) -> $ret:ty { $fallback:expr };
        $( $variant:ident = $code:expr, )+
        ; escape reserved $ecode:literal { $evariant:ident => never }
    ) => {
        impl $enum {
            $cvis fn $code_fn(self) -> $repr {
                match self {
                    $(Self::$variant => $code,)+
                    Self::$evariant { .. } => $ecode,
                }
            }

            $dvis fn $from_fn($arg: $repr) -> $ret {
                $(if $arg == $code {
                    return $decode(Self::$variant);
                })+
                $fallback
            }
        }

        // `never` says the member has a code but may not arrive on the wire:
        // the encoder can spell it, the decoder refuses it like any unassigned
        // value. Its code is still in the distinctness array, so a later
        // ordinary member cannot be given it.
        $crate::wire_coded_codes! { @distinct $repr; $( $code, )+ $ecode, }
    };
    (
        no_discriminant $enum:ident : $repr:ty,
        code = $cvis:vis $code_fn:ident,
        $decode:ident = $dvis:vis $from_fn:ident($arg:ident) -> $ret:ty { $fallback:expr };
        $( $variant:ident = $code:expr, )+
        ; escape reserved $ecode:literal { $evariant:ident with $default:expr }
    ) => {
        impl $enum {
            $cvis fn $code_fn(self) -> $repr {
                match self {
                    $(Self::$variant => $code,)+
                    Self::$evariant { .. } => $ecode,
                }
            }

            $dvis fn $from_fn($arg: $repr) -> $ret {
                $(if $arg == $code {
                    return $decode(Self::$variant);
                })+
                if $arg == $ecode {
                    return $decode(Self::$evariant($default));
                }
                $fallback
            }
        }

        // The reserved code is IN the distinctness array, so it cannot collide
        // with an ordinary one, and both directions read it from here.
        $crate::wire_coded_codes! { @distinct $repr; $( $code, )+ $ecode, }
    };
    (
        no_discriminant $enum:ident : $repr:ty,
        code = $cvis:vis $code_fn:ident,
        $decode:ident = $dvis:vis $from_fn:ident($arg:ident) -> $ret:ty { $fallback:expr };
        $( $variant:ident = $code:expr, )+
        ; escape masked $evariant:ident, flag = $flag:literal, payload = $payload:literal
    ) => {
        impl $enum {
            $cvis const fn $code_fn(self) -> $repr {
                match self {
                    $(Self::$variant => $code,)+
                    Self::$evariant(payload) => payload | $flag,
                }
            }

            $dvis fn $from_fn($arg: $repr) -> $ret {
                $(if $arg == $code {
                    return $decode(Self::$variant);
                })+
                if $arg & $flag != 0 {
                    return $decode(Self::$evariant($arg & $payload));
                }
                $fallback
            }
        }

        $crate::wire_coded_codes! { @distinct $repr; $( $code, )+ }

        // Both directions come off `flag`, so the encoder cannot leave the
        // escape's domain: `payload | flag` always carries the flag. What is
        // left to prove is that no ordinary code carries it, and that the
        // payload the decoder keeps is what the encoder passed through
        // (Codex rounds 13 and 14).
        const _: () = {
            assert!($flag != 0, "the escape's flag selects nothing");
            assert!(
                $flag & $payload == 0 && $flag | $payload == <$repr>::MAX,
                "the escape's flag and payload do not partition the code",
            );
            $(assert!(
                $code & $flag == 0,
                "an ordinary wire code carries the escape's own flag",
            );)+
        };
    };
    (
        no_discriminant $enum:ident : $repr:ty,
        code = $cvis:vis $code_fn:ident,
        $decode:ident = $dvis:vis $from_fn:ident($arg:ident) -> $ret:ty { $fallback:expr };
        $( $variant:ident = $code:expr, )+
    ) => {
        $crate::wire_coded_codes! {
            @table $enum : $repr,
            code = $cvis $code_fn,
            $decode = $dvis $from_fn($arg) -> $ret { $fallback };
            $( $variant = $code, )+
        }
    };
    (
        $enum:ident : $repr:ty,
        code = $cvis:vis $code_fn:ident,
        $decode:ident = $dvis:vis $from_fn:ident($arg:ident) -> $ret:ty { $fallback:expr };
        $( $variant:ident = $code:expr, )+
    ) => {
        $crate::wire_coded_codes! {
            @table $enum : $repr,
            code = $cvis $code_fn,
            $decode = $dvis $from_fn($arg) -> $ret { $fallback };
            $( $variant = $code, )+
        }

        const _: () = {
            $(assert!(
                $enum::$variant as $repr == $code,
                "a wire value's declared code is not its own discriminant",
            );)+
        };
    };
    (
        @table $enum:ident : $repr:ty,
        code = $cvis:vis $code_fn:ident,
        $decode:ident = $dvis:vis $from_fn:ident($arg:ident) -> $ret:ty { $fallback:expr };
        $( $variant:ident = $code:expr, )+
    ) => {
        impl $enum {
            $cvis const fn $code_fn(self) -> $repr {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            $dvis fn $from_fn($arg: $repr) -> $ret {
                $(if $arg == $code {
                    return $decode(Self::$variant);
                })+
                $fallback
            }
        }

        $crate::wire_coded_codes! { @distinct $repr; $( $code, )+ }
    };
    (@distinct $repr:ty; $( $code:expr, )+) => {
        const _: () = {
            let codes: [$repr; [$($code,)+].len()] = [$($code,)+];
            let mut i = 0;
            while i < codes.len() {
                let mut j = i + 1;
                while j < codes.len() {
                    assert!(codes[i] != codes[j], "two wire values share a code");
                    j += 1;
                }
                i += 1;
            }
        };
    };
}

pub(crate) use numbered_cause_id_pairs;
pub(crate) use numbered_cause_ids;
pub(crate) use wire_coded_causes;
