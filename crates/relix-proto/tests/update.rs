use relix_proto::update::{is_newer, sign, signed_message, verify};

#[test]
fn compares_versions_number_by_number() {
    assert!(is_newer("0.2.0", "0.1.9"));
    assert!(is_newer("0.10.0", "0.9.0"), "10 > 9, not text order");
    assert!(is_newer("1.0.0", "0.99.99"));
    assert!(!is_newer("0.1.0", "0.1.0"));
    assert!(!is_newer("0.1.0", "0.2.0"));
    assert!(!is_newer("garbage", "0.1.0"), "an unreadable version never counts as newer");
}

#[test]
fn signed_updates_verify_and_tampered_ones_dont() {
    let secret = [7u8; 32];
    let (public, signature) = sign(&secret, b"the zip");
    assert!(verify(&public, b"the zip", &signature));
    assert!(!verify(&public, b"the zip, modified", &signature));
    let (other_public, _) = sign(&[8u8; 32], b"x");
    assert!(!verify(&other_public, b"the zip", &signature), "signed by someone else");
    assert!(!verify("not base64!", b"the zip", &signature));
}

#[test]
fn a_signature_doesnt_carry_over_to_another_version() {
    let (public, signature) = sign(&[7u8; 32], &signed_message("0.1.0", b"zip"));
    assert!(verify(&public, &signed_message("0.1.0", b"zip"), &signature));
    assert!(!verify(&public, &signed_message("0.9.0", b"zip"), &signature));
}
