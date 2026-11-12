use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use mono_proto_storage::session_crypto::{self, SessionKey};
use rand::{TryRng, rngs::SysRng};
use zeroize::Zeroizing;

pub const MAX_IMPORT_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_SINGLE_FILE_BYTES: u64 = MAX_IMPORT_BYTES;
pub const MAX_ENTRIES: usize = 100_000;
pub const MAX_DEPTH: usize = 64;
const MAX_EVENT_BYTES: usize = 1 << 24;
const MAX_BINLOG_EVENTS: usize = 1_000_000;
const EVENT_HEADER: usize = 28;
const EVENT_TAIL: usize = 4;
const MIN_EVENT_SIZE: usize = EVENT_HEADER + EVENT_TAIL;
const AES_CTR_EVENT: i32 = -3;
const PMC_EVENT: i32 = 0x4327;
const CONFIG_PMC_EVENT: i32 = 0x1f18;
const EMPTY_PASSWORD: &[u8] = b"cucumber";
const HASH_MESSAGE: &[u8] = b"cucumbers everywhere";

#[derive(Debug, Clone)]
pub struct ImportToken {
    cancelled: Arc<AtomicBool>,
}

impl Default for ImportToken {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("selected TDLib database is not a real directory")]
    InvalidSource,
    #[error("session destination already exists and is not empty")]
    DestinationExists,
    #[error("selected TDLib database exceeds the {0} byte import limit")]
    TooLarge(u64),
    #[error("a binlog exceeds the {0} byte limit")]
    FileTooLarge(u64),
    #[error("selected TDLib database has too many entries or events")]
    TooManyEntries,
    #[error("selected TDLib database is nested too deeply")]
    TooDeep,
    #[error("no valid TDLib authorization key was found")]
    AuthKeyNotFound,
    #[error("import was cancelled")]
    Cancelled,
    #[error("encryption key is invalid")]
    InvalidKey,
    #[error("session encoding or encrypted commit failed")]
    Encryption,
    #[error("encrypted session could not be restored and verified")]
    VerificationFailed,
    #[error("filesystem operation failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    pub files: usize,
    pub bytes: u64,
    pub destination: PathBuf,
}

struct ImportedSession {
    dc_id: i32,
    auth_key: Zeroizing<Vec<u8>>,
    user_id: Option<i64>,
}

/// Imports TDLib authorization into a mono-rs session without modifying the source.
pub fn import(
    source: &Path,
    destination: &Path,
    key: &[u8],
    token: &ImportToken,
) -> Result<ImportSummary, ImportError> {
    if token.is_cancelled() {
        return Err(ImportError::Cancelled);
    }
    validate_source_dir(source)?;
    validate_destination(destination)?;
    let nested = source.join("td-db");
    let database = if fs::symlink_metadata(&nested).is_ok_and(|metadata| metadata.is_dir()) {
        nested
    } else {
        source.to_path_buf()
    };
    let imported = scan(&database, token)?;
    let plaintext = Zeroizing::new(session_json(&imported)?);
    let key = session_key(key)?;
    let encrypted =
        session_crypto::encrypt(&plaintext, &key).map_err(|_| ImportError::Encryption)?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = staging_path(destination)?;
    let mut committed = false;

    let result = (|| {
        write_staging(&staging, &encrypted)?;
        if token.is_cancelled() {
            return Err(ImportError::Cancelled);
        }
        verify_restore(&staging, &key, &imported)?;
        if token.is_cancelled() {
            return Err(ImportError::Cancelled);
        }
        fs::rename(&staging, destination)?;
        committed = true;
        fs::File::open(parent)?.sync_all()?;
        verify_restore(destination, &key, &imported)?;
        Ok::<(), ImportError>(())
    })();

    if let Err(error) = result {
        let _ = fs::remove_file(&staging);
        if committed {
            let _ = fs::remove_file(destination);
        }
        return Err(error);
    }

    Ok(ImportSummary {
        files: 1,
        bytes: fs::metadata(destination)?.len(),
        destination: destination.to_path_buf(),
    })
}

fn session_key(bytes: &[u8]) -> Result<SessionKey, ImportError> {
    let array: [u8; 32] = bytes.try_into().map_err(|_| ImportError::InvalidKey)?;
    Ok(Zeroizing::new(array))
}

