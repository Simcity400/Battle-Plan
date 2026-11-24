
const PROLOGUE: &str = "#undef __has_include\n#undef __has_include_next\n#line 1\n";

const REFUSED: &str = "Raw_HLSL_includes_are_not_supported";

pub fn without_host_files(source: &str) -> Result<String, String> {
    if source.contains('\0') {
        return Err(
            "raw HLSL holding NUL is refused: DXC reads where its comments end differently"
                .into(),
        );
    }
    let (view, origin) = spliced(source);
    let text = Text {
        view: &view,
        origin: &origin,
        source: source.as_bytes(),
    };
    let bytes = view.as_bytes();
    let mut replacements = Vec::new();
    let mut index = 0;
    let mut line_start = true;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' | b'\r' => {
                line_start = true;
                index += 1;
            }
            b'#' if line_start && bytes.get(index + 1) != Some(&b'#') => {
                line_start = false;
                index = text.directive(index + 1, &mut replacements)?;
            }
            _ => {
                if let Some(after) = text.blank(index) {
                    index = after;
                    continue;
                }
                line_start = false;
                index = match bytes[index] {
                    quote @ (b'"' | b'\'') => literal(bytes, index + 1, quote),
                    _ => index + 1,
                };
            }
        }
    }
    let mut out = String::with_capacity(PROLOGUE.len() + source.len() + 32 * replacements.len());
    out.push_str(PROLOGUE);
    let mut copied = 0;
    for range in replacements {
        let (start, end) = (origin[range.start], origin[range.end - 1] + 1);
        out.push_str(&source[copied..start]);
        out.push_str(REFUSED);
        copied = end;
    }
    out.push_str(&source[copied..]);
    Ok(out)
}

fn splice(bytes: &[u8], index: usize) -> Option<usize> {
    if bytes.get(index) != Some(&b'\\') {
        return None;
    }
    let mut end = index + 1;
    while end < bytes.len() && matches!(bytes[end], b' ' | b'\t' | b'\x0b' | b'\x0c') {
        end += 1;
    }
    let first = *bytes.get(end).filter(|byte| matches!(byte, b'\n' | b'\r'))?;
    end += 1;
    if bytes
        .get(end)
        .is_some_and(|&second| matches!(second, b'\n' | b'\r') && second != first)
    {
        end += 1;
    }
    Some(end - index)
}

fn spliced(source: &str) -> (String, Vec<usize>) {
    let bytes = source.as_bytes();
    let mut view = Vec::with_capacity(bytes.len());
    let mut origin = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some(length) = splice(bytes, index) {
            index += length;
            continue;
        }
        view.push(bytes[index]);
        origin.push(index);
        index += 1;
    }
    let view = String::from_utf8(view).expect("removing ASCII bytes keeps UTF-8 whole");
    (view, origin)
}

struct Text<'a> {
    view: &'a str,
    origin: &'a [usize],
    source: &'a [u8],
}

