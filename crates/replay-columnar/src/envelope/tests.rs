use super::*;

#[test]
fn a_capture_s_identity_of_another_format_is_taken_and_a_recording_s_is_refused() {
    let identity = Identity { schema: [7; 32], campaign: b"campaign".to_vec(), content: vec![1, 2, 3], capture_mode: 1 };
    let mut older = identity.chain_bytes().unwrap();
    older[0] = (VERSION - 1) as u8;
    assert!(Identity::read(&older).is_err());
    assert_eq!(Identity::read_capture(&older).unwrap(), identity);
    assert_eq!(Identity::read(&identity.chain_bytes().unwrap()).unwrap(), identity);
}