fn session_json(imported: &ImportedSession) -> Result<Vec<u8>, ImportError> {
    let session_id = SysRng.try_next_u64().map_err(|_| ImportError::Encryption)? as i64 | 1;
    let session = serde_json::json!({
        "snapshot": {
            "dc_id": imported.dc_id,
            "auth_key": imported.auth_key.as_slice(),
            "server_salt": 0,
            "session_id": session_id,
            "last_message_id": 0,
            "content_sequence": 0,
            "time_offset_micros": 0,
            "pending_acknowledgements": [],
            "received_message_ids": {},
            "reconnect": { "attempts": 0, "retry_at_micros": null }
        },
        "user_id": imported.user_id,
        "peers": {},
        "updates": null,
        "media": {},
        "media_order": [],
        "channel_pts": {},
        "channel_recovery": [],
        "seen_messages": [],
        "session_dead": false,
        "logout_tokens": [],
        "new_session": null,
        "test_dc": false,
        "dc_auth_keys": {}
    });
    serde_json::to_vec(&session).map_err(|_| ImportError::Encryption)
}

fn write_staging(path: &Path, bytes: &[u8]) -> Result<(), ImportError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn staging_path(destination: &Path) -> Result<PathBuf, ImportError> {
    let name = destination
        .file_name()
        .ok_or(ImportError::DestinationExists)?;
    let nonce = SysRng.try_next_u64().map_err(|_| ImportError::Encryption)?;
    let mut staging = name.to_os_string();
    staging.push(format!(".tdlib-import-{nonce:016x}.tmp"));
    Ok(destination.with_file_name(staging))
}

fn verify_restore(
    path: &Path,
    key: &SessionKey,
    expected: &ImportedSession,
) -> Result<(), ImportError> {
    let handle = mono_proto_client::create_encrypted_client(
        1,
        "a".repeat(32),
        path.to_string_lossy().into_owned(),
        key.as_ref().to_vec(),
    )
    .map_err(|_| ImportError::VerificationFailed)?;
    let result = mono_proto_client::registry::get_client(handle)
        .map_err(|_| ImportError::VerificationFailed)
        .and_then(|client| {
            let data = client.data.lock();
            if data.home_dc == expected.dc_id
                && data.user_id == expected.user_id
                && data.home_auth_key.as_deref() == Some(expected.auth_key.as_slice())
            {
                Ok(())
            } else {
                Err(ImportError::VerificationFailed)
            }
        });
    mono_proto_client::registry::destroy_client(handle);
    result
}

fn validate_source_dir(source: &Path) -> Result<(), ImportError> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ImportError::InvalidSource);
    }
    Ok(())
}

fn validate_destination(destination: &Path) -> Result<(), ImportError> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != 0 {
                return Err(ImportError::DestinationExists);
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn scan(database: &Path, token: &ImportToken) -> Result<ImportedSession, ImportError> {
    let files = binlog_files(database, token)?;
    let mut total = 0_u64;
    for path in files {
        if token.is_cancelled() {
            return Err(ImportError::Cancelled);
        }
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.len() > MAX_SINGLE_FILE_BYTES {
            continue;
        }
        total = total.saturating_add(metadata.len());
        if total > MAX_IMPORT_BYTES {
            return Err(ImportError::TooLarge(total));
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        if bytes.len() as u64 > MAX_SINGLE_FILE_BYTES {
            return Err(ImportError::FileTooLarge(bytes.len() as u64));
        }
        if let Some(imported) = parse_binlog(bytes, token)? {
            return Ok(imported);
        }
    }
    Err(ImportError::AuthKeyNotFound)
}

fn binlog_files(database: &Path, token: &ImportToken) -> Result<Vec<PathBuf>, ImportError> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for name in ["td.binlog", "db.binlog"] {
        let path = database.join(name);
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
            if let Ok(canonical) = path.canonicalize() {
                seen.insert(canonical);
            }
            files.push(path);
        }
    }

    let mut stack = vec![(database.to_path_buf(), 0_usize)];
    let mut entries = 0_usize;
    while let Some((directory, depth)) = stack.pop() {
        if token.is_cancelled() {
            return Err(ImportError::Cancelled);
        }
        if depth > MAX_DEPTH {
            return Err(ImportError::TooDeep);
        }
        for item in fs::read_dir(directory)? {
            let item = item?;
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(ImportError::TooManyEntries);
            }
            let path = item.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push((path, depth + 1));
            } else if metadata.is_file() && item.file_name().to_string_lossy().contains("binlog") {
                let canonical = path.canonicalize()?;
                if seen.insert(canonical) {
                    files.push(path);
                }
            }
        }
    }
    Ok(files)
}

