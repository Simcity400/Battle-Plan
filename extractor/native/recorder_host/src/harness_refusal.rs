use serde::Deserialize;
use std::path::Path;

pub const CAPTURE_REFUSAL_FILE: &str = "CAPTURE_REFUSAL_V1.json";
pub const CAPTURE_REFUSAL_SCHEMA: &str = "hoi4-capture-refusal-v1";

const MAX_REFUSAL_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct RefusalReceipt {
    schema: Option<String>,
    session_id: Option<String>,
    out_dir: Option<String>,
    capture_refusals: Option<Vec<RefusalRecord>>,
}

#[derive(Debug, Deserialize)]
struct RefusalRecord {
    stage: Option<String>,
    class: Option<String>,
    custody_established: Option<bool>,
    message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefusalVerdict {
    NotRefused,
    Refused(CustodyRefusal),
    Unreadable(String),
}

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

pub fn custody_refusal(out_dir: &Path, session_id: &str) -> RefusalVerdict {
    let path = out_dir.join(CAPTURE_REFUSAL_FILE);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return RefusalVerdict::NotRefused
        }
        Err(error) => {
            return RefusalVerdict::Unreadable(format!("its metadata could not be read: {error}"))
        }
    };
    if !metadata.is_file() {
        return RefusalVerdict::Unreadable(format!("{} is not a regular file", path.display()));
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
    let Some(schema) = receipt.schema.as_deref() else {
        return RefusalVerdict::Unreadable("it names no schema".to_string());
    };
    if schema != CAPTURE_REFUSAL_SCHEMA {
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
        let dir =
            std::env::temp_dir().join(format!("ll_harness_refusal_{name}_{}", std::process::id()));
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

    #[test]
    fn a_custody_losing_refusal_for_this_session_is_reported() {
        let dir = temp_dir("hit");
        write(&dir, &receipt("s-1", &dir, "false"));
        let RefusalVerdict::Refused(found) = custody_refusal(&dir, "s-1") else {
            panic!("the refusal must be read");
        };
        assert_eq!(found.stage, "ready_validation");
        assert_eq!(found.class, "receipt_admission");
        assert!(found
            .reason()
            .contains("READY custody was never established"));
    }

    #[test]
    fn a_refusal_from_another_session_is_not_this_run() {
        let dir = temp_dir("foreign");
        write(&dir, &receipt("s-OTHER", &dir, "false"));
        assert_eq!(custody_refusal(&dir, "s-1"), RefusalVerdict::NotRefused);
    }

    #[test]
    fn a_refusal_that_kept_custody_does_not_stop_the_fold() {
        let dir = temp_dir("custodied");
        write(&dir, &receipt("s-1", &dir, "true"));
        assert_eq!(custody_refusal(&dir, "s-1"), RefusalVerdict::NotRefused);
    }

    #[test]
    fn an_absent_receipt_is_not_a_refusal() {
        let dir = temp_dir("absent");
        assert_eq!(custody_refusal(&dir, "s-1"), RefusalVerdict::NotRefused);
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_refusal() {
        let dir = temp_dir("bom");
        write(&dir, &format!("\u{feff}{}", receipt("s-1", &dir, "false")));
        assert!(matches!(
            custody_refusal(&dir, "s-1"),
            RefusalVerdict::Refused(_)
        ));
    }

    #[test]
    fn a_refusal_naming_another_directory_is_rejected() {
        let dir = temp_dir("otherdir");
        let elsewhere = temp_dir("otherdir_elsewhere");
        write(&dir, &receipt("s-1", &elsewhere, "false"));
        assert_eq!(custody_refusal(&dir, "s-1"), RefusalVerdict::NotRefused);
    }

    #[test]
    fn an_unjudgeable_receipt_fails_closed() {
        let dir = temp_dir("garbage");
        write(&dir, "{ this is not json");
        let verdict = custody_refusal(&dir, "s-1");
        assert!(
            matches!(verdict, RefusalVerdict::Unreadable(_)),
            "{verdict:?}"
        );
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

    #[test]
    fn a_receipt_that_parses_but_decides_nothing_fails_closed() {
        let dir = temp_dir("gappy");
        let full = format!(
            "\"session_id\":\"s-1\",\"out_dir\":{}",
            serde_json::to_string(&dir.to_string_lossy().to_string()).unwrap()
        );
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

        write(
            &dir,
            &format!("{{\"schema\":\"some-other-tool-v3\",{full},\"capture_refusals\":[]}}"),
        );
        assert_eq!(custody_refusal(&dir, "s-1"), RefusalVerdict::NotRefused);
    }

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
