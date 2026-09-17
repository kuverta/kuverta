//! The passwords Thunderbird has saved, so an import needs none typed again.
//!
//! Thunderbird keeps logins in `logins.json`, encrypted with a key in
//! `key4.db` beside it — the same profile the servers are read from. Without a
//! primary password the key is protected by an empty one, which is why
//! anything that can read the profile can read the passwords: the file is the
//! secret. With a primary password set, that password is needed here too.
//!
//! The scheme is NSS's, and this implements the reading half of it:
//!
//! 1. `key4.db`'s `metaData` row `password` holds a global salt and an
//!    encrypted `password-check`, which says whether the primary password is
//!    right before anything else is attempted.
//! 2. `nssPrivate` holds the key the logins are encrypted with, encrypted the
//!    same way.
//! 3. Each login's fields are DER: a key id, the cipher and its IV, and the
//!    ciphertext — 3DES on older profiles, AES-256 on newer ones.
//!
//! Nothing here writes to the profile, and no password is returned to a
//! window: [`crate::setup`] puts them straight into the keychain.

use std::path::Path;

use aes::Aes256;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, KeyIvInit};
use des::TdesEde3;
use sha1::{Digest, Sha1};
use sha2::Sha256;

/// A login as Thunderbird saved it.
#[derive(Debug, Clone, PartialEq)]
pub struct Login {
    /// `imap://mail.example.de` or `smtp://mail.example.de`.
    pub origin: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("this profile has no saved passwords")]
    None,

    #[error("Thunderbird's saved passwords are behind its primary password")]
    NeedsPrimaryPassword,

    #[error("that is not the primary password")]
    WrongPrimaryPassword,

    #[error("could not read Thunderbird's passwords: {0}")]
    Unreadable(String),
}

type Result<T> = std::result::Result<T, PasswordError>;

fn unreadable(what: impl std::fmt::Display) -> PasswordError {
    PasswordError::Unreadable(what.to_string())
}

/// The logins saved in one Thunderbird profile.
///
/// `primary_password` is empty for a profile without one, which is the usual
/// case.
pub fn read(profile: &Path, primary_password: &str) -> Result<Vec<Login>> {
    let logins = profile.join("logins.json");
    if !logins.exists() {
        return Err(PasswordError::None);
    }
    let keys = key_for(profile, primary_password)?;
    let text = std::fs::read_to_string(&logins).map_err(unreadable)?;
    read_logins(&text, &keys)
}

/// Whether this profile's passwords need a primary password before they can
/// be read at all.
pub fn needs_primary_password(profile: &Path) -> bool {
    matches!(
        key_for(profile, ""),
        Err(PasswordError::WrongPrimaryPassword) | Err(PasswordError::NeedsPrimaryPassword)
    )
}

/// The keys the logins are encrypted with. A profile keeps one of each kind
/// as it is rewritten over the years, and each login says which it needs.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Keys {
    /// 24 bytes, for 3DES.
    pub des: Option<Vec<u8>>,
    /// 32 bytes, for AES-256.
    pub aes: Option<Vec<u8>>,
}

impl Keys {
    fn add(&mut self, key: Vec<u8>) {
        match key.len() {
            24 => self.des = Some(key),
            32 => self.aes = Some(key),
            _ => {}
        }
    }

    fn is_empty(&self) -> bool {
        self.des.is_none() && self.aes.is_none()
    }
}

/// The keys the logins are encrypted with.
fn key_for(profile: &Path, primary_password: &str) -> Result<Keys> {
    let db = profile.join("key4.db");
    if !db.exists() {
        return Err(PasswordError::None);
    }
    // A copy, because Thunderbird may be running and the database is then in
    // use — the same reason Apple Mail's accounts are read from a copy.
    let copy = copy_aside(&db)?;
    let result = key_from_db(&copy.0, primary_password);
    let _ = std::fs::remove_dir_all(&copy.1);
    result
}

struct Copied(std::path::PathBuf, std::path::PathBuf);

