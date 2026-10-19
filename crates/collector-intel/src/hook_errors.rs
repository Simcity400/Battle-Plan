//! Attach/detach diagnostics, outside native flights and suspended publication.
use crate::hook::Hook;
use engine_native::{Memory, read};

// The host's failure API retains a static reason. Allocate only on the failing
// attach/detach path so its diagnostic survives that family's destruction.
pub(crate) fn site(
    family: &str,
    name: &str,
    index: usize,
    phase: &str,
    cause: &str,
) -> &'static str {
    Box::leak(format!("{family}: {name} (site {index}) {phase}: {cause}").into_boxed_str())
}
pub(crate) fn verify(
    memory: &impl Memory,
    base: usize,
    rva: usize,
    sentinel: [u8; 16],
    family: &str,
    name: &str,
    index: usize,
) -> Result<(), &'static str> {
    let observed = base
        .checked_add(rva)
        .and_then(|at| read::<[u8; 16]>(memory, at));
    match observed {
        Some(bytes) if bytes == sentinel => Ok(()),
        Some(_) => Err(site(
            family,
            name,
            index,
            "sentinel verification failed",
            "native bytes differ from verified snapshot",
        )),
        None => Err(site(
            family,
            name,
            index,
            "sentinel verification failed",
            "native bytes unreadable or address overflow",
        )),
    }
}
pub(crate) fn publication<'a>(
    family: &str,
    hooks: &[Hook],
    names: impl Iterator<Item = &'a str>,
    cause: &str,
) -> &'static str {
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
    // Cache flush, thread suspension and protection failures can affect a whole
    // transaction. Name its actual member sites instead of inventing one culprit.
    let scope = if failed.is_empty() {
        "family transaction"
    } else {
        "snapshot mismatch"
    };
    Box::leak(
        format!("{family}: {scope} [{selected}] publication failed: {cause}").into_boxed_str(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Bytes(Option<[u8; 16]>);
    impl Memory for Bytes {
        fn copy(&self, _: usize, out: &mut [u8]) -> bool {
            if let Some(bytes) = self.0 {
                out.copy_from_slice(&bytes);
                true
            } else {
                false
            }
        }
    }
    #[test]
    fn installer_failures_keep_family_site_and_underlying_cause() {
        for family in ["intel sources", "network auxiliary", "propaganda"] {
            assert_eq!(
                verify(&Bytes(Some([7; 16])), 4096, 0, [7; 16], family, "clear", 23),
                Ok(())
            );
            for memory in [Bytes(None), Bytes(Some([6; 16]))] {
                let reason = verify(&memory, 4096, 0, [7; 16], family, "clear", 23).unwrap_err();
                assert!(
                    reason.contains(family)
                        && reason.contains("clear (site 23)")
                        && reason.contains("sentinel verification failed")
                );
                assert!(reason.contains("unreadable") || reason.contains("bytes differ"));
            }
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
