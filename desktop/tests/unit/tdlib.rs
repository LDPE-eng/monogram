use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

const KEY: [u8; 32] = [5_u8; 32];
const ENCRYPTED_FIXTURE: &[u8] = include_bytes!("../fixtures/tdlib/encrypted.binlog");
const DAMAGED_FIXTURE: &[u8] = include_bytes!("../fixtures/tdlib/damaged.binlog");

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "monogram-tdlib-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn decode_hex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digits = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(digits, 16).unwrap()
        })
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn encrypted_fixture_decrypts_pmc_auth_key_and_user() {
    let imported = parse_binlog(ENCRYPTED_FIXTURE.to_vec(), &ImportToken::new())
        .unwrap()
        .unwrap();
    assert_eq!(imported.dc_id, 4);
    assert_eq!(imported.user_id, Some(12_345));
    assert_eq!(imported.auth_key.as_slice(), &[7_u8; 256]);
    assert_eq!(auth_key_id(&imported.auth_key), 6_587_356_363_582_867_870);
}

#[test]
fn damaged_fixture_advances_past_bad_records_and_recovers_auth() {
    let imported = parse_binlog(DAMAGED_FIXTURE.to_vec(), &ImportToken::new())
        .unwrap()
        .unwrap();
    assert_eq!(imported.dc_id, 3);
    assert_eq!(imported.user_id, None);
    assert_eq!(imported.auth_key.as_slice(), &[9_u8; 256]);
}

#[test]
fn cancelled_parser_and_import_do_not_write_a_session() {
    let token = ImportToken::new();
    token.cancel();
    assert!(matches!(
        parse_binlog(ENCRYPTED_FIXTURE.to_vec(), &token),
        Err(ImportError::Cancelled)
    ));

    let root = TestDirectory::new("cancelled");
    let source = root.0.join("source");
    let destination = root.0.join("account.session");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("td.binlog"), ENCRYPTED_FIXTURE).unwrap();
    assert!(matches!(
        import(&source, &destination, &KEY, &token),
        Err(ImportError::Cancelled)
    ));
    assert!(!destination.exists());
    assert_eq!(
        fs::read(source.join("td.binlog")).unwrap(),
        ENCRYPTED_FIXTURE
    );
}

#[test]
fn import_commits_encrypted_restorable_mono_session_without_touching_source() {
    let root = TestDirectory::new("roundtrip");
    let source = root.0.join("source");
    let binlog = source.join("td-db/nested/db.binlog");
    let destination = root.0.join("account-1.session");
    fs::create_dir_all(binlog.parent().unwrap()).unwrap();
    fs::write(&binlog, ENCRYPTED_FIXTURE).unwrap();

    let summary = import(&source, &destination, &KEY, &ImportToken::new()).unwrap();
    assert_eq!(summary.files, 1);
    assert!(summary.bytes > 256);
    assert_eq!(summary.destination, destination);
    assert_eq!(fs::read(&binlog).unwrap(), ENCRYPTED_FIXTURE);
    let stored = fs::read(&destination).unwrap();
    assert!(stored.starts_with(session_crypto::MAGIC));
    assert!(serde_json::from_slice::<serde_json::Value>(&stored).is_err());

    let handle = mono_proto_client::create_encrypted_client(
        1,
        "a".repeat(32),
        destination.to_string_lossy().into_owned(),
        KEY.to_vec(),
    )
    .unwrap();
    let client = mono_proto_client::registry::get_client(handle).unwrap();
    {
        let data = client.data.lock();
        assert_eq!(data.home_dc, 4);
        assert_eq!(data.user_id, Some(12_345));
        assert_eq!(data.home_auth_key.as_deref(), Some(&[7_u8; 256][..]));
        assert!(!data.session_dead);
        assert!(!data.test_dc);
    }
    drop(client);
    mono_proto_client::registry::destroy_client(handle);

    let leftovers = fs::read_dir(&root.0)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains(".tdlib-import-")
        })
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn existing_session_is_not_replaced_by_an_import() {
    let root = TestDirectory::new("existing");
    let source = root.0.join("source");
    let destination = root.0.join("account.session");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("td.binlog"), ENCRYPTED_FIXTURE).unwrap();
    fs::write(&destination, b"existing session").unwrap();

    assert!(matches!(
        import(&source, &destination, &KEY, &ImportToken::new()),
        Err(ImportError::DestinationExists)
    ));
    assert_eq!(fs::read(destination).unwrap(), b"existing session");
    assert_eq!(
        fs::read(source.join("td.binlog")).unwrap(),
        ENCRYPTED_FIXTURE
    );
}