fn copy_aside(file: &Path) -> Result<Copied> {
    let dir = std::env::temp_dir().join(format!("kuverta-key4-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(unreadable)?;
    let name = file.file_name().unwrap_or_default();
    let copy = dir.join(name);
    std::fs::copy(file, &copy).map_err(unreadable)?;
    // The write-ahead log, when there is one, holds the newest rows.
    for suffix in ["-wal", "-shm"] {
        let extra = file.with_file_name(format!("{}{suffix}", name.to_string_lossy()));
        if extra.exists() {
            let _ = std::fs::copy(&extra, dir.join(extra.file_name().unwrap_or_default()));
        }
    }
    Ok(Copied(copy, dir))
}

fn key_from_db(db: &Path, primary_password: &str) -> Result<Keys> {
    let connection = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(unreadable)?;

    let (global_salt, check): (Vec<u8>, Vec<u8>) = connection
        .query_row(
            "SELECT item1, item2 FROM metaData WHERE id = 'password'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => PasswordError::None,
            other => unreadable(other),
        })?;

    // "password-check" comes back only for the right primary password.
    let checked = decrypt_item(&check, &global_salt, primary_password)?;
    if !checked.starts_with(b"password-check") {
        return Err(if primary_password.is_empty() {
            PasswordError::NeedsPrimaryPassword
        } else {
            PasswordError::WrongPrimaryPassword
        });
    }

    // The key itself, under the id NSS gives it.
    const KEY_ID: [u8; 16] = [
        0xf8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ];
    let mut keys = Keys::default();
    let mut statement = connection
        .prepare("SELECT a11, a102 FROM nssPrivate")
        .map_err(unreadable)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<Vec<u8>>>(0)?,
                row.get::<_, Option<Vec<u8>>>(1)?,
            ))
        })
        .map_err(unreadable)?;
    for row in rows {
        let (Some(item), Some(id)) = row.map_err(unreadable)? else {
            continue;
        };
        if id != KEY_ID {
            continue;
        }
        keys.add(decrypt_item(&item, &global_salt, primary_password)?);
    }
    if keys.is_empty() {
        return Err(PasswordError::None);
    }
    Ok(keys)
}

/// One of NSS's encrypted items: DER describing how it was encrypted, and the
/// ciphertext. Either PBES2 with AES-256, or the older PBE with 3DES.
fn decrypt_item(item: &[u8], global_salt: &[u8], primary_password: &str) -> Result<Vec<u8>> {
    let outer = der::sequence(item).ok_or_else(|| unreadable("not a stored item"))?;
    let (algorithm, rest) = der::next(outer).ok_or_else(|| unreadable("no algorithm"))?;
    let (ciphertext, _) = der::next(rest).ok_or_else(|| unreadable("no ciphertext"))?;
    let ciphertext = der::octet_string(ciphertext).ok_or_else(|| unreadable("no ciphertext"))?;

    let parameters = der::sequence(algorithm).ok_or_else(|| unreadable("no parameters"))?;
    let (oid, parameters) = der::next(parameters).ok_or_else(|| unreadable("no algorithm id"))?;
    let oid = der::oid(oid).ok_or_else(|| unreadable("no algorithm id"))?;

    // PBES2: PBKDF2 with SHA-256, then AES-256-CBC.
    if oid == der::OID_PBES2 {
        let (kdf, rest) =
            der::next(der::sequence(parameters).ok_or_else(|| unreadable("no PBES2 parameters"))?)
                .ok_or_else(|| unreadable("no key derivation"))?;
        let (cipher, _) = der::next(rest).ok_or_else(|| unreadable("no cipher"))?;

        let kdf = der::sequence(kdf).ok_or_else(|| unreadable("no key derivation"))?;
        let (_, kdf_parameters) = der::next(kdf).ok_or_else(|| unreadable("no key derivation"))?;
        let kdf_parameters =
            der::sequence(kdf_parameters).ok_or_else(|| unreadable("no key derivation"))?;
        let (salt, rest) = der::next(kdf_parameters).ok_or_else(|| unreadable("no salt"))?;
        let salt = der::octet_string(salt).ok_or_else(|| unreadable("no salt"))?;
        let (iterations, rest) = der::next(rest).ok_or_else(|| unreadable("no iterations"))?;
        let iterations = der::integer(iterations).ok_or_else(|| unreadable("no iterations"))?;
        let (length, _) = der::next(rest).ok_or_else(|| unreadable("no key length"))?;
        let length = der::integer(length).ok_or_else(|| unreadable("no key length"))?;
        if length != 32 {
            return Err(unreadable(format!("a {length}-byte key is not AES-256")));
        }

        let cipher = der::sequence(cipher).ok_or_else(|| unreadable("no cipher"))?;
        let (_, iv) = der::next(cipher).ok_or_else(|| unreadable("no cipher"))?;
        let iv = der::octet_string(iv).ok_or_else(|| unreadable("no IV"))?;

        // NSS writes the last fourteen bytes of the IV and prefixes the two
        // bytes every one of its items begins with.
        let mut whole_iv = vec![0x04, 0x0e];
        whole_iv.extend_from_slice(iv);
        if whole_iv.len() != 16 {
            return Err(unreadable("the IV is not sixteen bytes"));
        }

        let mut secret = Sha1::new();
        secret.update(global_salt);
        secret.update(primary_password.as_bytes());
        let secret = secret.finalize();

        let mut key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(
            &secret,
            salt,
            u32::try_from(iterations).map_err(|_| unreadable("too many iterations"))?,
            &mut key,
        );
        return aes_cbc(&key, &whole_iv, ciphertext);
    }

    Err(unreadable(
        "an encryption this does not know; the profile may predate Thunderbird 60",
    ))
}