fn parse_binlog(
    mut buffer: Vec<u8>,
    token: &ImportToken,
) -> Result<Option<ImportedSession>, ImportError> {
    let mut pmc = BTreeMap::<String, Vec<u8>>::new();
    let mut offset = 0_usize;
    let mut decrypted = false;
    let mut steps = 0_usize;

    while offset + MIN_EVENT_SIZE <= buffer.len() {
        if token.is_cancelled() {
            return Err(ImportError::Cancelled);
        }
        steps += 1;
        if steps > MAX_BINLOG_EVENTS {
            return Err(ImportError::TooManyEntries);
        }
        let raw_size = le_i32(&buffer, offset);
        let valid_size = raw_size >= MIN_EVENT_SIZE as i32
            && raw_size <= MAX_EVENT_BYTES as i32
            && raw_size % 4 == 0
            && (raw_size as usize) <= buffer.len() - offset;
        if !valid_size {
            offset += 4;
            continue;
        }
        let size = raw_size as usize;
        let event_type = le_i32(&buffer, offset + 12);
        let data = &buffer[offset + EVENT_HEADER..offset + size - EVENT_TAIL];

        if event_type == AES_CTR_EVENT && !decrypted {
            if let Some(plain) = decrypt_rest(data, &buffer[offset + size..], token)? {
                buffer = plain;
                offset = 0;
                decrypted = true;
                continue;
            }
        } else if (event_type == PMC_EVENT || event_type == CONFIG_PMC_EVENT)
            && let Some((key, value)) = fetch_key_value(data)
        {
            pmc.insert(key, value);
        }
        offset += size;
    }

    if let Some(imported) = imported_from_pmc(&pmc) {
        return Ok(Some(imported));
    }
    Ok(find_auth_key(&buffer).map(|key| ImportedSession {
        dc_id: pmc_dc_id(&pmc).or_else(|| find_dc_id(&buffer)).unwrap_or(2),
        auth_key: Zeroizing::new(key),
        user_id: pmc_user_id(&pmc),
    }))
}

fn imported_from_pmc(pmc: &BTreeMap<String, Vec<u8>>) -> Option<ImportedSession> {
    let dc_id =
        pmc_dc_id(pmc).or_else(|| (1..=5).find(|dc| pmc.contains_key(&format!("auth{dc}"))))?;
    let key = parse_auth_key(pmc.get(&format!("auth{dc_id}"))?)?;
    Some(ImportedSession {
        dc_id,
        auth_key: Zeroizing::new(key),
        user_id: pmc_user_id(pmc),
    })
}

fn pmc_dc_id(pmc: &BTreeMap<String, Vec<u8>>) -> Option<i32> {
    let text = std::str::from_utf8(pmc.get("main_dc_id")?).ok()?.trim();
    text.parse::<i32>().ok().filter(|dc| (1..=5).contains(dc))
}

fn pmc_user_id(pmc: &BTreeMap<String, Vec<u8>>) -> Option<i64> {
    let text = std::str::from_utf8(pmc.get("my_id")?).ok()?.trim();
    text.parse::<i64>()
        .or_else(|_| text.strip_prefix("user").unwrap_or_default().parse::<i64>())
        .ok()
        .filter(|id| *id > 0)
}

fn fetch_key_value(data: &[u8]) -> Option<(String, Vec<u8>)> {
    let (key, next) = fetch_tl_string(data, 0)?;
    let (value, _) = fetch_tl_string(data, next)?;
    if key.is_empty() {
        return None;
    }
    Some((std::str::from_utf8(key).ok()?.to_owned(), value.to_vec()))
}

fn fetch_tl_string(bytes: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let first = *bytes.get(start)?;
    let (length, payload_start, total) = if first < 254 {
        (
            usize::from(first),
            start.checked_add(1)?,
            start.checked_add(4 + usize::from(first & 0x7c))?,
        )
    } else if first == 254 {
        let header = bytes.get(start..start.checked_add(4)?)?;
        let length =
            usize::from(header[1]) | (usize::from(header[2]) << 8) | (usize::from(header[3]) << 16);
        let total = start.checked_add(4 + length.checked_add(3)? / 4 * 4)?;
        (length, start.checked_add(4)?, total)
    } else {
        return None;
    };
    if total > bytes.len() || length > bytes.len() - payload_start {
        return None;
    }
    Some((&bytes[payload_start..payload_start + length], total))
}

fn parse_auth_key(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 12 {
        return None;
    }
    let stored_id = le_i64(bytes, 0);
    let (key, _) = fetch_tl_string(bytes, 12)?;
    if key.len() != 256 || auth_key_id(key) != stored_id {
        return None;
    }
    Some(key.to_vec())
}

