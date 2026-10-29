use relix_proto::crypto::{DataKey, MasterKey, Recovery, new_salt};

#[test]
fn same_password_and_salt_give_the_same_login_hash() {
    let salt = new_salt();
    let a = MasterKey::derive("correct horse", &salt);
    let b = MasterKey::derive("correct horse", &salt);
    let c = MasterKey::derive("wrong horse", &salt);
    assert_eq!(a.login_hash(), b.login_hash());
    assert_ne!(a.login_hash(), c.login_hash());
    assert_ne!(a.login_hash(), MasterKey::derive("correct horse", &new_salt()).login_hash(), "salt matters");
}

#[test]
fn the_password_unwraps_the_data_key_and_a_wrong_one_does_not() {
    let salt = new_salt();
    let data = DataKey::generate();
    let wrapped = MasterKey::derive("hunter22", &salt).wrap(&data);

    let back = MasterKey::derive("hunter22", &salt).unwrap(&wrapped).unwrap();
    assert_eq!(back.decrypt(&data.encrypt(b"x")).unwrap(), b"x");
    assert!(MasterKey::derive("hunter23", &salt).unwrap(&wrapped).is_err());
}

#[test]
fn the_recovery_key_unwraps_the_data_key_even_typed_sloppily() {
    let data = DataKey::generate();
    let (recovery, shown) = Recovery::generate();
    let wrapped = recovery.wrap(&data);

    let typed = shown.to_lowercase().replace('-', " ");
    let back = Recovery::parse(&typed).unwrap().unwrap(&wrapped).unwrap();
    assert_eq!(back.decrypt(&data.encrypt(b"x")).unwrap(), b"x");
    assert!(Recovery::parse("not a key").is_err());
}

#[test]
fn records_encrypt_and_decrypt_and_tampering_is_caught() {
    let key = DataKey::generate();
    let secret = r#"[{"manifest_url":"https://torrentio.strem.fun/realdebrid=SECRET/manifest.json"}]"#;
    let a = key.encrypt(secret.as_bytes());
    let b = key.encrypt(secret.as_bytes());
    assert_ne!(a, b, "fresh nonce every time");
    assert!(!a.contains("SECRET"));
    assert_eq!(key.decrypt(&a).unwrap(), secret.as_bytes());

    let mut tampered = a.clone();
    tampered.replace_range(10..11, if &a[10..11] == "A" { "B" } else { "A" });
    assert!(key.decrypt(&tampered).is_err());
    assert!(DataKey::generate().decrypt(&a).is_err(), "another account's key can't read it");
}