impl Text<'_> {
    fn blank(&self, index: usize) -> Option<usize> {
        let bytes = self.view.as_bytes();
        match bytes[index] {
            b' ' | b'\t' | b'\x0b' | b'\x0c' => Some(index + 1),
            b'/' if bytes.get(index + 1) == Some(&b'/') => Some(line_end(bytes, index + 2)),
            b'/' if bytes.get(index + 1) == Some(&b'*') => Some(self.comment_end(index + 2)),
            b'\\' => {
                let digits = match bytes.get(index + 1) {
                    Some(b'u') => 4,
                    Some(b'U') => 8,
                    _ => return None,
                };
                let hex = bytes.get(index + 2..index + 2 + digits)?;
                hex.iter()
                    .all(u8::is_ascii_hexdigit)
                    .then_some(index + 2 + digits)
            }
            byte if !byte.is_ascii() => {
                Some(index + self.view[index..].chars().next().map_or(1, char::len_utf8))
            }
            _ => None,
        }
    }

    fn comment_end(&self, index: usize) -> usize {
        let bytes = self.view.as_bytes();
        let mut at = index;
        while let Some(found) = bytes[at..].windows(2).position(|pair| pair == b"*/") {
            let star = at + found;
            let gap = &self.source[self.origin[star] + 1..self.origin[star + 1]];
            if gap.is_empty() || splice(gap, 0) == Some(gap.len()) {
                return star + 2;
            }
            at = star + 1;
        }
        bytes.len()
    }

    fn comment_past_line(&self, mut index: usize) -> bool {
        let bytes = self.view.as_bytes();
        while index < bytes.len() {
            match bytes[index] {
                b'\n' | b'\r' => return false,
                quote @ (b'"' | b'\'') => index = literal(bytes, index + 1, quote),
                b'/' if bytes.get(index + 1) == Some(&b'/') => return false,
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    let end = self.comment_end(index + 2);
                    if bytes[index + 2..end]
                        .iter()
                        .any(|&byte| matches!(byte, b'\n' | b'\r'))
                    {
                        return true;
                    }
                    index = end;
                }
                _ => index += 1,
            }
        }
        false
    }

    fn directive(
        &self,
        index: usize,
        replacements: &mut Vec<std::ops::Range<usize>>,
    ) -> Result<usize, String> {
        let bytes = self.view.as_bytes();
        let skip = |mut index: usize| {
            while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
                match self.blank(index) {
                    Some(after) => index = after,
                    None => break,
                }
            }
            index
        };
        let word = |start: usize| {
            let end = bytes[start..]
                .iter()
                .position(|&byte| !(byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'))
                .map_or(bytes.len(), |length| start + length);
            (&bytes[start..end], end)
        };
        let raw = |name: &str, index: usize| {
            if self.comment_past_line(index) {
                return Err(format!(
                    "a #{name} line opens a block comment past it, which DXC reads or not as the \
                     line is active or left out by #if, so the raw HLSL's includes cannot be found"
                ));
            }
            Ok(line_end(bytes, index))
        };
        let start = skip(index);
        let (name, end) = word(start);
        match name {
            b"include" => {
                replacements.push(start..end);
                Ok(end)
            }
            b"error" => raw("error", end),
            b"warning" => raw("warning", end),
            b"pragma" => {
                let at = skip(end);
                match word(at) {
                    (b"mark", after) => raw("pragma mark", after),
                    _ => Ok(at),
                }
            }
            _ => Ok(end),
        }
    }
}

fn line_end(bytes: &[u8], index: usize) -> usize {
    bytes[index..]
        .iter()
        .position(|&byte| matches!(byte, b'\n' | b'\r'))
        .map_or(bytes.len(), |end| index + end)
}