fn find_auth_key(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut offset = 0_usize;
    while offset + 264 <= bytes.len() {
        let stored_id = le_i64(bytes, offset);
        if stored_id != 0 {
            let key = &bytes[offset + 8..offset + 264];
            if auth_key_id(key) == stored_id {
                return Some(key.to_vec());
            }
        }
        offset += 8;
    }
    None
}

fn find_dc_id(bytes: &[u8]) -> Option<i32> {
    let mut offset = 0_usize;
    while offset + 4 <= bytes.len() {
        if bytes[offset + 2] == 0 && bytes[offset + 3] == 0 {
            let dc = i32::from(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]));
            if (1..=5).contains(&dc) {
                return Some(dc);
            }
        }
        offset += 4;
    }
    None
}

fn auth_key_id(key: &[u8]) -> i64 {
    i64::from_le_bytes(sha1(key)[12..20].try_into().expect("fixed SHA-1 slice"))
}

fn decrypt_rest(
    event_data: &[u8],
    rest: &[u8],
    token: &ImportToken,
) -> Result<Option<Vec<u8>>, ImportError> {
    let Some((salt, next)) = fetch_tl_string(event_data, 4) else {
        return Ok(None);
    };
    let Some((iv, next)) = fetch_tl_string(event_data, next) else {
        return Ok(None);
    };
    let Some((stored_hash, _)) = fetch_tl_string(event_data, next) else {
        return Ok(None);
    };
    if salt.is_empty() || iv.len() != 16 || stored_hash.len() != 32 {
        return Ok(None);
    }
    let key = Zeroizing::new(pbkdf2_sha256_2(EMPTY_PASSWORD, salt));
    let expected = hmac_sha256(key.as_ref(), HASH_MESSAGE);
    if !constant_time_eq(&expected, stored_hash) {
        return Ok(None);
    }
    let mut plain = rest.to_vec();
    let Some(()) = aes_ctr_apply_with(&mut plain, key.as_ref(), iv, || {
        if token.is_cancelled() {
            Err(ImportError::Cancelled)
        } else {
            Ok(())
        }
    })?
    else {
        return Ok(None);
    };
    Ok(Some(plain))
}

fn le_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked event header"),
    )
}