fn aes_cbc(key: &[u8], iv: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    cbc::Decryptor::<Aes256>::new_from_slices(key, iv)
        .map_err(unreadable)?
        .decrypt_padded_vec::<Pkcs7>(ciphertext)
        .map_err(|_| PasswordError::WrongPrimaryPassword)
}

fn des_cbc(key: &[u8], iv: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    cbc::Decryptor::<TdesEde3>::new_from_slices(key, iv)
        .map_err(unreadable)?
        .decrypt_padded_vec::<Pkcs7>(ciphertext)
        .map_err(|_| PasswordError::WrongPrimaryPassword)
}

/// The logins in a `logins.json`, decrypted with the profile's key.
pub fn read_logins(json: &str, keys: &Keys) -> Result<Vec<Login>> {
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        logins: Vec<Entry>,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        #[serde(default)]
        hostname: Option<String>,
        #[serde(default)]
        origin: Option<String>,
        #[serde(rename = "encryptedUsername")]
        username: String,
        #[serde(rename = "encryptedPassword")]
        password: String,
    }

    let file: File = serde_json::from_str(json).map_err(unreadable)?;
    let mut logins = Vec::new();
    for entry in file.logins {
        let Some(origin) = entry.origin.or(entry.hostname) else {
            continue;
        };
        // One unreadable login does not spoil the rest: an account whose
        // password is missing is one the person types again.
        let (Ok(username), Ok(password)) = (
            decrypt_field(&entry.username, keys),
            decrypt_field(&entry.password, keys),
        ) else {
            continue;
        };
        logins.push(Login {
            origin,
            username,
            password,
        });
    }
    Ok(logins)
}

/// One `encryptedUsername` or `encryptedPassword`: base64 around DER that says
/// which key, which cipher, and the ciphertext.
fn decrypt_field(field: &str, keys: &Keys) -> Result<String> {
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(field.trim())
        .map_err(unreadable)?;
    let outer = der::sequence(&raw).ok_or_else(|| unreadable("not an encrypted field"))?;
    let (_key_id, rest) = der::next(outer).ok_or_else(|| unreadable("no key id"))?;
    let (algorithm, rest) = der::next(rest).ok_or_else(|| unreadable("no cipher"))?;
    let (ciphertext, _) = der::next(rest).ok_or_else(|| unreadable("no ciphertext"))?;
    let ciphertext = der::octet_string(ciphertext).ok_or_else(|| unreadable("no ciphertext"))?;

    let algorithm = der::sequence(algorithm).ok_or_else(|| unreadable("no cipher"))?;
    let (oid, iv) = der::next(algorithm).ok_or_else(|| unreadable("no cipher"))?;
    let oid = der::oid(oid).ok_or_else(|| unreadable("no cipher"))?;
    let iv = der::octet_string(iv).ok_or_else(|| unreadable("no IV"))?;

    let key = |key: &Option<Vec<u8>>| {
        key.clone()
            .ok_or_else(|| unreadable("this profile has no key of that kind"))
    };
    let plain = match oid {
        der::OID_DES_EDE3_CBC => des_cbc(&key(&keys.des)?, iv, ciphertext)?,
        der::OID_AES_256_CBC => aes_cbc(&key(&keys.aes)?, iv, ciphertext)?,
        _ => return Err(unreadable("a cipher this does not know")),
    };
    String::from_utf8(plain).map_err(unreadable)
}

