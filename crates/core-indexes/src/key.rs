use std::io::{self, Write};

pub fn write_integer(mut value: u64, out: &mut impl Write) -> io::Result<()> {
    let mut groups = [0; 10];
    let mut len = 0;
    loop {
        groups[len] = (value & 127) as u8;
        len += 1;
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    write_wide_integer(groups[..len].iter().rev().copied(), out)
}

pub fn write_wide_integer(
    mut groups: impl ExactSizeIterator<Item = u8>,
    out: &mut impl Write,
) -> io::Result<()> {
    let len = groups.len();
    let first = groups
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty integer"))?;
    if first > 127 || (len > 1 && first == 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "noncanonical integer",
        ));
    }
    if len > 1 {
        let mut extra = len - 2;
        while extra >= 127 {
            out.write_all(&[255])?;
            extra -= 127;
        }
        out.write_all(&[128 + extra as u8])?;
    }
    out.write_all(&[first])?;
    for group in groups {
        if group > 127 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "base-128 digit",
            ));
        }
        out.write_all(&[group])?;
    }
    Ok(())
}

pub fn integer(value: u64, output: &mut [u8; 21]) -> &[u8] {
    let mut target = &mut output[..];
    write_integer(value, &mut target).expect("u64 varint fits");
    let len = 21 - target.len();
    &output[..len]
}
