use super::{GameUiError, Parser, Result, Scalar};

/// Read CTextLexer token text and quote flags, without CReader assignments or variables.
/// Input is the Latin-1 view of the original bytes; this does not decode content.
/// Unlike `parse_str`, punctuation is content and a bare token ending directly at EOF
/// is not returned (1424a7a90 returns false after constructing that token).
pub fn parse_native_token_stream(text: &str) -> Result<Vec<Scalar>> {
    // Byte FF aliases the native EOF sentinel. Its interaction with stream pushback
    // is not closed; do not quietly reinterpret it as an authored character.
    if text.chars().any(|ch| ch >= '\u{ff}') {
        return Err(GameUiError::parse("native token stream byte FF or non-Latin-1 input is not closed"));
    }
    let mut parser = Parser::new(text);
    let mut tokens = Vec::new();
    while let Some((token, line, column)) = parser.next_native_scalar()? {
        tokens.try_reserve(1).map_err(|_| GameUiError::parse(format!(
            "native token stream {line}:{column} list allocation refused")))?;
        tokens.push(token);
    }
    Ok(tokens)
}

impl Parser<'_> {
    fn native_space(ch: char) -> bool {
        matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}')
    }

    fn native_push(&self, text: &mut String, ch: char) -> Result<()> {
        text.try_reserve(ch.len_utf8()).map_err(|_| GameUiError::parse(format!(
            "native token stream {}:{} allocation refused", self.line, self.column)))?;
        text.push(ch);
        Ok(())
    }

    fn next_native_scalar(&mut self) -> Result<Option<(Scalar, usize, usize)>> {
        loop {
            while self.peek().is_some_and(Self::native_space) { self.bump(); }
            let (line, column) = (self.line, self.column);
            let Some(first) = self.bump() else { return Ok(None); };
            match first {
                '!' | '#' | ';' => {
                    while let Some(ch) = self.bump() { if ch == '\n' { break; } }
                }
                '"' => return Ok(Some((Scalar::Quoted(self.read_native_quoted()?), line, column))),
                '(' | ')' | ',' | '<' | '=' | '>' | '{' | '}' => {
                    let mut token = String::new();
                    self.native_push(&mut token, first)?;
                    return Ok(Some((Scalar::Bare(token), line, column)));
                }
                '?' => {
                    if self.peek() == Some('=') {
                        self.bump();
                        return Ok(Some((Scalar::Bare("?=".into()), line, column)));
                    }
                    // The native diagnostic discards only this leading '?' and
                    // pushes its lookahead back before trying the next token.
                }
                _ => {
                    let mut token = String::new();
                    self.native_push(&mut token, first)?;
                    loop {
                        let Some(ch) = self.peek() else { return Ok(None); };
                        if Self::native_space(ch) || matches!(ch,
                            '!' | '"' | '#' | '(' | ')' | ',' | ';' | '<' | '=' | '>' | '{' | '}') {
                            return Ok(Some((Scalar::Bare(token), line, column)));
                        }
                        self.bump();
                        self.native_push(&mut token, ch)?;
                    }
                }
            }
        }
    }

    fn read_native_quoted(&mut self) -> Result<String> {
        let mut text = String::new();
        let mut previous_backslash = false;
        while let Some(ch) = self.bump() {
            if ch == '"' && !previous_backslash { break; }
            if previous_backslash && matches!(ch, '"' | '\\') {
                text.pop();
                previous_backslash = false;
            } else {
                // Other escapes are diagnosed but both bytes survive natively.
                previous_backslash = ch == '\\';
            }
            self.native_push(&mut text, ch)?;
        }
        // 1424a7a90 also returns quoted text successfully at unterminated EOF.
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(input: &str) -> Vec<String> {
        parse_native_token_stream(input).unwrap().into_iter().map(|token|
            match token { Scalar::Bare(text) | Scalar::Quoted(text) => text }).collect()
    }

    #[test]
    fn native_stream_registers_punctuation_and_does_not_parse_variable_assignments() {
        assert_eq!(text("@a = german @a @(german) {} < > , ?= ?GER\n"),
            ["@a", "=", "german", "@a", "@", "(", "german", ")", "{", "}", "<", ">", ",", "?=", "GER"]);
        assert_eq!(text("a! discarded\nb; discarded\nc# discarded\nd\n"), ["a", "b", "c", "d"]);
        assert_eq!(text("german\"French\""), ["german", "French"]);
        assert_eq!(text("a\u{b}b\u{c}c\n"), ["a", "b", "c"]);
    }

    #[test]
    fn native_stream_eof_success_depends_on_token_kind() {
        assert_eq!(text("first last"), ["first"]);
        assert_eq!(text("first last\n"), ["first", "last"]);
        assert_eq!(text("german"), Vec::<String>::new());
        assert_eq!(text("\"german\""), ["german"]);
        assert_eq!(text("\"unterminated"), ["unterminated"]);
        assert_eq!(text("{}"), ["{", "}"]);
        assert_eq!(text("?"), Vec::<String>::new());
    }

    #[test]
    fn native_stream_keeps_case_quote_flags_and_native_escape_bytes() {
        assert_eq!(parse_native_token_stream(r#"@x "@x" "@(" "GER" GER "" "#).unwrap(),
            [Scalar::Bare("@x".into()), Scalar::Quoted("@x".into()), Scalar::Quoted("@(".into()),
                Scalar::Quoted("GER".into()), Scalar::Bare("GER".into()), Scalar::Quoted("".into())]);
        assert_eq!(text(r#""a\nb\rc\td\qe\\f\"g""#), ["a\\nb\\rc\\td\\qe\\f\"g"]);
        assert_eq!(text("\"caf\u{e9}\""), ["caf\u{e9}"]);
        assert!(parse_native_token_stream("\u{ff}").unwrap_err().message.contains("byte FF"));
        assert!(parse_native_token_stream("\u{100}").is_err());
        // Opt-in behavior cannot change existing assignment-parser consumers.
        assert_eq!(super::super::parse_str("x = \"a\\nb\"").root.first_scalar_ci("x"), Some("a\nb"));
        assert_eq!(super::super::parse_str("german").root.fields().len(), 1);
    }
}