/// Just enough DER to walk NSS's structures: what is inside a SEQUENCE, the
/// next value beside one, and the three kinds of leaf that matter.
mod der {
    pub const OID_PBES2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0d];
    pub const OID_DES_EDE3_CBC: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x03, 0x07];
    pub const OID_AES_256_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2a];

    /// A value's tag, its contents, and whatever follows it.
    fn split(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
        let (&tag, rest) = bytes.split_first()?;
        let (&first, rest) = rest.split_first()?;
        let (length, rest) = if first < 0x80 {
            (first as usize, rest)
        } else {
            // Long form: the low bits say how many bytes the length takes.
            let count = (first & 0x7f) as usize;
            if count == 0 || count > 4 || rest.len() < count {
                return None;
            }
            let (bytes, rest) = rest.split_at(count);
            (
                bytes.iter().fold(0usize, |n, &b| (n << 8) | b as usize),
                rest,
            )
        };
        if rest.len() < length {
            return None;
        }
        let (value, after) = rest.split_at(length);
        Some((tag, value, after))
    }

    /// What is inside a SEQUENCE, or `None` for anything else.
    pub fn sequence(bytes: &[u8]) -> Option<&[u8]> {
        match split(bytes)? {
            (0x30, value, _) => Some(value),
            _ => None,
        }
    }

    /// The next value in a sequence's contents, whole, and the rest.
    pub fn next(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
        let (_, _, after) = split(bytes)?;
        let taken = bytes.len() - after.len();
        Some((&bytes[..taken], after))
    }

    pub fn octet_string(bytes: &[u8]) -> Option<&[u8]> {
        match split(bytes)? {
            (0x04, value, _) => Some(value),
            _ => None,
        }
    }

    pub fn oid(bytes: &[u8]) -> Option<&[u8]> {
        match split(bytes)? {
            (0x06, value, _) => Some(value),
            _ => None,
        }
    }

    pub fn integer(bytes: &[u8]) -> Option<i64> {
        match split(bytes)? {
            (0x02, value, _) if value.len() <= 8 => {
                Some(value.iter().fold(0i64, |n, &b| (n << 8) | b as i64))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use cbc::cipher::BlockModeEncrypt;

    /// A field as Thunderbird writes one: DER around AES-256-CBC.
    fn encrypted_field(key: &[u8; 32], iv: &[u8; 16], text: &str) -> String {
        let ciphertext = cbc::Encryptor::<Aes256>::new_from_slices(key, iv)
            .unwrap()
            .encrypt_padded_vec::<Pkcs7>(text.as_bytes());

        let der = |tag: u8, value: &[u8]| {
            let mut out = vec![tag];
            if value.len() < 0x80 {
                out.push(value.len() as u8);
            } else {
                out.push(0x82);
                out.extend_from_slice(&(value.len() as u16).to_be_bytes());
            }
            out.extend_from_slice(value);
            out
        };
        let mut cipher = der(0x06, der::OID_AES_256_CBC);
        cipher.extend(der(0x04, iv));
        let mut inner = der(0x04, &[0u8; 16]); // the key id
        inner.extend(der(0x30, &cipher));
        inner.extend(der(0x04, &ciphertext));
        base64::engine::general_purpose::STANDARD.encode(der(0x30, &inner))
    }

    #[test]
    fn a_login_is_read_back_with_the_key_it_was_written_with() {
        let key = [7u8; 32];
        let iv = [9u8; 16];
        let json = format!(
            r#"{{"logins":[{{"hostname":"imap://mail.example.de","encryptedUsername":"{}","encryptedPassword":"{}"}}]}}"#,
            encrypted_field(&key, &iv, "erika"),
            encrypted_field(&key, &iv, "hunter2 with spaces"),
        );

        let keys = Keys {
            des: None,
            aes: Some(key.to_vec()),
        };
        let logins = read_logins(&json, &keys).unwrap();
        assert_eq!(
            logins,
            vec![Login {
                origin: "imap://mail.example.de".into(),
                username: "erika".into(),
                password: "hunter2 with spaces".into(),
            }]
        );

        // The wrong key leaves the login out rather than returning nonsense.
        let other = Keys {
            des: None,
            aes: Some([8u8; 32].to_vec()),
        };
        assert!(read_logins(&json, &other).unwrap().is_empty());
        // A profile with no AES key at all does not panic either.
        assert!(read_logins(&json, &Keys::default()).unwrap().is_empty());
    }

    #[test]
    fn der_values_are_walked_in_order() {
        // SEQUENCE { INTEGER 3, OCTET STRING "hi", OID 1.2.840.113549.3.7 }
        let bytes = [
            0x30, 0x11, 0x02, 0x01, 0x03, 0x04, 0x02, b'h', b'i', 0x06, 0x08, 0x2a, 0x86, 0x48,
            0x86, 0xf7, 0x0d, 0x03, 0x07,
        ];
        let inside = der::sequence(&bytes).unwrap();
        let (first, rest) = der::next(inside).unwrap();
        assert_eq!(der::integer(first), Some(3));
        let (second, rest) = der::next(rest).unwrap();
        assert_eq!(der::octet_string(second), Some(&b"hi"[..]));
        let (third, rest) = der::next(rest).unwrap();
        assert_eq!(der::oid(third), Some(der::OID_DES_EDE3_CBC));
        assert!(rest.is_empty());

        // A length that runs past the end is not a value.
        assert_eq!(der::sequence(&[0x30, 0x7f, 0x02]), None);
        assert_eq!(der::sequence(&[0x04, 0x01, 0x00]), None, "not a sequence");
    }

    #[test]
    fn a_long_length_is_read() {
        let mut bytes = vec![0x04, 0x82, 0x01, 0x00];
        bytes.extend(std::iter::repeat_n(7u8, 256));
        assert_eq!(der::octet_string(&bytes).map(|v| v.len()), Some(256));
    }
}
