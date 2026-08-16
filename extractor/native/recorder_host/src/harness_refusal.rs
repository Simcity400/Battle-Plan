//! What the SUPERVISOR refused, read by the process it was supervising.
//!
//! The recorder and its harness are separate processes that share one run
//! directory and, until now, no judgement. The harness validates the recorder's
//! `CAPTURE_READY_V2.json` against the collector's own producer source and, when
//! that fails, writes `CAPTURE_REFUSAL_V1.json` on the spot and stops the
//! attempt. The recorder never hears about it: it keeps capturing for its
//! configured duration, tears down, and then folds.
//!
//! On capture t60 (2026-08-16) that cost 49.6 minutes and several gigabytes.
//! READY custody was never established -- the receipt's `ready_payload_sha256`
//! did not match -- so nothing the stream contains is attributable to a
//! custodied capture, and the fold was building a sealed replay out of frames no
//! one could vouch for. It filled the system disk doing it.
//!
//! # What this is NOT
//!
//! It is not the recorder taking orders from a file. The refusal is read as
//! EVIDENCE about this run and admitted only when it names this run: same
//! schema, same session, same output directory. A receipt left in a reused
//! directory by an earlier attempt names a different session and is ignored --
//! the same lesson the harness learned about the teardown receipt, applied in
//! the other direction.
//!
//! It is also not a destruction. The spool stays exactly where it is and stays
//! foldable offline by the repack command; what is skipped is the automatic
//! fold, not the data. "Partial data still seals" governs what a recording
//! ACCOUNTS for; a READY that never established custody has nothing to account
//! for yet, which is a different statement from "some of its hours are injured".

use serde::Deserialize;
use std::path::Path;

/// The file the harness writes at the refusal site, before it rethrows.
pub const CAPTURE_REFUSAL_FILE: &str = "CAPTURE_REFUSAL_V1.json";
pub const CAPTURE_REFUSAL_SCHEMA: &str = "hoi4-capture-refusal-v1";

/// Bytes of a refusal receipt this reader will admit at all.
///
/// The real ones are a couple of kilobytes; the bound exists so a corrupt or
/// hostile file cannot be read into memory before it is judged.
const MAX_REFUSAL_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct RefusalReceipt {
    schema: Option<String>,
    session_id: Option<String>,
    out_dir: Option<String>,
    // NOT `#[serde(default)]`. An absent list is a gap in the document, and
    // defaulting it to empty spelled that gap as "nothing was refused".
    capture_refusals: Option<Vec<RefusalRecord>>,
}

#[derive(Debug, Deserialize)]
struct RefusalRecord {
    stage: Option<String>,
    class: Option<String>,
    custody_established: Option<bool>,
    message: Option<String>,
}

/// What the supervisor's receipt says about this run.
///
/// THREE VALUES, NOT TWO, and the third is the point. A missing receipt and a
/// receipt that cannot be judged are opposite facts: the first says there was
/// nothing to refuse, the second says something was written here and this
/// process cannot tell what. Collapsing them into one "no refusal" is the
/// original defect in a new place -- it would fold a refused capture because the
/// refusal was truncated, over-long, or written in bytes this reader cannot
/// parse (Codex review round 3, finding E2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefusalVerdict {
    /// No receipt, or one that positively does not concern this run's custody:
    /// a foreign schema, another session, another directory, or refusals that
    /// all kept custody. Fold exactly as before.
    NotRefused,
    /// A refusal naming this run, whose custody was never established.
    Refused(CustodyRefusal),
    /// A receipt exists for this directory and could not be judged. Fail closed.
    Unreadable(String),
}

/// A refusal that names THIS run and cost it its custody.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyRefusal {
    pub stage: String,
    pub class: String,
    pub message: String,
}

impl CustodyRefusal {
    pub fn reason(&self) -> String {
        format!(
            "the supervising harness refused this capture at stage '{}' (class '{}') and READY custody was \
             never established, so the recorder did not fold the stream into a sealed replay: {}",
            self.stage, self.class, self.message
        )
    }
}

impl RefusalVerdict {
    /// The sentence a receipt and a manifest should carry, or `None` to fold.
    pub fn skip_reason(&self) -> Option<String> {
        match self {
            RefusalVerdict::NotRefused => None,
            RefusalVerdict::Refused(refusal) => Some(refusal.reason()),
            RefusalVerdict::Unreadable(why) => Some(format!(
                "a supervisor refusal receipt exists in this run directory and could not be judged \
                 ({why}), so the recorder did not fold the stream into a sealed replay. A receipt \
                 this process cannot read is not evidence that nothing was refused."
            )),
        }
    }
}

