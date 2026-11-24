use crate::hook::Hook;
use engine_sites::Rva;

pub(crate) fn site(family: &str, name: &str, index: usize, phase: &str, cause: &str) -> String {
    format!("{family}: {name} (site {index}) {phase}: {cause}")
}
pub(crate) fn locate(
    base: usize,
    key: Rva,
    family: &str,
    name: &str,
    index: usize,
) -> Result<(usize, [u8; 16]), String> {
    engine_sites::code(base, key)
        .and_then(|code| Ok((code.address, code.original::<16>()?)))
        .map_err(|withheld| {
            site(
                family,
                name,
                index,
                "site verification failed",
                &withheld.to_string(),
            )
        })
}
pub(crate) fn publication<'a>(
    family: &str,
    hooks: &[Hook],
    names: impl Iterator<Item = &'a str>,
    cause: &str,
) -> String {
    let sites: Vec<_> = names.enumerate().collect();
    let failed: Vec<_> = hooks
        .iter()
        .enumerate()
        .filter(|(_, hook)| !hook.matches_published_snapshot())
        .map(|(index, _)| index)
        .collect();
    let selected = sites
        .iter()
        .filter(|(index, _)| failed.is_empty() || failed.contains(index))
        .map(|(index, name)| format!("{name} (site {index})"))
        .collect::<Vec<_>>()
        .join(", ");
    let scope = if failed.is_empty() {
        "family transaction"
    } else {
        "snapshot mismatch"
    };
    format!("{family}: {scope} [{selected}] publication failed: {cause}")
}

