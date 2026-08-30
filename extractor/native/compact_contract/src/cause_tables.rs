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
/// `dense` is for a taxonomy whose code is also its POSITION -- a counter
/// array addressed by the discriminant while the totals iterate `$all`, so the
/// two orders were independent and a row replaced by a duplicate dropped one
/// member from every total and double-counted another (lane TV, row R17). The
/// codes are held to a bijection over the count, and the count comes off the
/// column.
#[macro_export]
macro_rules! wire_coded_variants {
    (
        dense $enum:ident : $repr:ty, all = $avis:vis $all:ident,
        count = $cvis:vis $count:ident, index = $ivis:vis $index_fn:ident,
        label = $lvis:vis $label_fn:ident;
        $( $variant:ident = $code:literal => $label:literal, )+
    ) => {
        impl $enum {
            $avis const $all: [Self; [$($code,)+].len()] = [$(Self::$variant,)+];

            $ivis const fn $index_fn(self) -> $repr {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            $lvis const fn $label_fn(self) -> &'static str {
                match self {
                    $(Self::$variant => $label,)+
                }
            }
        }

        $cvis const $count: usize = $enum::$all.len();

        const _: () = {
            $(assert!(
                $enum::$variant as $repr == $code,
                "a value's declared index is not its own discriminant",
            );)+
            assert!(
                $crate::diagnostic_channel::indices_are_a_bijection(&[$($code,)+], $count),
                "a dense taxonomy's codes are not its own positions",
            );
            assert!(
                $crate::diagnostic_channel::const_names_are_distinct(&[$($label,)+]),
                "two values carry the same label",
            );
        };
    };
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
///
/// `payloads` is `escape` for a taxonomy with SEVERAL payload-carrying
/// members, where the payload travels in its own field rather than inside the
/// code. `tuple` and `named` name the two variant shapes and each row carries
/// its field once; the generated decoder takes those fields as parameters, so
/// the caller reads them from the wire and the macro decides which variant
/// receives them. `never` is a member the encoder may spell that the wire may
/// not carry (lane TV, row R8).
#[macro_export]
macro_rules! wire_coded_codes {
    (
        payloads $enum:ident : $repr:ty,
        code = $cvis:vis $code_fn:ident,
        $decode:ident = $dvis:vis $from_fn:ident($arg:ident) -> $ret:ty { $fallback:expr };
        $( $variant:ident = $code:literal, )+
        ; tuple $( $tvariant:ident($tfield:ident : $tty:ty) = $tcode:literal, )*
        ; named $( $nvariant:ident { $nfield:ident : $nty:ty } = $ncode:literal, )*
        ; never $( $xvariant:ident = $xcode:literal, )*
    ) => {
        impl $enum {
            $cvis const fn $code_fn(self) -> $repr {
                match self {
                    $(Self::$variant => $code,)+
                    $(Self::$tvariant(..) => $tcode,)*
                    $(Self::$nvariant { .. } => $ncode,)*
                    $(Self::$xvariant => $xcode,)*
                }
            }

            $dvis fn $from_fn(
                $arg: $repr,
                $($tfield: $tty,)*
                $($nfield: $nty,)*
            ) -> $ret {
                $(if $arg == $code {
                    return $decode(Self::$variant);
                })+
                $(if $arg == $tcode {
                    return $decode(Self::$tvariant($tfield));
                })*
                $(if $arg == $ncode {
                    return $decode(Self::$nvariant { $nfield });
                })*
                $fallback
            }
        }

        $crate::wire_coded_codes! {
            @distinct $repr; $( $code, )+ $( $tcode, )* $( $ncode, )* $( $xcode, )*
        }
    };
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
        ; escape masked $evariant:ident, flag = $flag:expr, payload = $payload:expr
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

/// A table of wire flag BITS with one wire leaf each.
///
/// THE DEFECT. The bits are free constants, the known mask is a hand-written
/// OR of them, and the names are a third roster. A bit added to the constants
/// and the mask but left out of the roster compiles, is ACCEPTED on the wire,
/// and reaches the host with no name (lane TV, rows R9, R10, R12 and R16).
///
/// The bit's constant, its shift and its leaf are written once; the mask and
/// the roster come off that column, so a bit cannot be known and nameless.
///
/// `explicit_leaves` is for a taxonomy whose published names deliberately do
/// not spell their constants -- only there does the spelling assert drop out,
/// and the leaf is a literal instead of an identifier.
///
/// `classified` carries a second mask that is a per-bit POLICY rather than the
/// taxonomy: each row declares `failure` or `informational` and the failure
/// mask is the rows that said `failure`, so a bit added without a class does
/// not compile.
macro_rules! bit_flag_causes {
    (
        repr = $repr:ty, mask = $mvis:vis $mask:ident, names = $nvis:vis $names:ident,
        bit_prefix = $bit_prefix:literal;
        $( $(#[$doc:meta])* $bit:ident = $shift:literal => $leaf:ident, )+
    ) => {
        crate::cause_tables::bit_flag_causes! {
            @core repr = $repr, mask = $mvis $mask, names = $nvis $names;
            $( $(#[$doc])* $bit = $shift => stringify!($leaf), )+
        }

        const _: () = {
            $(assert!(
                crate::diagnostic_channel::const_str_eq_ignore_ascii_case(
                    stringify!($bit),
                    concat!($bit_prefix, stringify!($leaf)),
                ),
                "a flag bit's constant and its wire leaf name different causes",
            );)+
        };
    };
    (
        explicit_leaves repr = $repr:ty, mask = $mvis:vis $mask:ident,
        names = $nvis:vis $names:ident;
        $( $(#[$doc:meta])* $bit:ident = $shift:literal => $leaf:literal, )+
    ) => {
        crate::cause_tables::bit_flag_causes! {
            @core repr = $repr, mask = $mvis $mask, names = $nvis $names;
            $( $(#[$doc])* $bit = $shift => $leaf, )+
        }
    };
    (
        classified repr = $repr:ty, mask = $mvis:vis $mask:ident,
        names = $nvis:vis $names:ident, failure_mask = $fvis:vis $failure:ident,
        bit_prefix = $bit_prefix:literal;
        $( $(#[$doc:meta])* $bit:ident = $shift:literal => $leaf:ident : $class:ident, )+
    ) => {
        crate::cause_tables::bit_flag_causes! {
            repr = $repr, mask = $mvis $mask, names = $nvis $names,
            bit_prefix = $bit_prefix;
            $( $(#[$doc])* $bit = $shift => $leaf, )+
        }

        $fvis const $failure: $repr = 0
            $(| crate::cause_tables::bit_flag_causes!(@class $repr, $shift, $class))+;
    };
    (@class $repr:ty, $shift:literal, failure) => { (1 as $repr) << $shift };
    (@class $repr:ty, $shift:literal, informational) => { 0 as $repr };
    (
        associated $owner:ident, repr = $repr:ty, mask = $mvis:vis $mask:ident,
        names = $nvis:vis $names:ident;
        $( $(#[$doc:meta])* $bit:ident = $shift:literal => $leaf:literal, )+
    ) => {
        impl $owner {
            $($(#[$doc])* $mvis const $bit: $repr = (1 as $repr) << $shift;)+

            $mvis const $mask: $repr = 0 $(| Self::$bit)+;

            $nvis const $names: &'static [($repr, &str)] = &[$((Self::$bit, $leaf),)+];
        }

        crate::cause_tables::bit_flag_causes! { @asserts $repr; $( $shift => $leaf, )+ }
    };
    (
        @core repr = $repr:ty, mask = $mvis:vis $mask:ident, names = $nvis:vis $names:ident;
        $( $(#[$doc:meta])* $bit:ident = $shift:literal => $leaf:expr, )+
    ) => {
        $($(#[$doc])* $mvis const $bit: $repr = (1 as $repr) << $shift;)+

        $mvis const $mask: $repr = 0 $(| $bit)+;

        $nvis const $names: &[($repr, &str)] = &[$(($bit, $leaf),)+];

        crate::cause_tables::bit_flag_causes! { @asserts $repr; $( $shift => $leaf, )+ }
    };
    (@asserts $repr:ty; $( $shift:literal => $leaf:expr, )+) => {
        const _: () = {
            let shifts = [$($shift,)+];
            let mut i = 0;
            while i < shifts.len() {
                assert!(
                    shifts[i] < (<$repr>::BITS as usize),
                    "a flag bit is shifted past the end of its own repr",
                );
                let mut j = i + 1;
                while j < shifts.len() {
                    assert!(shifts[i] != shifts[j], "two flag causes share a bit");
                    j += 1;
                }
                i += 1;
            }
            assert!(
                crate::diagnostic_channel::const_names_are_distinct(&[$($leaf,)+]),
                "two flag causes are published under the same name",
            );
        };
    };
}

/// A table of wire cause CODES whose published leaf deliberately does not
/// spell its constant.
///
/// Same defect as [`bit_flag_causes`] without a mask: the codes are free
/// constants and the roster that names them is a second list, so a constant
/// used in production but missing from the roster compiles and its diagnostic
/// is refused at runtime for having no name (lane TV, row R13).
macro_rules! coded_cause_names {
    (
        repr = $repr:ty, roster = $rvis:vis $roster:ident;
        $( $(#[$doc:meta])* $code_name:ident = $code:literal => $leaf:literal, )+
    ) => {
        $($(#[$doc])* $rvis const $code_name: $repr = $code;)+

        $rvis const $roster: &[($repr, &str)] = &[$(($code_name, $leaf),)+];

        const _: () = {
            let codes = [$($code,)+];
            let mut i = 0;
            while i < codes.len() {
                let mut j = i + 1;
                while j < codes.len() {
                    assert!(codes[i] != codes[j], "two causes share a code");
                    j += 1;
                }
                i += 1;
            }
            let mut leaves = [""; [$($code,)+].len()];
            let mut k = 0;
            while k < leaves.len() {
                leaves[k] = $roster[k].1;
                k += 1;
            }
            assert!(
                crate::diagnostic_channel::const_names_are_distinct(&leaves),
                "two causes are published under the same name",
            );
        };
    };
}

/// A histogram whose bucket BOUNDS and bucket NAMES were two positional
/// arrays.
///
/// THE DEFECT. Moving a bound without editing the name beside it compiled and
/// published values up to the new bound under the old bound's name (lane TV,
/// row R15). The bound is one number per row now: the microsecond bound is
/// that number scaled and the leaf spells it, so neither can move alone.
///
/// `last` is the final bounded bucket. Its number also spells the overflow
/// bucket that follows it, which is the one name with no bound of its own.
macro_rules! millisecond_buckets {
    (
        bounds = $bvis:vis $bounds:ident, names = $nvis:vis $names:ident,
        unit_prefix = $prefix:literal;
        $( $ms:literal, )* last $last_ms:literal,
    ) => {
        $bvis const $bounds: [u64; [$($ms,)* $last_ms,].len()] =
            [$($ms * 1_000,)* $last_ms * 1_000];

        $nvis const $names: [&str; $bounds.len() + 1] = [
            $(concat!($prefix, "le", stringify!($ms), "ms"),)*
            concat!($prefix, "le", stringify!($last_ms), "ms"),
            concat!($prefix, "gt", stringify!($last_ms), "ms"),
        ];

        const _: () = {
            let mut i = 1;
            while i < $bounds.len() {
                assert!(
                    $bounds[i - 1] < $bounds[i],
                    "the bucket bounds do not increase, so a value falls in the earlier bucket",
                );
                i += 1;
            }
            assert!(
                crate::diagnostic_channel::const_names_are_distinct(&$names),
                "two buckets are published under the same name",
            );
        };
    };
}

/// A span taxonomy whose code, semantic name and positional unit name were
/// three tables on two sides of the contract/collector boundary.
///
/// THE DEFECT. The contract held the codes and the names; the collector held
/// the SAME codes again as its own enum's discriminants and published the
/// spans by position. Exchanging two discriminants in the collector compiled
/// and published each span under the other segment's wire slot (lane TV, row
/// R14). The enum is the contract's now and the column below is its only code
/// authority: the declared code must be the discriminant, the codes must be
/// their own positions -- so the positional publication is the same order --
/// the unit name is spelled from the code rather than beside it, and the
/// published name must spell its own variant, which is the only thing that
/// stops two `=> "name"` literals being exchanged with every other assertion
/// still holding.
macro_rules! ordered_segments {
    (
        $enum:ident, segments = $svis:vis $segments:ident,
        unit_names = $uvis:vis $units:ident, count = $cvis:vis $count:ident,
        unit_prefix = $pvis:vis $prefix_name:ident = $prefix:literal,
        unit_suffix = $svis2:vis $suffix_name:ident = $suffix:literal;
        $( $variant:ident = $code:literal => $name:literal, )+
    ) => {
        $svis const $segments: &[(u16, &str)] = &[$(($code, $name),)+];

        $uvis const $units: &[&str] = &[$(concat!($prefix, stringify!($code), $suffix),)+];

        // THE WIRE PREFIX AND SUFFIX, EXPORTED. They are what a reader splits a
        // published unit name on, and they were written here and then written
        // AGAIN in every decoder -- `strip_prefix("lw_o")` in one binary,
        // `strip_prefix('o')` in another, `"lw_t"` in two more. That is R18
        // exactly: one coded meaning in crates that cannot see each other, so
        // editing this literal renamed every span and left every decoder
        // looking for the old name (Codex round 32). Minted from the same
        // literal the names are built from, so there is one authority.
        $pvis const $prefix_name: &str = $prefix;

        $svis2 const $suffix_name: &str = $suffix;

        $cvis const $count: usize = $segments.len();

        const _: () = {
            $(assert!(
                $enum::$variant as u16 == $code,
                "a segment's declared code is not its own discriminant",
            );)+
            // Over `$segments` -- the table that SHIPS -- not over the macro's
            // arguments. Validating the arguments validates nothing a consumer
            // reads: an emitted array built from them by a const expression can
            // differ from them entirely, with every argument-side assertion
            // still true.
            assert!(
                $segments.len() == $count && $count == [$($code,)+].len(),
                "the published table is not as long as the rows it was built from",
            );
            assert!(
                crate::diagnostic_channel::indices_are_a_bijection(&[$($code,)+], $count),
                "two segments share a code, or one is past the end of the table",
            );
            assert!(
                crate::diagnostic_channel::const_segment_codes_are_their_own_positions(
                    $segments),
                "a segment is not published in its own code's position, so the \
                 unit names and the accumulator slots are in different orders",
            );
            assert!(
                crate::diagnostic_channel::const_segment_names_are_distinct($segments),
                "two segments are published under the same name",
            );
            $(assert!(
                crate::diagnostic_channel::const_segment_spells_the_variant(
                    $segments, $code, stringify!($variant),
                ),
                "a segment's published name does not spell its own variant",
            );)+
            $(assert!(
                crate::diagnostic_channel::const_segment_name_is(
                    $segments, $code, $name,
                ),
                "the published table does not carry the name the row declares",
            );)+
            assert!(
                $units.len() == $count,
                "the published unit names are not as many as the segments",
            );
            $(assert!(
                // Against the EXPORTED constants, not the literals that built
                // the names. Validating `$prefix` against `$prefix` says
                // nothing about what a consumer reads, and the exported pair
                // could be declared from anything (Codex round 33, finding 1).
                crate::diagnostic_channel::const_unit_is_spelled(
                    $units[$code], $prefix_name, $suffix_name, $code,
                ),
                "a published WIRE UNIT NAME is not its prefix, its own code                  and its suffix -- the host keys the span by this",
            );)+
            assert!(
                crate::diagnostic_channel::const_no_spelling_is_a_prefix_of_another(
                    &[$(stringify!($variant),)+],
                ),
                "one variant's spelling is a prefix of another's, so their names \
                 could be exchanged and still spell them",
            );
        };
    };
}

pub(crate) use bit_flag_causes;
pub(crate) use coded_cause_names;
pub(crate) use millisecond_buckets;
pub(crate) use ordered_segments;
pub(crate) use numbered_cause_id_pairs;
pub(crate) use numbered_cause_ids;
pub(crate) use wire_coded_causes;