/// Read the harness's refusal receipt and say what it means for this run.
pub fn custody_refusal(out_dir: &Path, session_id: &str) -> RefusalVerdict {
    let path = out_dir.join(CAPTURE_REFUSAL_FILE);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        // ABSENCE, and only absence, is "no supervisor said anything". Anything
        // else about the file -- a permission error, a directory in its place --
        // is a fact this process cannot resolve.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return RefusalVerdict::NotRefused
        }
        Err(error) => {
            return RefusalVerdict::Unreadable(format!("its metadata could not be read: {error}"))
        }
    };
    if !metadata.is_file() {
        return RefusalVerdict::Unreadable(format!(
            "{} is not a regular file",
            path.display()
        ));
    }
    if metadata.len() > MAX_REFUSAL_BYTES {
        return RefusalVerdict::Unreadable(format!(
            "it is {} bytes, past the {MAX_REFUSAL_BYTES}-byte bound this reader will admit",
            metadata.len()
        ));
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => return RefusalVerdict::Unreadable(format!("it could not be read: {error}")),
    };
    // The harness writes UTF-8 with a BOM (`Set-Content -Encoding UTF8` on
    // Windows PowerShell). serde_json will not parse past one, and a receipt
    // that fails to parse would silently read as "nothing was refused" -- which
    // is the exact failure this module exists to end.
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return RefusalVerdict::Unreadable("it is not UTF-8".to_string());
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let receipt: RefusalReceipt = match serde_json::from_str(text) {
        Ok(receipt) => receipt,
        Err(error) => {
            return RefusalVerdict::Unreadable(format!("it does not parse as JSON: {error}"))
        }
    };
    // A RECEIPT UNDER THIS NAME MUST BE COMPLETE OR IT IS UNJUDGEABLE. Only a
    // POSITIVE, well-formed statement that this receipt is about something else
    // lets the fold proceed. A truncated document that still happens to parse --
    // no schema, no session, no refusal list -- was previously indistinguishable
    // from "nothing was refused", which is the fail-open hole one level in
    // (Codex review round 4, finding R4.2).
    let Some(schema) = receipt.schema.as_deref() else {
        return RefusalVerdict::Unreadable("it names no schema".to_string());
    };
    if schema != CAPTURE_REFUSAL_SCHEMA {
        // A DIFFERENT, NAMED format. That is a fact, not a gap: some other
        // producer owns this filename here, and it is not making a claim about
        // this run's custody.
        return RefusalVerdict::NotRefused;
    }
    let Some(receipt_session) = receipt.session_id.as_deref() else {
        return RefusalVerdict::Unreadable(
            "it carries this schema but names no session".to_string(),
        );
    };
    if receipt_session != session_id {
        return RefusalVerdict::NotRefused;
    }
    // The receipt names the directory it was written for. Comparing it against
    // the directory we are finalizing catches a receipt copied or inherited
    // between run directories, which a session id alone would not. It is
    // REQUIRED, not optional: a receipt for our session that will not say which
    // directory it describes is incomplete, and admitting it on the strength of
    // the session id alone was the same gap in a third place.
    let Some(stated_out_dir) = receipt.out_dir.as_deref() else {
        return RefusalVerdict::Unreadable(
            "it carries this schema and session but names no output directory".to_string(),
        );
    };
    if !out_dir_matches(stated_out_dir, out_dir) {
        return RefusalVerdict::NotRefused;
    }
    let Some(records) = receipt.capture_refusals else {
        return RefusalVerdict::Unreadable(
            "it names this run but carries no capture_refusals list at all".to_string(),
        );
    };
    // A RECORD THAT WILL NOT SAY WHETHER CUSTODY SURVIVED decides nothing and
    // must not be read as "custody survived".
    if let Some(record) = records
        .iter()
        .find(|record| record.custody_established.is_none())
    {
        return RefusalVerdict::Unreadable(format!(
            "one of its refusal records (stage {:?}) does not say whether custody was established",
            record.stage.as_deref().unwrap_or("unnamed")
        ));
    }
    match records
        .into_iter()
        .find(|record| record.custody_established == Some(false))
    {
        Some(record) => RefusalVerdict::Refused(CustodyRefusal {
            stage: record.stage.unwrap_or_else(|| "unnamed_stage".to_string()),
            class: record.class.unwrap_or_else(|| "unnamed_class".to_string()),
            message: record
                .message
                .unwrap_or_else(|| "the refusal carried no message".to_string()),
        }),
        None => RefusalVerdict::NotRefused,
    }
}

