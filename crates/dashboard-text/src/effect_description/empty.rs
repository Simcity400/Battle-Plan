#[cfg(test)]
mod tests;

// Native own text is empty; setters never execute.
pub(super) fn describe() -> Result<String, String> {
    Ok(String::new())
}