#[test]
fn missing_or_mismatched_auth_material_is_rejected() {
    let root = TestDirectory::new("missing");
    let source = root.0.join("source");
    let destination = root.0.join("account.session");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("td.binlog"), b"not a binlog").unwrap();
    assert!(matches!(
        import(&source, &destination, &KEY, &ImportToken::new()),
        Err(ImportError::AuthKeyNotFound)
    ));
    assert!(!destination.exists());

    let mut invalid_blob = Vec::new();
    invalid_blob.extend_from_slice(&1_i64.to_le_bytes());
    invalid_blob.extend_from_slice(&1_i32.to_le_bytes());
    invalid_blob.extend_from_slice(&[254, 0, 1, 0]);
    invalid_blob.extend_from_slice(&[7_u8; 256]);
    invalid_blob.resize(280, 0);
    assert!(parse_auth_key(&invalid_blob).is_none());
}

#[test]
fn aes_ctr_header_authentication_rejects_tampering() {
    let size = le_i32(ENCRYPTED_FIXTURE, 0) as usize;
    let data = &ENCRYPTED_FIXTURE[EVENT_HEADER..size - EVENT_TAIL];
    let rest = &ENCRYPTED_FIXTURE[size..];
    let mut damaged = data.to_vec();
    damaged[4 + 36 + 20 + 1] ^= 1;
    let token = ImportToken::new();
    assert!(decrypt_rest(&damaged, rest, &token).unwrap().is_none());
    assert!(decrypt_rest(data, rest, &token).unwrap().is_some());
}

#[test]
fn aes_ctr_stops_when_cancellation_check_fails() {
    let mut data = vec![0_u8; 128 * 1024];
    let mut checked = false;
    assert!(matches!(
        aes_ctr_apply_with(&mut data, &[3_u8; 32], &[4_u8; 16], || {
            if checked {
                Err(ImportError::Cancelled)
            } else {
                checked = true;
                Ok(())
            }
        }),
        Err(ImportError::Cancelled)
    ));
}

#[cfg(unix)]
#[test]
fn binlog_discovery_does_not_follow_symlinks() {
    let root = TestDirectory::new("symlinks");
    let actual = root.0.join("actual");
    let source = root.0.join("source");
    let destination = root.0.join("account.session");
    fs::create_dir_all(&actual).unwrap();
    fs::create_dir_all(&source).unwrap();
    fs::write(actual.join("td.binlog"), ENCRYPTED_FIXTURE).unwrap();
    std::os::unix::fs::symlink(&actual, source.join("td-db")).unwrap();
    std::os::unix::fs::symlink(actual.join("td.binlog"), source.join("db.binlog")).unwrap();

    assert!(matches!(
        import(&source, &destination, &KEY, &ImportToken::new()),
        Err(ImportError::AuthKeyNotFound)
    ));
    assert!(!destination.exists());
    assert_eq!(
        fs::read(actual.join("td.binlog")).unwrap(),
        ENCRYPTED_FIXTURE
    );
}

#[test]
fn sha_hmac_and_pbkdf2_match_independent_vectors() {
    assert_eq!(
        encode_hex(&sha1(b"abc")),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert_eq!(
        encode_hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        encode_hex(&sha256(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        encode_hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing")),
        "a0f14ba018012b764b32dc157ce0a20e8322dae09da80cae035670ce0879e5d4"
    );
    assert_eq!(
        encode_hex(&pbkdf2_sha256_2(b"password", b"salt")),
        "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
    );
}

#[test]
fn aes256_block_and_big_endian_ctr_match_nist_vectors() {
    let key = decode_hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
    let plain: [u8; 16] = decode_hex("00112233445566778899aabbccddeeff")
        .try_into()
        .unwrap();
    assert_eq!(
        encode_hex(&aes256_encrypt_block(
            plain,
            &aes256_round_keys(key.as_slice().try_into().unwrap())
        )),
        "8ea2b7ca516745bfeafc49904b496089"
    );

    let key = decode_hex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
    let iv = decode_hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
    let mut plain = decode_hex("6bc1bee22e409f96e93d7e117393172a");
    aes_ctr_apply_with(&mut plain, &key, &iv, || Ok(()))
        .unwrap()
        .unwrap();
    assert_eq!(encode_hex(&plain), "601ec313775789a5b7a7f504bbf3d228");
}