/// Compare the receipt's stated directory against the one being finalized.
///
/// Both sides are canonicalized when the filesystem allows it, so a run reached
/// through a different spelling of the same directory still matches. A receipt
/// with no `out_dir` at all is admitted: it is an older or partial shape, and
/// the session id already pinned the run.
fn out_dir_matches(stated: &str, out_dir: &Path) -> bool {
    let stated = Path::new(stated);
    match (
        std::fs::canonicalize(stated),
        std::fs::canonicalize(out_dir),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => paths_equal_ignoring_case(stated, out_dir),
    }
}

fn paths_equal_ignoring_case(left: &Path, right: &Path) -> bool {
    let normalize = |path: &Path| {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    normalize(left) == normalize(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, body: &str) {
        std::fs::write(dir.join(CAPTURE_REFUSAL_FILE), body).unwrap();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ll_harness_refusal_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn receipt(session: &str, dir: &Path, custody: &str) -> String {
        format!(
            r#"{{"schema":"hoi4-capture-refusal-v1","out_dir":{},"session_id":"{session}",
                "capture_refusals":[{{"schema":"hoi4-capture-refusal-record-v1","stage":"ready_validation",
                "class":"receipt_admission","custody_established":{custody},"message":"payload sha mismatch"}}],
                "supervision_failures":[]}}"#,
            serde_json::to_string(&dir.to_string_lossy().to_string()).unwrap()
        )
    }

    /// The case that cost t60 fifty minutes: a refusal that named this run and
    /// took its custody must be visible to the process about to fold.
    ///
    /// kills: reading the file but not matching the schema; ignoring
    /// `custody_established`; returning the first refusal regardless of custody.
    #[test]
    fn a_custody_losing_refusal_for_this_session_is_reported() {
        let dir = temp_dir("hit");
        write(&dir, &receipt("s-1", &dir, "false"));
        let RefusalVerdict::Refused(found) = custody_refusal(&dir, "s-1") else {
            panic!("the refusal must be read");
        };
        assert_eq!(found.stage, "ready_validation");
        assert_eq!(found.class, "receipt_admission");
        assert!(found.reason().contains("READY custody was never established"));
    }

    /// A receipt left behind by an EARLIER attempt in a reused run directory
    /// proves nothing about this one, and acting on it would skip the fold of a
    /// perfectly good capture.
    ///
    /// kills: matching on the file's existence alone; matching on schema alone.
    #[test]
    fn a_refusal_from_another_session_is_not_this_run() {
        let dir = temp_dir("foreign");
        write(&dir, &receipt("s-OTHER", &dir, "false"));
        assert_eq!(custody_refusal(&dir, "s-1"), RefusalVerdict::NotRefused);
    }

    /// A refusal that KEPT custody is a different statement: the capture was
    /// custodied and something else went wrong, and its hours are still
    /// accountable. It must not stop the fold.
    ///
    /// kills: treating any refusal record as a custody loss.
    #[test]
    fn a_refusal_that_kept_custody_does_not_stop_the_fold() {
        let dir = temp_dir("custodied");
        write(&dir, &receipt("s-1", &dir, "true"));
        assert_eq!(custody_refusal(&dir, "s-1"), RefusalVerdict::NotRefused);
    }

    /// No supervisor, no receipt, no change in behaviour.
    ///
    /// kills: an implementation that fails closed on absence and stops folding
    /// every unsupervised run.
    #[test]
    fn an_absent_receipt_is_not_a_refusal() {
        let dir = temp_dir("absent");
        assert_eq!(custody_refusal(&dir, "s-1"), RefusalVerdict::NotRefused);
    }

    /// The harness writes UTF-8 WITH A BOM. A reader that cannot parse past it
    /// reports "nothing was refused" on the exact file that refused, which is
    /// the defect this module exists to end.
    ///
    /// kills: handing the bytes straight to serde_json.
    #[test]
    fn a_byte_order_mark_does_not_hide_the_refusal() {
        let dir = temp_dir("bom");
        write(&dir, &format!("\u{feff}{}", receipt("s-1", &dir, "false")));
        assert!(matches!(custody_refusal(&dir, "s-1"), RefusalVerdict::Refused(_)));
    }

    /// A receipt naming a DIFFERENT run directory is not about this run even if
    /// the session id were to collide.
    ///
    /// kills: dropping the out_dir check.
    #[test]
    fn a_refusal_naming_another_directory_is_rejected() {
        let dir = temp_dir("otherdir");
        let elsewhere = temp_dir("otherdir_elsewhere");
        write(&dir, &receipt("s-1", &elsewhere, "false"));
        assert_eq!(custody_refusal(&dir, "s-1"), RefusalVerdict::NotRefused);
    }

    /// A RECEIPT THAT CANNOT BE JUDGED IS NOT A CLEAN BILL OF HEALTH.
    ///
    /// Something wrote a refusal receipt into this run directory. If this reader
    /// cannot parse it, "nothing was refused" is a guess -- and it is the guess
    /// that costs an hour of folding and a full disk, which is the whole reason
    /// this module exists. It fails closed instead and names why. Never a panic:
    /// this runs on the finalize path of every capture.
    /// (Codex review round 3, finding E2.)
    ///
    /// kills: unwrapping the parse; returning NotRefused on a parse failure;
    /// treating a non-UTF-8 or over-long receipt as absence.
    #[test]
    fn an_unjudgeable_receipt_fails_closed() {
        let dir = temp_dir("garbage");
        write(&dir, "{ this is not json");
        let verdict = custody_refusal(&dir, "s-1");
        assert!(matches!(verdict, RefusalVerdict::Unreadable(_)), "{verdict:?}");
        assert!(verdict
            .skip_reason()
            .expect("an unjudgeable receipt must stop the fold")
            .contains("could not be judged"));

        std::fs::write(dir.join(CAPTURE_REFUSAL_FILE), vec![0xffu8; 64]).unwrap();
        assert!(matches!(
            custody_refusal(&dir, "s-1"),
            RefusalVerdict::Unreadable(_)
        ));
    }

    /// A DOCUMENT THAT PARSES IS NOT A DOCUMENT THAT DECIDES. A truncated
    /// receipt for this run -- no schema, no session, no directory, no refusal
    /// list, or a record that will not say whether custody survived -- decides
    /// nothing, and reading it as "nothing was refused" is the fail-open hole
    /// one level in from the parse (Codex review round 4, finding R4.2).
    ///
    /// kills: #[serde(default)] on capture_refusals; admitting a receipt with no
    /// out_dir; treating a missing custody_established as custody kept; treating
    /// an absent schema as a foreign producer's file.
    #[test]
    fn a_receipt_that_parses_but_decides_nothing_fails_closed() {
        let dir = temp_dir("gappy");
        let full = format!("\"session_id\":\"s-1\",\"out_dir\":{}", 
            serde_json::to_string(&dir.to_string_lossy().to_string()).unwrap());
        for (name, body) in [
            ("no schema", format!("{{{full},\"capture_refusals\":[]}}")),
            (
                "no session",
                format!(
                    "{{\"schema\":\"hoi4-capture-refusal-v1\",\"capture_refusals\":[]}}"
                ),
            ),
            (
                "no out_dir",
                "{\"schema\":\"hoi4-capture-refusal-v1\",\"session_id\":\"s-1\",\"capture_refusals\":[]}"
                    .to_string(),
            ),
            (
                "no refusal list",
                format!("{{\"schema\":\"hoi4-capture-refusal-v1\",{full}}}"),
            ),
            (
                "a record with no custody verdict",
                format!(
                    "{{\"schema\":\"hoi4-capture-refusal-v1\",{full},\"capture_refusals\":[{{\"stage\":\"ready_validation\"}}]}}"
                ),
            ),
        ] {
            write(&dir, &body);
            let verdict = custody_refusal(&dir, "s-1");
            assert!(
                matches!(verdict, RefusalVerdict::Unreadable(_)),
                "{name} must fail closed, got {verdict:?}"
            );
        }

        // ...but a receipt that positively belongs to a DIFFERENT producer of
        // this filename is a fact, not a gap, and must still fold.
        write(
            &dir,
            &format!("{{\"schema\":\"some-other-tool-v3\",{full},\"capture_refusals\":[]}}"),
        );
        assert_eq!(custody_refusal(&dir, "s-1"), RefusalVerdict::NotRefused);
    }

    /// The three verdicts must drive the fold decision, and only the first may
    /// let it proceed.
    ///
    /// kills: a `skip_reason` that returns Some for NotRefused (every capture
    /// stops folding) or None for Unreadable (the fail-open hole reopens).
    #[test]
    fn only_a_clean_verdict_lets_the_fold_proceed() {
        assert_eq!(RefusalVerdict::NotRefused.skip_reason(), None);
        assert!(RefusalVerdict::Unreadable("probe".into())
            .skip_reason()
            .is_some());
        assert!(RefusalVerdict::Refused(CustodyRefusal {
            stage: "ready_validation".into(),
            class: "receipt_admission".into(),
            message: "probe".into(),
        })
        .skip_reason()
        .is_some());
    }
}
