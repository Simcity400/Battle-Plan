//! D3DCompile at 1423FBC10 receives a null include handler. Only PdxShader `Includes` (already
//! resolved by installed-content) can load content. A compiler CLI must not search its host filesystem
//! for raw HLSL #include directives. Turn them into preprocessor errors, preserving #if branches.

pub fn native_includes(source: &str) -> String {
    // Line splicing precedes preprocessing/comment removal in HLSL. Splicing first also keeps
    // a split directive from taking a different path through an external compiler's includer.
    let source = source.replace("\\\r\n", "").replace("\\\n", "");
    let bytes = source.as_bytes();
    let mut visible = bytes.to_vec();
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        if let Some(delimiter) = quote {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if bytes[index] == delimiter {
                quote = None;
            }
        } else if matches!(bytes[index], b'"' | b'\'') {
            quote = Some(bytes[index]);
        } else if bytes[index..].starts_with(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                visible[index] = b' ';
                index += 1;
            }
            continue;
        } else if bytes[index..].starts_with(b"/*") {
            visible[index..index + 2].fill(b' ');
            index += 2;
            while index < bytes.len() {
                if bytes[index..].starts_with(b"*/") {
                    visible[index..index + 2].fill(b' ');
                    index += 2;
                    break;
                }
                if bytes[index] != b'\n' {
                    visible[index] = b' ';
                }
                index += 1;
            }
            continue;
        }
        index += 1;
    }
    let mut replacements = Vec::new();
    let mut offset = 0;
    for line in visible.split_inclusive(|byte| *byte == b'\n') {
        let first = line.iter().position(|byte| !byte.is_ascii_whitespace());
        if let Some(first) = first.filter(|first| line[*first] == b'#') {
            let tail = &line[first + 1..];
            if let Some(word) = tail.iter().position(|byte| !byte.is_ascii_whitespace()) {
                let word = first + 1 + word;
                let end = line[word..]
                    .iter()
                    .position(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
                    .map_or(line.len(), |length| word + length);
                if &line[word..end] == b"include" {
                    replacements.push(offset + word..offset + end);
                }
            }
        }
        offset += line.len();
    }
    let mut out = source;
    for range in replacements.into_iter().rev() {
        out.replace_range(range, "error Native_D3DCompile_has_no_HLSL_include_handler");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_includes_become_conditional_compiler_errors() {
        let source = "/* #include \"comment\" */\n#if 0\n# /*comment*/ include \"inactive\"\n#endif\n#inc\\\nlude \"active\"\n// #include \"comment\"\n";
        let lowered = native_includes(source);
        assert_eq!(lowered.matches("#error Native_").count(), 1);
        assert!(lowered.contains("# /*comment*/ error Native_"));
        assert!(lowered.contains("/* #include \"comment\" */"));
        assert!(lowered.contains("// #include \"comment\""));
        assert!(lowered.contains("#if 0\n"));
    }
}
