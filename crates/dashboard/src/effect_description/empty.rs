#[cfg(test)]
mod tests;

// Native own text is empty (140176EE0 / 1402E9B40); setters never execute.
pub(super) fn describe() -> Result<String, String> {
    Ok(String::new())
}
