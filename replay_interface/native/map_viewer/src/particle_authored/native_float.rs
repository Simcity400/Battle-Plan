#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RoundingMode { NearestEven, Down, Up, TowardZero }

pub(crate) struct Context<'a> {
    pub decimal_point: u8,
    pub whitespace: [u64; 4],
    pub rounding: RoundingMode,
    pub special_tokens: Option<&'a SpecialTokens>,
}

pub(crate) struct SpecialTokens {
    pub infinity: [[u8; 3]; 2],
    pub infinity_tail: [[u8; 5]; 2],
    pub nan: [[u8; 3]; 2],
    pub signaling_tail: [[u8; 5]; 2],
    pub indeterminate_tail: [[u8; 4]; 2],
}

impl Context<'_> {
    fn space(&self, byte: u8) -> bool {
        self.whitespace[usize::from(byte / 64)] & (1u64 << (byte % 64)) != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReadOutcome { Assigned, RetainedAtEof }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReadRefusal { Encoding, MatchingFailure, MissingContext, MissingSpecialTokens, IntegerCapacity }

impl std::fmt::Display for ReadRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Encoding => "entity.cull_radius.encoding",
            Self::MatchingFailure => "entity.cull_radius.native_matching_failure",
            Self::MissingContext => "entity.cull_radius.native_float_context_unavailable",
            Self::MissingSpecialTokens => "entity.cull_radius.native_special_tokens_unavailable",
            Self::IntegerCapacity => "entity.cull_radius.native_integer_capacity",
        })
    }
}
impl std::error::Error for ReadRefusal {}

struct Input<'a> {
    bytes: &'a [u8],
    position: usize,
    reads: usize,
    current: u8,
    valid: bool,
}

