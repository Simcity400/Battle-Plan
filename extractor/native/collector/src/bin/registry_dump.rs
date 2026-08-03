use std::io::{self, Write};

use compact_contract::{is_descheduled_from_capture, ASSIGNED_COMPACT_DOMAINS};

fn write_json_string(output: &mut impl Write, value: &str) -> io::Result<()> {
    output.write_all(b"\"")?;
    for ch in value.chars() {
        match ch {
            '"' => output.write_all(b"\\\"")?,
            '\\' => output.write_all(b"\\\\")?,
            '\u{08}' => output.write_all(b"\\b")?,
            '\u{0c}' => output.write_all(b"\\f")?,
            '\n' => output.write_all(b"\\n")?,
            '\r' => output.write_all(b"\\r")?,
            '\t' => output.write_all(b"\\t")?,
            ch if ch <= '\u{1f}' => write!(output, "\\u{:04x}", ch as u32)?,
            ch => write!(output, "{ch}")?,
        }
    }
    output.write_all(b"\"")
}

fn main() -> io::Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();

    for descriptor in ASSIGNED_COMPACT_DOMAINS {
        write!(output, "{{\"id\":{},\"wire_name\":", descriptor.id)?;
        write_json_string(&mut output, descriptor.wire_name)?;
        output.write_all(b",\"internal_name\":")?;
        write_json_string(&mut output, descriptor.canonical_name)?;
        output.write_all(b",\"status\":")?;
        write_json_string(&mut output, &format!("{:?}", descriptor.status))?;
        // `capture_scheduled` is NOT a descriptor field: descheduling leaves
        // status and `schedulable` alone, so a consumer cannot derive it from
        // the rest of this row. Emitting it here is what lets the C0 census
        // compute profile membership from compiled truth instead of trusting
        // the JSON mirror, and lets it cross-check the two.
        write!(
            output,
            ",\"schedulable\":{},\"capture_scheduled\":{},\"requires_data_record\":{},\"tags\":[",
            descriptor.schedulable,
            !is_descheduled_from_capture(descriptor.id),
            descriptor.requires_data_record
        )?;
        for (index, tag) in descriptor.aliases.iter().enumerate() {
            if index != 0 {
                output.write_all(b",")?;
            }
            write_json_string(&mut output, tag)?;
        }
        output.write_all(b"]}\n")?;
    }
    Ok(())
}