fn literal(bytes: &[u8], mut index: usize, quote: u8) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if matches!(bytes.get(index + 1), Some(b'\n' | b'\r')) => index += 1,
            b'\\' => index += 2,
            b'\n' | b'\r' => return index,
            byte if byte == quote => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lowered(source: &str) -> String {
        without_host_files(source).unwrap()
    }

    fn refused(source: &str) -> usize {
        lowered(source).matches(REFUSED).count()
    }

    #[test]
    fn host_includes_become_conditional_compiler_errors() {
        let source = "/* #include \"comment\" */\n#if 0\n# /*comment*/ include \"inactive\"\n#endif\n#inc\\\nlude \"active\"\n// #include \"comment\"\n";
        let lowered = lowered(source);
        assert_eq!(
            lowered
                .matches("#Raw_HLSL_includes_are_not_supported \"active\"")
                .count(),
            1
        );
        assert!(lowered.contains("# /*comment*/ Raw_HLSL_includes_are_not_supported"));
        assert!(lowered.contains("/* #include \"comment\" */"));
        assert!(lowered.contains("// #include \"comment\""));
        assert!(lowered.contains("#if 0\n"));
        assert!(lowered.starts_with(PROLOGUE));
    }

    #[test]
    fn every_newline_the_compiler_takes_ends_a_line() {
        for newline in ["\r", "\n", "\r\n", "\n\r"] {
            let source = format!("float x; // comment{newline}#include \"C:/host.h\"{newline}");
            assert_eq!(refused(&source), 1, "{newline:?}");
        }
        assert_eq!(refused("#if 0\r#include \"a\"\r#endif\r#include \"b\"\r"), 2);
    }

    #[test]
    fn splices_follow_the_compilers_escaped_newlines() {
        for splice in ["\\\n", "\\\r", "\\\r\n", "\\\n\r", "\\ \t\x0b\x0c\n"] {
            assert_eq!(refused(&format!("#inc{splice}lude \"a\"\n")), 1, "{splice:?}");
            assert_eq!(
                refused(&format!("// note{splice}#include \"a\"\n")),
                0,
                "{splice:?}"
            );
        }
        assert_eq!(refused("float x; \\\n#include \"a\"\n"), 0);
    }

    #[test]
    fn the_compiler_receives_the_programs_own_splices() {
        let source = "#inc\\\\\n\nlude \"C:/host.h\"\n";
        assert_eq!(lowered(source), format!("{PROLOGUE}{source}"));
    }

    #[test]
    fn whatever_the_compiler_skips_cannot_hide_a_directive() {
        for gap in [
            "\x0b",
            "\x0c",
            "\u{a0}",
            "\u{3000}",
            "\\u00A0",
            "\\U000000a0",
            "/*\n*/",
            "/*\r*/",
        ] {
            assert_eq!(refused(&format!("{gap}#include \"a\"\n")), 1, "{gap:?} before");
            assert_eq!(refused(&format!("#{gap}include \"a\"\n")), 1, "{gap:?} after");
        }
        for other in [
            "x #include \"a\"\n",
            "#include_next \"a\"\n",
            "#include$ \"a\"\n",
            "##include \"a\"\n",
        ] {
            assert_eq!(refused(other), 0, "{other:?}");
        }
    }

    #[test]
    fn raw_hlsl_holding_nul_is_refused() {
        let source = "/* *\\\0\n/\n#include \"C:/host.h\"\n// */\n";
        assert!(without_host_files(source).is_err());
        assert!(without_host_files("float x;\0\n").is_err());
    }

    #[test]
    fn a_comment_ends_where_dxc_ends_it() {
        assert_eq!(refused("/* x *\\\n/ #include \"a\"\n"), 1);
        assert_eq!(refused("/* x *\\ \r\n/ #include \"a\"\n"), 1);
        assert_eq!(refused("/* x *\\\n\\\n/ // */ #include \"C:/host.h\"\n"), 1);
        assert_eq!(refused("/* x *\\\n\\\n/ #include \"a\"\n*/\n"), 0);
    }

    #[test]
    fn a_refused_include_line_reads_as_tokens() {
        assert_eq!(refused("#include \"a\" /*\n#include \"b\"\n*/\n"), 1);
    }

    #[test]
    fn a_line_dxc_may_read_raw_or_as_tokens_is_refused_when_the_readings_differ() {
        for line in [
            "#error x /*",
            "#warning x /*",
            "#pragma mark x /*",
            "# pragma /**/ mark x /*",
            "#error x \" /* \" /*",
        ] {
            let source = format!("{line}\n#include \"b\"\n*/\n");
            assert!(without_host_files(&source).is_err(), "{line}");
        }
        let skipped = "#if 0\n#warning /*\n\" */ \" /*\n#endif\n#include \"C:/host.h\"\n// */\n";
        assert!(without_host_files(skipped).is_err());
        for line in ["#error x /* closed */", "#warning \"/*\" it's", "#pragma mark // x"] {
            let source = format!("{line}\n#include \"b\"\n");
            assert_eq!(refused(&source), 1, "{line}");
        }
        assert_eq!(refused("#define X /*\n#include \"a\"\n*/\n"), 0);
    }

    #[test]
    fn literals_end_where_the_compiler_ends_them() {
        let source = "#if 0\n\"\n#endif\n\"/*\"\n#include \"a\"\n// */\n";
        assert_eq!(refused(source), 1);
        assert_eq!(refused("float x = '\\''; /* \n#include \"a\"\n */\n"), 0);
        assert_eq!(refused("#define S \"/*\"\n#include \"a\"\n"), 1);
        assert_eq!(refused("float s = \"a\\\\\n\n#include \"b\"\n"), 1);
    }

}
