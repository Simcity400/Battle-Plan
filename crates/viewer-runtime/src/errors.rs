use std::io;

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn examples(values: &[String]) -> String {
    let mut sample = values
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if values.len() > 8 {
        sample.push_str(&format!(", and {} more", values.len() - 8));
    }
    sample
}