fn le_i64(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("checked integer slice"),
    )
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (&left, &right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

fn pbkdf2_sha256_2(password: &[u8], salt: &[u8]) -> [u8; 32] {
    let mut first_input = Vec::with_capacity(salt.len() + 4);
    first_input.extend_from_slice(salt);
    first_input.extend_from_slice(&1_u32.to_be_bytes());
    let first = hmac_sha256(password, &first_input);
    let second = hmac_sha256(password, &first);
    let mut output = [0_u8; 32];
    for index in 0..32 {
        output[index] = first[index] ^ second[index];
    }
    output
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0_u8; 64];
    if key.len() > block.len() {
        block[..32].copy_from_slice(&sha256(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = [0_u8; 64];
    let mut outer = [0_u8; 64];
    for index in 0..64 {
        inner[index] = block[index] ^ 0x36;
        outer[index] = block[index] ^ 0x5c;
    }
    let mut inner_message = Vec::with_capacity(64 + message.len());
    inner_message.extend_from_slice(&inner);
    inner_message.extend_from_slice(message);
    let inner_hash = sha256(&inner_message);
    let mut outer_message = Vec::with_capacity(64 + inner_hash.len());
    outer_message.extend_from_slice(&outer);
    outer_message.extend_from_slice(&inner_hash);
    sha256(&outer_message)
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut message = Vec::with_capacity(data.len() + 72);
    message.extend_from_slice(data);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(data.len() as u64).wrapping_mul(8).to_be_bytes());

    let mut state = [
        0x6745_2301_u32,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    for chunk in message.as_chunks::<64>().0 {
        let mut words = [0_u32; 80];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes(
                chunk[index * 4..index * 4 + 4]
                    .try_into()
                    .expect("SHA-1 word"),
            );
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in words.iter().enumerate() {
            let (function, constant) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(function)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (value, addition) in state.iter_mut().zip([a, b, c, d, e]) {
            *value = value.wrapping_add(addition);
        }
    }

    let mut output = [0_u8; 20];
    for (chunk, value) in output.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    output
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut message = Vec::with_capacity(data.len() + 72);
    message.extend_from_slice(data);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(data.len() as u64).wrapping_mul(8).to_be_bytes());
    let mut state = [
        0x6a09_e667_u32,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    for chunk in message.as_chunks::<64>().0 {
        let mut words = [0_u32; 64];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes(
                chunk[index * 4..index * 4 + 4]
                    .try_into()
                    .expect("SHA-256 word"),
            );
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let first = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let second = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(first);
            d = c;
            c = b;
            b = a;
            a = first.wrapping_add(second);
        }
        for (value, addition) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *value = value.wrapping_add(addition);
        }
    }

    let mut output = [0_u8; 32];
    for (chunk, value) in output.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    output
}

fn aes_ctr_apply_with(
    data: &mut [u8],
    key: &[u8],
    iv: &[u8],
    mut check: impl FnMut() -> Result<(), ImportError>,
) -> Result<Option<()>, ImportError> {
    if key.len() != 32 || iv.len() != 16 {
        return Ok(None);
    }
    let round_keys = aes256_round_keys(key.try_into().expect("validated AES key"));
    let mut counter: [u8; 16] = iv.try_into().expect("validated AES IV");
    for large_chunk in data.chunks_mut(64 * 1024) {
        check()?;
        for chunk in large_chunk.chunks_mut(16) {
            let stream = aes256_encrypt_block(counter, &round_keys);
            for (byte, mask) in chunk.iter_mut().zip(stream) {
                *byte ^= mask;
            }
            for byte in counter.iter_mut().rev() {
                let (next, overflow) = byte.overflowing_add(1);
                *byte = next;
                if !overflow {
                    break;
                }
            }
        }
    }
    Ok(Some(()))
}

fn aes256_round_keys(key: &[u8; 32]) -> [u32; 60] {
    const RCON: [u32; 8] = [0, 1, 2, 4, 8, 16, 32, 64];
    let mut words = [0_u32; 60];
    for (index, chunk) in key.as_chunks::<4>().0.iter().enumerate() {
        words[index] = u32::from_be_bytes(*chunk);
    }
    for index in 8..60 {
        let mut temporary = words[index - 1];
        if index % 8 == 0 {
            temporary = aes_sub_word(temporary.rotate_left(8)) ^ (RCON[index / 8] << 24);
        } else if index % 8 == 4 {
            temporary = aes_sub_word(temporary);
        }
        words[index] = words[index - 8] ^ temporary;
    }
    words
}

fn aes_sub_word(word: u32) -> u32 {
    let bytes = word.to_be_bytes();
    u32::from_be_bytes(bytes.map(|byte| AES_SBOX[usize::from(byte)]))
}

fn aes256_encrypt_block(mut state: [u8; 16], round_keys: &[u32; 60]) -> [u8; 16] {
    aes_add_round_key(&mut state, round_keys, 0);
    for round in 1..14 {
        aes_sub_bytes(&mut state);
        aes_shift_rows(&mut state);
        aes_mix_columns(&mut state);
        aes_add_round_key(&mut state, round_keys, round);
    }
    aes_sub_bytes(&mut state);
    aes_shift_rows(&mut state);
    aes_add_round_key(&mut state, round_keys, 14);
    state
}

fn aes_add_round_key(state: &mut [u8; 16], words: &[u32; 60], round: usize) {
    for column in 0..4 {
        let key = words[round * 4 + column].to_be_bytes();
        for row in 0..4 {
            state[column * 4 + row] ^= key[row];
        }
    }
}

fn aes_sub_bytes(state: &mut [u8; 16]) {
    for byte in state {
        *byte = AES_SBOX[usize::from(*byte)];
    }
}

fn aes_shift_rows(state: &mut [u8; 16]) {
    let original = *state;
    for row in 0..4 {
        for column in 0..4 {
            state[column * 4 + row] = original[((column + row) % 4) * 4 + row];
        }
    }
}

fn aes_mix_columns(state: &mut [u8; 16]) {
    for column in state.as_chunks_mut::<4>().0.iter_mut() {
        let combined = column[0] ^ column[1] ^ column[2] ^ column[3];
        let first = column[0];
        column[0] ^= combined ^ aes_xtime(column[0] ^ column[1]);
        column[1] ^= combined ^ aes_xtime(column[1] ^ column[2]);
        column[2] ^= combined ^ aes_xtime(column[2] ^ column[3]);
        column[3] ^= combined ^ aes_xtime(column[3] ^ first);
    }
}

fn aes_xtime(value: u8) -> u8 {
    (value << 1) ^ if value & 0x80 == 0 { 0 } else { 0x1b }
}

const AES_SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

#[cfg(test)]
#[path = "../../tests/unit/tdlib.rs"]
mod tests;
