//! Where a prepared answer is kept on disk.

use super::*;

/// A kept answer is filed under a digest of the store's key, whatever the size of the content
/// stack it stamps: the same for equal keys, another for keys stamping other content.
#[test]
fn kept_answers_are_filed_under_a_digest_of_the_store_key() {
    let folder = std::env::temp_dir().join(format!("kept-key-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&folder).unwrap();
    let recording = folder.join("synthetic.battleplan");
    std::fs::write(&recording, b"synthetic recording").unwrap();
    let files: Vec<_> = (0..200)
        .map(|index| {
            let path = folder.join(format!("content-{index}.txt"));
            std::fs::write(&path, format!("file {index}")).unwrap();
            path
        })
        .collect();
    let key = store::SourceKey::capture(&recording, &files).unwrap();
    let digest = key_digest(&key).unwrap();
    assert_eq!(digest.len(), 64, "{digest}");
    assert_eq!(key_digest(&key.clone()).unwrap(), digest);
    let again = store::SourceKey::capture(&recording, &files).unwrap();
    assert_eq!(key_digest(&again).unwrap(), digest, "an equal key files alike");
    std::fs::write(&files[7], "changed content, longer than before").unwrap();
    let changed = store::SourceKey::capture(&recording, &files).unwrap();
    assert_ne!(
        key_digest(&changed).unwrap(),
        digest,
        "other content files elsewhere"
    );
    let _ = std::fs::remove_dir_all(&folder);
}