impl Input<'_> {
    fn get(&mut self) {
        self.reads += 1;
        self.current = self.bytes.get(self.position).copied().unwrap_or(0);
        if self.position < self.bytes.len() { self.position += 1; }
    }
    fn unget(&mut self) {
        self.reads -= 1;
        if !matches!(self.current, 0 | 255) && self.position != 0 { self.position -= 1; }
    }
    fn reset(&mut self, mark: usize) -> bool {
        self.unget();
        self.current = 0;
        let unchanged = self.reads == mark;
        self.valid &= unchanged;
        unchanged
    }
    fn pair<const N: usize>(&mut self, pair: &[[u8; N]; 2]) -> bool {
        for index in 0..N {
            if self.current != pair[0][index] && self.current != pair[1][index] { return false; }
            self.get();
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind { Decimal, Hex, Zero, Infinity, QuietNan, SignalingNan, Indeterminate, Invalid, Underflow, Overflow }

struct Number {
    negative: bool,
    exponent: i32,
    digits: [u8; 768],
    count: usize,
}
impl Number {
    fn push(&mut self, digit: u8) {
        if self.count < self.digits.len() {
            self.digits[self.count] = digit;
            self.count += 1;
        }
    }
}

fn digit(byte: u8) -> u32 {
    match byte {
        b'0'..=b'9' => u32::from(byte - b'0'),
        b'a'..=b'z' => u32::from(byte - b'a') + 10,
        b'A'..=b'Z' => u32::from(byte - b'A') + 10,
        _ => u32::MAX,
    }
}

fn parse(input: &mut Input<'_>, number: &mut Number, context: &Context<'_>) -> Result<Kind, ReadRefusal> {
    let mut mark = input.reads;
    input.get();
    while context.space(input.current) { input.get(); }
    number.negative = input.current == b'-';
    if matches!(input.current, b'+' | b'-') { input.get(); }
    if matches!(input.current, b'i' | b'I' | b'n' | b'N') {
        let tokens = context.special_tokens.ok_or(ReadRefusal::MissingSpecialTokens)?;
        if matches!(input.current, b'i' | b'I') {
            if !input.pair(&tokens.infinity) { input.reset(mark); return Ok(Kind::Invalid); }
            input.unget();
            let tail = input.reads;
            input.get();
            if !input.pair(&tokens.infinity_tail) {
                return Ok(if input.reset(tail) { Kind::Infinity } else { Kind::Invalid });
            }
            input.unget();
            return Ok(Kind::Infinity);
        }
        if !input.pair(&tokens.nan) { input.reset(mark); return Ok(Kind::Invalid); }
        input.unget();
        let tail = input.reads;
        input.get();
        if input.current == b'(' {
            input.get();
            if input.pair(&tokens.signaling_tail) { input.unget(); return Ok(Kind::SignalingNan); }
            // The native second recognizer continues from the first recognizer's cursor.
            if input.pair(&tokens.indeterminate_tail) { input.unget(); return Ok(Kind::Indeterminate); }
            loop {
                if input.current == b')' { return Ok(Kind::QuietNan); }
                if !(input.current.is_ascii_alphanumeric() || input.current == b'_') { break; }
                input.get();
            }
        }
        return Ok(if input.reset(tail) { Kind::QuietNan } else { Kind::Invalid });
    }
    let mut hex = false;
    if input.current == b'0' {
        let before_prefix = input.reads;
        input.get();
        if matches!(input.current, b'x' | b'X') {
            hex = true;
            mark = before_prefix;
            input.get();
        } else {
            input.unget();
            input.current = b'0';
        }
    }
    let mut saw_digit = false;
    while input.current == b'0' { saw_digit = true; input.get(); }
    let mut integer_digits = 0i32;
    let max_digit = if hex { 15 } else { 9 };
    while digit(input.current) <= max_digit {
        saw_digit = true;
        number.push(digit(input.current) as u8);
        integer_digits = integer_digits.wrapping_add(1);
        input.get();
    }
    if input.current == context.decimal_point {
        input.get();
        if number.count == 0 {
            while input.current == b'0' {
                saw_digit = true;
                integer_digits = integer_digits.wrapping_sub(1);
                input.get();
            }
        }
        while digit(input.current) <= max_digit {
            saw_digit = true;
            number.push(digit(input.current) as u8);
            input.get();
        }
    }
    if !saw_digit {
        return Ok(if input.reset(mark) && hex { Kind::Zero } else { Kind::Invalid });
    }
    input.unget();
    let exponent_mark = input.reads;
    input.get();
    let mut exponent = 0i32;
    if (hex && matches!(input.current, b'p' | b'P')) || (!hex && matches!(input.current, b'e' | b'E')) {
        input.get();
        let negative = input.current == b'-';
        if matches!(input.current, b'+' | b'-') { input.get(); }
        let mut exponent_digits = false;
        while input.current.is_ascii_digit() {
            exponent_digits = true;
            if exponent <= 5200 { exponent = (exponent * 10 + i32::from(input.current - b'0')).min(5201); }
            input.get();
        }
        if negative { exponent = -exponent; }
        if !exponent_digits {
            if !input.reset(exponent_mark) { return Ok(Kind::Invalid); }
            input.get();
        }
    }
    input.unget();
    while number.count != 0 && number.digits[number.count - 1] == 0 { number.count -= 1; }
    if number.count == 0 { return Ok(Kind::Zero); }
    if exponent > 5200 { return Ok(Kind::Overflow); }
    if exponent < -5200 { return Ok(Kind::Underflow); }
    exponent = exponent.wrapping_add(integer_digits.wrapping_mul(if hex { 4 } else { 1 }));
    if exponent > 5200 { return Ok(Kind::Overflow); }
    if exponent < -5200 { return Ok(Kind::Underflow); }
    number.exponent = exponent;
    Ok(if hex { Kind::Hex } else { Kind::Decimal })
}

pub(super) fn read_with_context(
    text: &str,
    destination: &mut u32,
    context: Option<&Context<'_>>,
) -> Result<ReadOutcome, ReadRefusal> {
    // The reader accepts EOF and leaves its already-initialized destination untouched.
    if text.is_empty() || text.starts_with('\0') {
        return Ok(ReadOutcome::RetainedAtEof);
    }
    read_into(text, destination, context.ok_or(ReadRefusal::MissingContext)?)
}

#[cfg(test)]
pub(crate) fn radius_test_context() -> Context<'static> {
    let mut whitespace = [0; 4];
    for byte in [b' ', b'\t', b'\n', b'\r', 11, 12] {
        whitespace[usize::from(byte / 64)] |= 1u64 << (byte % 64);
    }
    Context { whitespace, decimal_point: b'.', rounding: RoundingMode::NearestEven, special_tokens: None }
}

pub(super) fn read_into(text: &str, destination: &mut u32, context: &Context<'_>) -> Result<ReadOutcome, ReadRefusal> {
    let bytes = text.chars().take_while(|c| *c != '\0')
        .map(|c| u8::try_from(u32::from(c)).map_err(|_| ReadRefusal::Encoding))
        .collect::<Result<Vec<_>, _>>()?;
    let mut input = Input { bytes: &bytes, position: 0, reads: 0, current: 0, valid: true };
    while input.bytes.get(input.position).is_some_and(|byte| context.space(*byte)) { input.position += 1; }
    let mut number = Number { negative: false, exponent: 0, digits: [0; 768], count: 0 };
    let kind = parse(&mut input, &mut number, context)?;
    if !input.valid || kind == Kind::Invalid {
        return if input.position == input.bytes.len() { Ok(ReadOutcome::RetainedAtEof) }
            else { Err(ReadRefusal::MatchingFailure) };
    }
    let sign = u32::from(number.negative) << 31;
    *destination = match kind {
        Kind::Zero | Kind::Underflow => sign,
        Kind::Infinity | Kind::Overflow => sign | 0x7f80_0000,
        Kind::QuietNan => sign | 0x7fff_ffff,
        Kind::SignalingNan => sign | 0x7f80_0001,
        Kind::Indeterminate => 0xffc0_0000,
        Kind::Decimal => decimal(&number, context.rounding)?,
        Kind::Hex => hexadecimal(&number, context.rounding),
        Kind::Invalid => unreachable!(),
    };
    Ok(ReadOutcome::Assigned)
}

fn round_right(value: u64, shift: u32, negative: bool, tail_zero: bool, mode: RoundingMode) -> u64 {
    if shift >= 64 { return 0; }
    let guard = 1u64 << (shift - 1);
    let sticky = !tail_zero || value & (guard - 1) != 0;
    let round = value & guard != 0;
    let increment = (round || sticky) && match mode {
        RoundingMode::NearestEven => round && (sticky || value & (guard << 1) != 0),
        RoundingMode::Down => negative,
        RoundingMode::Up => !negative,
        RoundingMode::TowardZero => false,
    };
    (value >> shift) + u64::from(increment)
}

fn pack(mut mantissa: u64, exponent: i32, negative: bool, tail_zero: bool, mode: RoundingMode) -> u32 {
    let sign = u32::from(negative) << 31;
    let shift = 24 - (64 - mantissa.leading_zeros()) as i32;
    let mut adjusted = exponent - shift;
    if adjusted > 127 { return sign | 0x7f80_0000; }
    if adjusted < -126 {
        let denormal_shift = exponent + 126;
        adjusted = -127;
        if denormal_shift < 0 {
            mantissa = round_right(mantissa, (-denormal_shift) as u32, negative, tail_zero, mode);
            if mantissa == 0 { return sign; }
            if mantissa > 0x7f_ffff { adjusted = exponent - denormal_shift - shift - 1; }
        } else { mantissa <<= denormal_shift; }
    } else if shift < 0 {
        mantissa = round_right(mantissa, (-shift) as u32, negative, tail_zero, mode);
        if mantissa > 0xff_ffff {
            mantissa >>= 1;
            adjusted += 1;
            if adjusted > 127 { return sign | 0x7f80_0000; }
        }
    } else if shift > 0 { mantissa <<= shift; }
    sign | (((adjusted + 127) as u32 * 0x80_0000) & 0x7f80_0000) | (mantissa as u32 & 0x7f_ffff)
}

fn hexadecimal(number: &Number, mode: RoundingMode) -> u32 {
    let mut mantissa = 0u64;
    let mut exponent = number.exponent + 23;
    let mut index = 0;
    while index < number.count && mantissa <= 0xff_ffff {
        mantissa = mantissa * 16 + u64::from(number.digits[index]);
        exponent -= 4;
        index += 1;
    }
    pack(mantissa, exponent, number.negative, number.digits[index..number.count].iter().all(|digit| *digit == 0), mode)
}

#[derive(Clone)]
struct Integer([u32; 115]);
impl Integer {
    fn zero() -> Self { Self([0; 115]) }
    fn one() -> Self { let mut value = Self::zero(); value.0[0] = 1; value }
    fn bits(&self) -> u32 {
        self.0.iter().rposition(|word| *word != 0)
            .map_or(0, |i| i as u32 * 32 + 32 - self.0[i].leading_zeros())
    }
    fn multiply_add(&mut self, multiplier: u32, addend: u32) -> Result<(), ReadRefusal> {
        let mut carry = u64::from(addend);
        for word in &mut self.0 {
            let value = u64::from(*word) * u64::from(multiplier) + carry;
            *word = value as u32;
            carry = value >> 32;
        }
        if carry != 0 { Err(ReadRefusal::IntegerCapacity) } else { Ok(()) }
    }
    fn shifted(&self, shift: u32) -> Result<Self, ReadRefusal> {
        if self.bits() == 0 { return Ok(Self::zero()); }
        if self.bits() + shift > 115 * 32 { return Err(ReadRefusal::IntegerCapacity); }
        let mut result = Self::zero();
        let words = (shift / 32) as usize;
        let bits = shift % 32;
        for index in 0..115 - words {
            result.0[index + words] |= self.0[index] << bits;
            if bits != 0 && index + words + 1 < 115 {
                result.0[index + words + 1] |= self.0[index] >> (32 - bits);
            }
        }
        Ok(result)
    }
    fn compare(&self, other: &Self) -> std::cmp::Ordering {
        self.0.iter().rev().cmp(other.0.iter().rev())
    }
    fn subtract(&mut self, other: &Self) {
        let mut borrow = 0u64;
        for (word, rhs) in self.0.iter_mut().zip(other.0) {
            let subtrahend = u64::from(rhs) + borrow;
            let lhs = u64::from(*word);
            borrow = u64::from(lhs < subtrahend);
            *word = (lhs.wrapping_sub(subtrahend)) as u32;
        }
        debug_assert_eq!(borrow, 0);
    }
}

fn decimal_digits(digits: &[u8]) -> Result<Integer, ReadRefusal> {
    let mut value = Integer::zero();
    for chunk in digits.chunks(9) {
        let mut word = 0;
        let mut multiplier = 1;
        for digit in chunk {
            word = word * 10 + u32::from(*digit);
            multiplier *= 10;
        }
        value.multiply_add(multiplier, word)?;
    }
    Ok(value)
}

fn multiply_decimal_power(value: &mut Integer, power: u32) -> Result<(), ReadRefusal> {
    for _ in 0..power { value.multiply_add(10, 0)?; }
    Ok(())
}

fn integer_pack(integer: &Integer, fractional_tail: bool, negative: bool, mode: RoundingMode) -> u32 {
    let discarded = integer.bits().saturating_sub(64);
    let mut top = 0u64;
    let mut tail_zero = !fractional_tail;
    for bit in 0..integer.bits() {
        let set = integer.0[(bit / 32) as usize] & (1u32 << (bit % 32)) != 0;
        if bit < discarded { tail_zero &= !set; }
        else if set { top |= 1u64 << (bit - discarded); }
    }
    pack(top, 23 + discarded as i32, negative, tail_zero, mode)
}

fn quotient(numerator: &mut Integer, denominator: &Integer) -> Result<(u64, bool), ReadRefusal> {
    // The native single-word divisor branch retains one remainder word even when its value is zero.
    let retained_zero_word = denominator.bits() <= 32 && denominator.0[0] != 1 && numerator.bits() > 32;
    let highest = numerator.bits().saturating_sub(denominator.bits());
    if highest >= 64 || denominator.bits() == 0 { return Err(ReadRefusal::IntegerCapacity); }
    let mut value = 0;
    for shift in (0..=highest).rev() {
        let candidate = denominator.shifted(shift)?;
        if numerator.compare(&candidate) != std::cmp::Ordering::Less {
            numerator.subtract(&candidate);
            value |= 1u64 << shift;
        }
    }
    Ok((value, !retained_zero_word && numerator.bits() == 0))
}

fn decimal(number: &Number, mode: RoundingMode) -> Result<u32, ReadRefusal> {
    let sign = u32::from(number.negative) << 31;
    let integer_count = number.count.min(number.exponent.max(0) as usize);
    let mut integer = decimal_digits(&number.digits[..integer_count])?;
    if multiply_decimal_power(&mut integer, number.exponent.max(0) as u32 - integer_count as u32).is_err() {
        return Ok(sign | 0x7f80_0000);
    }
    let integer_bits = integer.bits();
    let fractional_digits = &number.digits[integer_count..number.count];
    if integer_bits >= 25 || fractional_digits.is_empty() {
        return Ok(integer_pack(&integer, !fractional_digits.is_empty(), number.negative, mode));
    }
    let mut numerator = decimal_digits(fractional_digits)?;
    let mut denominator = Integer::one();
    let decimal_power = fractional_digits.len() as u32 + (-number.exponent).max(0) as u32;
    if multiply_decimal_power(&mut denominator, decimal_power).is_err() { return Ok(sign); }
    let mut scale = denominator.bits().saturating_sub(numerator.bits());
    numerator = numerator.shifted(scale)?;
    let remaining_bits = 25 - integer_bits;
    if integer_bits != 0 && remaining_bits < scale {
        return Ok(integer_pack(&integer, true, number.negative, mode));
    }
    let quotient_shift = if integer_bits == 0 { remaining_bits } else { remaining_bits - scale };
    if numerator.compare(&denominator) == std::cmp::Ordering::Less { scale += 1; }
    numerator = match numerator.shifted(quotient_shift) {
        Ok(shifted) => shifted,
        // Native quotient scaling clears the fixed-capacity numerator on overflow before division.
        Err(ReadRefusal::IntegerCapacity) => Integer::zero(),
        Err(error) => return Err(error),
    };
    let (mut mantissa, mut tail_zero) = quotient(&mut numerator, &denominator)?;
    let quotient_bits = 64 - mantissa.leading_zeros();
    if quotient_bits > remaining_bits {
        let shift = quotient_bits - remaining_bits;
        tail_zero &= mantissa & ((1u64 << shift) - 1) == 0;
        mantissa >>= shift;
    }
    let low_integer = u64::from(integer.0[0]) | (u64::from(integer.0[1]) << 32);
    let exponent = if integer_bits == 0 { -(scale as i32) - 1 } else { integer_bits as i32 - 2 };
    Ok(pack(mantissa + (low_integer << remaining_bits), exponent, number.negative, tail_zero, mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKENS: SpecialTokens = SpecialTokens {
        infinity: [*b"INF", *b"inf"], infinity_tail: [*b"INITY", *b"inity"],
        nan: [*b"NAN", *b"nan"], signaling_tail: [*b"SNAN)", *b"snan)"],
        indeterminate_tail: [*b"IND)", *b"ind)"],
    };
    fn context(rounding: RoundingMode, decimal_point: u8) -> Context<'static> {
        let mut whitespace = [0; 4];
        for byte in [b' ', b'\t', b'\n', b'\r', 11, 12] {
            whitespace[usize::from(byte / 64)] |= 1u64 << (byte % 64);
        }
        Context { whitespace, decimal_point, rounding, special_tokens: Some(&TOKENS) }
    }
    fn bits(text: &str, mode: RoundingMode) -> u32 {
        let mut output = 0xdead_beef;
        assert_eq!(read_into(text, &mut output, &context(mode, b'.')), Ok(ReadOutcome::Assigned), "{text:?}");
        output
    }

    #[test]
    fn unavailable_context_cannot_assign_but_cstring_eof_needs_no_context() {
        for initial in [0x3f80_0000, 0x43fa_0000, 0x8000_0000, 0x7f80_0001] {
            for text in ["", "\0not-read"] {
                let mut output = initial;
                assert_eq!(read_with_context(text, &mut output, None), Ok(ReadOutcome::RetainedAtEof));
                assert_eq!(output, initial);
            }
            for text in ["1", "500tail", " ", "inf"] {
                let mut output = initial;
                assert_eq!(read_with_context(text, &mut output, None), Err(ReadRefusal::MissingContext));
                assert_eq!(output, initial);
            }
        }
    }

    #[test]
    fn eof_keeps_the_callers_bits_including_after_a_consumed_incomplete_exponent() {
        for initial in [0x3f80_0000, 0x43fa_0000, 0x8000_0000, 0x7f80_0001] {
            for text in ["", " \t\n", "+", "-", ".", "-.", "1e", "1e+", "0x", "infi", "\0bad"] {
                let mut output = initial;
                assert_eq!(read_into(text, &mut output, &context(RoundingMode::NearestEven, b'.')),
                    Ok(ReadOutcome::RetainedAtEof), "{text:?}");
                assert_eq!(output, initial, "{text:?}");
            }
        }
        for text in ["bad", "+bad", "1ex", "1e+x", "0xz", ".x", "infiX", "\u{a0}1"] {
            let mut output = 0x43fa_0000;
            assert_eq!(read_into(text, &mut output, &context(RoundingMode::NearestEven, b'.')),
                Err(ReadRefusal::MatchingFailure), "{text:?}");
            assert_eq!(output, 0x43fa_0000);
        }
    }

    #[test]
    fn scalar_prefix_cstring_and_supplied_decimal_separator_drive_the_result() {
        for (text, expected) in [("500tail",0x43fa_0000), ("010",0x4120_0000), ("  +1.5tail",0x3fc0_0000),
            ("0x1.8p1tail",0x4040_0000), ("1\0e99",0x3f80_0000), ("-0tail",0x8000_0000)] {
            assert_eq!(bits(text, RoundingMode::NearestEven), expected, "{text}");
        }
        for (text, separator, expected) in [("1,5tail", b',', 0x3fc0_0000), ("1,5tail", b'.', 0x3f80_0000),
            ("1.5tail", b',', 0x3f80_0000), ("0x1,8p1tail", b',', 0x4040_0000)] {
            let mut output = 0;
            assert_eq!(read_into(text, &mut output, &context(RoundingMode::NearestEven, separator)), Ok(ReadOutcome::Assigned));
            assert_eq!(output, expected);
        }
    }

    #[test]
    fn decimal_rounding_uses_the_supplied_native_mode_and_sign() {
        for (mode, positive, negative, tie) in [
            (RoundingMode::NearestEven,0x3dcc_cccd,0xbdcc_cccd,0x4b80_0000),
            (RoundingMode::Down,0x3dcc_cccc,0xbdcc_cccd,0x4b80_0000),
            (RoundingMode::Up,0x3dcc_cccd,0xbdcc_cccc,0x4b80_0001),
            (RoundingMode::TowardZero,0x3dcc_cccc,0xbdcc_cccc,0x4b80_0000),
        ] {
            assert_eq!(bits("0.1", mode), positive);
            assert_eq!(bits("-0.1", mode), negative);
            assert_eq!(bits("16777217", mode), tie);
        }
        assert_eq!(bits("16777219", RoundingMode::NearestEven), 0x4b80_0002);
    }

    #[test]
    fn native_remainder_word_count_controls_rounding_even_when_remainder_value_is_zero() {
        let mut numerator = Integer::one().shifted(25).unwrap();
        numerator.multiply_add(1000, 0).unwrap();
        let mut denominator = Integer::one();
        denominator.multiply_add(1000, 0).unwrap();
        let (value, tail_zero) = quotient(&mut numerator, &denominator).unwrap();
        assert_eq!(value, 1 << 25);
        assert_eq!(numerator.bits(), 0);
        assert!(!tail_zero);
        assert_eq!(bits("0.125", RoundingMode::NearestEven), 0x3e00_0000);
        assert_eq!(bits("0.125", RoundingMode::Up), 0x3e00_0001);
        assert_eq!(bits("-0.125", RoundingMode::Down), 0xbe00_0001);
        assert_eq!(bits("0.5", RoundingMode::Up), 0x3f00_0000);
    }

    #[test]
    fn subnormal_and_overflow_outputs_are_not_filtered_or_replaced_by_defaults() {
        for mode in [RoundingMode::NearestEven, RoundingMode::Down, RoundingMode::Up, RoundingMode::TowardZero] {
            assert_eq!(bits("0x1p-149", mode), 1);
            assert_eq!(bits("0x1p-150", mode), u32::from(mode == RoundingMode::Up));
            assert_eq!(bits("-0x1p-150", mode), 0x8000_0000 | u32::from(mode == RoundingMode::Down));
            assert_eq!(bits("0x1p-213", mode), 0);
            assert_eq!(bits("1e5201", mode), 0x7f80_0000);
            assert_eq!(bits("-1e5201", mode), 0xff80_0000);
            assert_eq!(bits("1e-5201", mode), 0);
            assert_eq!(bits("-1e-5201", mode), 0x8000_0000);
            assert_eq!(bits("0x1.fffffep127", mode), 0x7f7f_ffff);
        }
        assert_eq!(bits("0x1.ffffffp127", RoundingMode::NearestEven), 0x7f80_0000);
        assert_eq!(bits("0x1.ffffffp127", RoundingMode::Down), 0x7f7f_ffff);
    }

    #[test]
    fn native_fraction_scaling_capacity_exhaustion_flows_through_zero_dividend_and_pack() {
        let mut denominator = Integer::one();
        multiply_decimal_power(&mut denominator, 1101).unwrap();
        let normalized = Integer::one().shifted(denominator.bits() - 1).unwrap();
        assert!(matches!(normalized.shifted(25), Err(ReadRefusal::IntegerCapacity)));
        for mode in [RoundingMode::NearestEven, RoundingMode::Down, RoundingMode::Up, RoundingMode::TowardZero] {
            for text in ["1e-1100", "1e-1101", "1e-1108"] {
                assert_eq!(bits(text, mode), 0, "{text}, {mode:?}");
                assert_eq!(bits(&format!("-{text}"), mode), 0x8000_0000, "-{text}, {mode:?}");
            }
        }
    }

    #[test]
    fn special_value_payloads_and_sequential_recognizers_retain_native_bits() {
        for (text, expected) in [("inf",0x7f80_0000), ("-INFINITYtail",0xff80_0000),
            ("nan",0x7fff_ffff), ("-nan(payload)",0xffff_ffff), ("nan(snan)",0x7f80_0001),
            ("-nan(snan)",0xff80_0001), ("nan(ind)",0xffc0_0000), ("nan(sind)",0xffc0_0000)] {
            assert_eq!(bits(text, RoundingMode::NearestEven), expected, "{text}");
        }
        let mut context = context(RoundingMode::NearestEven, b'.');
        context.special_tokens = None;
        let mut output = 0x3f80_0000;
        assert_eq!(read_into("inf", &mut output, &context), Err(ReadRefusal::MissingSpecialTokens));
        assert_eq!(output, 0x3f80_0000);
    }

    #[test]
    fn digit_capacity_matches_the_native_truncated_mantissa_not_the_full_scalar() {
        let text = format!("16777216.{}1", "0".repeat(760));
        assert_eq!(bits(&text, RoundingMode::Up), 0x4b80_0000);
        let retained = format!("16777216.{}1", "0".repeat(759));
        assert_eq!(bits(&retained, RoundingMode::Up), 0x4b80_0001);
        let below_fraction_window = format!("1.{}1", "0".repeat(766));
        assert_eq!(bits(&below_fraction_window, RoundingMode::Up), 0x3f80_0000);
    }
}
