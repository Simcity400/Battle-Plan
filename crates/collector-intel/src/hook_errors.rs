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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installer_failures_keep_family_site_and_underlying_cause() {
        for family in ["intel sources", "network auxiliary", "propaganda"] {
            let reason = locate(4096, Rva(0), family, "clear", 23).unwrap_err();
            assert!(
                reason.contains(family)
                    && reason.contains("clear (site 23)")
                    && reason.contains("site verification failed")
            );
            let reason = site(
                family,
                "getter",
                39,
                "construction failed",
                "relocation cause",
            );
            assert!(reason.contains("getter (site 39)") && reason.contains("relocation cause"));
            let reason = publication(
                family,
                &[],
                ["getter", "clear"].into_iter(),
                "cache publication cause",
            );
            assert!(
                reason.contains("getter (site 0)")
                    && reason.contains("clear (site 1)")
                    && reason.contains("cache publication cause")
            );
        }
    }
}
