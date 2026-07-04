//! Keys, PGP/MIME both ways, and inline PGP — against real keys and real
//! built messages, with passphrases held in memory so no keychain is asked.

use std::sync::Arc;

use core_pgp::{
    open_message, protect, Keyring, MemoryPassphrases, PgpError, Protection, SignatureState,
};
use core_smtp::{Draft, Mailbox};
use mail_parser::{MessageParser, MimeHeaders};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kuverta-pgp-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const ERIKA: &str = "erika@example.com";
const MAX: &str = "max@example.org";

fn keyring(dir: &TempDir, who: &str) -> Keyring {
    Keyring::new(dir.0.join(who), Arc::new(MemoryPassphrases::default()))
}

/// Erika and Max, each with a key pair of their own and the other's
/// certificate.
struct Pair {
    erika: Keyring,
    max: Keyring,
    erika_fpr: String,
    max_fpr: String,
    _dir: TempDir,
}

fn pair(name: &str) -> Pair {
    let dir = TempDir::new(name);
    let erika = keyring(&dir, "erika");
    let max = keyring(&dir, "max");
    let erika_key = erika
        .generate("Erika Mustermann", ERIKA, Some("correct horse"))
        .unwrap();
    let max_key = max.generate("Max Mustermann", MAX, None).unwrap();

    let report = max
        .import(&erika.export_public(&erika_key.fingerprint).unwrap())
        .unwrap();
    assert_eq!(report.imported.len(), 1, "{report:?}");
    erika
        .import(&max.export_public(&max_key.fingerprint).unwrap())
        .unwrap();

    Pair {
        erika,
        max,
        erika_fpr: erika_key.fingerprint,
        max_fpr: max_key.fingerprint,
        _dir: dir,
    }
}

fn letter(body: &str) -> core_smtp::BuiltMessage {
    Draft::new(Mailbox::named("Erika Mustermann", ERIKA))
        .to(Mailbox::named("Max Mustermann", MAX))
        .subject("Rechnung März")
        .body(body)
        .build()
        .unwrap()
}

fn content_type(raw: &[u8]) -> (String, String) {
    let parsed = MessageParser::default().parse(raw).unwrap();
    let ct = parsed.root_part().content_type().unwrap();
    (
        format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or_default()),
        ct.attribute("protocol").unwrap_or_default().to_string(),
    )
}

#[test]
fn a_generated_key_is_listed_with_what_it_can_do() {
    let dir = TempDir::new("generate");
    let ring = keyring(&dir, "erika");
    let key = ring
        .generate("Erika Mustermann", ERIKA, Some("correct horse"))
        .unwrap();

    assert_eq!(key.fingerprint.len(), 40);
    assert_eq!(key.emails, vec![ERIKA]);
    assert_eq!(key.user_ids, vec!["Erika Mustermann <erika@example.com>"]);
    assert!(key.can_encrypt && key.can_sign && key.has_secret && key.has_passphrase);
    assert!(key.expires.is_none() && !key.expired && !key.revoked);

    let listed = ring.keys().unwrap();
    assert_eq!(listed, vec![key.clone()]);
    assert_eq!(
        ring.key_for_email("Erika <ERIKA@example.com>")
            .unwrap()
            .map(|k| k.fingerprint),
        Some(key.fingerprint.clone())
    );

    // The secret key on disk is protected, never in the clear.
    let secret =
        std::fs::read_to_string(ring.dir().join(format!("{}.secret.asc", key.fingerprint)))
            .unwrap();
    assert!(secret.contains("PRIVATE KEY BLOCK"));
    let (parsed, _) =
        <pgp::composed::SignedSecretKey as pgp::composed::Deserializable>::from_string(&secret)
            .unwrap();
    assert!(parsed.primary_key.secret_params().is_encrypted());

    ring.delete(&key.fingerprint).unwrap();
    assert!(ring.keys().unwrap().is_empty());
}

#[test]
fn a_passphrase_is_checked_before_it_is_stored() {
    let dir = TempDir::new("passphrase");
    let passphrases = Arc::new(MemoryPassphrases::default());
    let ring = Keyring::new(dir.0.join("k"), passphrases.clone());
    let key = ring.generate("", ERIKA, Some("right")).unwrap();

    assert!(matches!(
        ring.set_passphrase(&key.fingerprint, "wrong"),
        Err(PgpError::WrongPassphrase(_))
    ));
    ring.set_passphrase(&key.fingerprint, "right").unwrap();
}

#[test]
fn a_signed_message_verifies_and_keeps_its_headers() {
    let p = pair("sign");
    let built = letter("Hallo Max,\nFrom now on trailing space \nGrüße\n");
    let signed = protect(
        &p.erika,
        &built.rfc822,
        ERIKA,
        &[MAX.to_string()],
        &[],
        Protection {
            sign: true,
            encrypt: false,
        },
    )
    .unwrap();

    let (ct, protocol) = content_type(&signed);
    assert_eq!(ct, "multipart/signed");
    assert_eq!(protocol, "application/pgp-signature");
    let text = String::from_utf8(signed.clone()).unwrap();
    assert!(text.contains("micalg=pgp-sha256"));
    assert!(text.contains(&format!("Message-ID: <{}>", built.message_id)));
    // The signed part is 7-bit and immune to the classic relay rewrites.
    assert!(text.is_ascii());
    assert!(text.contains("=46rom now on"));
    assert!(text.contains("trailing space=20\r\n"));

    let parsed = MessageParser::default().parse(&signed).unwrap();
    assert_eq!(parsed.subject(), Some("Rechnung März"));
    assert_eq!(parsed.parts[0].sub_parts().unwrap().len(), 2);

    let opened = open_message(&signed, Some(&p.max));
    let security = opened.security.expect("signed");
    assert_eq!(security.format, "pgp/mime");
    assert!(security.signed && !security.encrypted);
    let signature = security.signature.clone().expect("a signature");
    assert_eq!(signature.state, SignatureState::Valid, "{security:?}");
    assert_eq!(signature.fingerprint.as_deref(), Some(p.erika_fpr.as_str()));
    assert_eq!(
        signature.signer.as_deref(),
        Some("Erika Mustermann <erika@example.com>")
    );
    assert_eq!(signature.signer_matches_sender, Some(true));
    assert_eq!(
        opened.body_text.as_deref().map(|t| t.replace("\r\n", "\n")),
        Some("Hallo Max,\nFrom now on trailing space \nGrüße\n".to_string())
    );
}

#[test]
fn a_tampered_signed_message_is_reported_invalid() {
    let p = pair("tamper");
    let built = letter("Bitte überweise 100 Euro.\n");
    let signed = protect(
        &p.erika,
        &built.rfc822,
        ERIKA,
        &[MAX.to_string()],
        &[],
        Protection {
            sign: true,
            encrypt: false,
        },
    )
    .unwrap();
    let text = String::from_utf8(signed).unwrap();
    assert!(text.contains("100 Euro"));
    let tampered = text.replace("100 Euro", "900 Euro");

    let security = open_message(tampered.as_bytes(), Some(&p.max))
        .security
        .unwrap();
    assert_eq!(security.signature.unwrap().state, SignatureState::Invalid);

    // A relay rewriting line endings alone must not break it.
    let lf = text.replace("\r\n", "\n");
    let security = open_message(lf.as_bytes(), Some(&p.max)).security.unwrap();
    assert_eq!(security.signature.unwrap().state, SignatureState::Valid);
}

#[test]
fn a_signature_by_an_unknown_key_says_so() {
    let p = pair("unknown");
    let dir = TempDir::new("stranger");
    let stranger = keyring(&dir, "stranger");
    let built = letter("hi\n");
    let signed = protect(
        &p.erika,
        &built.rfc822,
        ERIKA,
        &[MAX.to_string()],
        &[],
        Protection {
            sign: true,
            encrypt: false,
        },
    )
    .unwrap();

    let signature = open_message(&signed, Some(&stranger))
        .security
        .unwrap()
        .signature
        .unwrap();
    assert_eq!(signature.state, SignatureState::UnknownKey);
    assert!(signature.key_id.is_some());
}

#[test]
fn an_encrypted_message_round_trips_for_the_recipient_and_the_sender() {
    let p = pair("encrypt");
    let built = letter("Das Passwort ist geheim.\n");
    let encrypted = protect(
        &p.erika,
        &built.rfc822,
        ERIKA,
        &[format!("Max Mustermann <{MAX}>")],
        &[],
        Protection {
            sign: true,
            encrypt: true,
        },
    )
    .unwrap();

    let (ct, protocol) = content_type(&encrypted);
    assert_eq!(ct, "multipart/encrypted");
    assert_eq!(protocol, "application/pgp-encrypted");
    let text = String::from_utf8(encrypted.clone()).unwrap();
    assert!(!text.contains("Passwort"), "the body must not leak");
    assert!(text.contains("-----BEGIN PGP MESSAGE-----"));
    assert!(text.contains("Version: 1"));
    let parsed = MessageParser::default().parse(&encrypted).unwrap();
    assert_eq!(parsed.subject(), Some("Rechnung März"));
    assert_eq!(parsed.message_id(), Some(built.message_id.as_str()));
    assert_eq!(parsed.to().unwrap().first().unwrap().address(), Some(MAX));
    let parts = parsed.parts[0].sub_parts().unwrap();
    assert_eq!(parts.len(), 2);
    let version = parsed.part(parts[0]).unwrap().content_type().unwrap();
    assert_eq!(version.subtype(), Some("pgp-encrypted"));

    for (who, ring) in [("max", &p.max), ("erika", &p.erika)] {
        let opened = open_message(&encrypted, Some(ring));
        let security = opened.security.expect("encrypted");
        assert!(
            security.encrypted && security.decrypted,
            "{who}: {security:?}"
        );
        assert!(security.signed, "{who}");
        assert_eq!(
            security.signature.as_ref().unwrap().state,
            SignatureState::Valid,
            "{who}"
        );
        assert_eq!(security.encrypted_to.len(), 2, "recipient and sender");
        assert_eq!(
            opened.body_text.as_deref().map(|t| t.replace("\r\n", "\n")),
            Some("Das Passwort ist geheim.\n".to_string()),
            "{who}"
        );
    }
    let _ = &p.max_fpr;
}

#[test]
fn encryption_without_a_stored_passphrase_says_which_key() {
    let p = pair("no-passphrase");
    let built = letter("x\n");
    let encrypted = protect(
        &p.erika,
        &built.rfc822,
        ERIKA,
        &[MAX.to_string()],
        &[],
        Protection {
            sign: false,
            encrypt: true,
        },
    )
    .unwrap();

    // Max's key as it would be on another computer: the files, but no
    // passphrase in that computer's keychain.
    let elsewhere = Keyring::new(p.max.dir(), Arc::new(MemoryPassphrases::default()));
    let security = open_message(&encrypted, Some(&elsewhere)).security.unwrap();
    assert!(security.encrypted && !security.decrypted);
    let error = security.error.unwrap();
    assert!(error.contains("no passphrase"), "{error}");
    assert!(error.contains(&p.max_fpr), "{error}");

    // And a keyring with no secret key at all.
    let dir = TempDir::new("nobody");
    let nobody = keyring(&dir, "nobody");
    let error = open_message(&encrypted, Some(&nobody))
        .security
        .unwrap()
        .error
        .unwrap();
    assert!(error.contains("no secret key"), "{error}");
}

#[test]
fn bcc_and_encryption_are_refused_together() {
    let p = pair("bcc");
    let built = letter("x\n");
    let err = protect(
        &p.erika,
        &built.rfc822,
        ERIKA,
        &[MAX.to_string()],
        &["hidden@example.net".to_string()],
        Protection {
            sign: false,
            encrypt: true,
        },
    )
    .unwrap_err();
    assert!(matches!(err, PgpError::BccWithEncryption));
    assert!(err.to_string().contains("Bcc"));

    // Signing alone does not disclose anyone, so it is not refused.
    protect(
        &p.erika,
        &built.rfc822,
        ERIKA,
        &[MAX.to_string()],
        &["hidden@example.net".to_string()],
        Protection {
            sign: true,
            encrypt: false,
        },
    )
    .unwrap();
}

#[test]
fn a_recipient_without_a_key_is_named() {
    let p = pair("missing");
    let built = letter("x\n");
    let err = protect(
        &p.erika,
        &built.rfc822,
        ERIKA,
        &[MAX.to_string(), "Anna <anna@example.net>".to_string()],
        &[],
        Protection {
            sign: false,
            encrypt: true,
        },
    )
    .unwrap_err();
    match &err {
        PgpError::NoKeyFor(missing) => assert_eq!(missing, &vec!["anna@example.net".to_string()]),
        other => panic!("expected NoKeyFor, got {other:?}"),
    }
    assert!(err.to_string().contains("anna@example.net"));

    let status = p
        .erika
        .recipients(ERIKA, &[MAX.to_string(), "anna@example.net".into()])
        .unwrap();
    assert!(status.can_sign);
    assert!(!status.can_encrypt);
    assert_eq!(status.missing, vec!["anna@example.net"]);
    assert_eq!(
        status.recipients[0].fingerprint.as_deref(),
        Some(p.max_fpr.as_str())
    );
    assert_eq!(status.sender_key.as_deref(), Some(p.erika_fpr.as_str()));

    let ok = p.erika.recipients(ERIKA, &[MAX.to_string()]).unwrap();
    assert!(ok.can_encrypt && ok.missing.is_empty());
}

#[test]
fn inline_pgp_is_decrypted_and_verified() {
    use pgp::composed::{
        ArmorOptions, CleartextSignedMessage, Deserializable, SignedPublicKey, SignedSecretKey,
    };

    let p = pair("inline");
    let max_cert = SignedPublicKey::from_string(&p.erika.export_public(&p.max_fpr).unwrap())
        .unwrap()
        .0;
    let erika_secret = SignedSecretKey::from_string(
        &std::fs::read_to_string(p.erika.dir().join(format!("{}.secret.asc", p.erika_fpr)))
            .unwrap(),
    )
    .unwrap()
    .0;

    // An encrypted inline message, as older clients write it.
    let mut rng = rand::thread_rng();
    let mut builder = pgp::composed::MessageBuilder::from_bytes("", &b"Treffen um drei."[..])
        .seipd_v1(&mut rng, pgp::crypto::sym::SymmetricKeyAlgorithm::AES128);
    builder
        .encrypt_to_key(&mut rng, &max_cert.public_subkeys[0])
        .unwrap();
    let armored = builder
        .to_armored_string(&mut rng, ArmorOptions::default())
        .unwrap();
    let raw = format!(
        "From: Erika Mustermann <{ERIKA}>\r\nTo: {MAX}\r\nSubject: inline\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\nHallo,\r\n\r\n{}\r\n",
        armored.replace('\n', "\r\n")
    );
    let opened = open_message(raw.as_bytes(), Some(&p.max));
    let security = opened.security.unwrap();
    assert_eq!(security.format, "inline");
    assert!(security.encrypted && security.decrypted, "{security:?}");
    assert!(!security.signed);
    let body = opened.body_text.unwrap();
    assert!(body.contains("Hallo,"), "{body}");
    assert!(body.contains("Treffen um drei."), "{body}");
    assert!(!body.contains("BEGIN PGP"), "{body}");

    // A cleartext signature, and the same one tampered with.
    let signed = CleartextSignedMessage::sign(
        &mut rng,
        "Gezeichnet: Erika\n",
        &erika_secret.primary_key,
        &"correct horse".into(),
    )
    .unwrap()
    .to_armored_string(ArmorOptions::default())
    .unwrap();
    let raw = format!(
        "From: {ERIKA}\r\nTo: {MAX}\r\nSubject: signed\r\n\r\n{}\r\n",
        signed.replace('\n', "\r\n")
    );
    let opened = open_message(raw.as_bytes(), Some(&p.max));
    let security = opened.security.unwrap();
    assert!(security.signed && !security.encrypted);
    assert_eq!(security.signature.unwrap().state, SignatureState::Valid);
    assert!(opened.body_text.unwrap().contains("Gezeichnet: Erika"));

    let forged = raw.replace("Gezeichnet: Erika", "Gezeichnet: Mallory");
    let security = open_message(forged.as_bytes(), Some(&p.max))
        .security
        .unwrap();
    assert_eq!(security.signature.unwrap().state, SignatureState::Invalid);
}

#[test]
fn ordinary_mail_has_no_security_to_report() {
    let dir = TempDir::new("plain");
    let ring = keyring(&dir, "k");
    let built = letter("nothing to see\n");
    let opened = open_message(&built.rfc822, Some(&ring));
    assert!(opened.security.is_none());
    assert!(opened.body_text.is_none());
    assert!(opened.pgp_keys.is_empty());
}

#[test]
fn attached_keys_are_offered_for_import() {
    let p = pair("attached");
    let armored = p.erika.export_public(&p.erika_fpr).unwrap();
    let raw = format!(
        "From: {ERIKA}\r\nTo: {MAX}\r\nSubject: my key\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\nhere it is\r\n\
         --b\r\nContent-Type: application/pgp-keys; name=\"key.asc\"\r\n\
         Content-Disposition: attachment; filename=\"key.asc\"\r\n\r\n{}\r\n--b--\r\n",
        armored.replace('\n', "\r\n")
    );
    let opened = open_message(raw.as_bytes(), None);
    assert_eq!(opened.pgp_keys.len(), 1);

    let dir = TempDir::new("attached-into");
    let fresh = keyring(&dir, "fresh");
    let report = fresh.import(&opened.pgp_keys[0]).unwrap();
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.imported[0].fingerprint, p.erika_fpr);
    assert!(!report.imported[0].secret);
}

#[test]
fn import_takes_several_blocks_and_protects_a_bare_secret_key() {
    use pgp::composed::{
        ArmorOptions, EncryptionCaps, KeyType, SecretKeyParamsBuilder, SubkeyParamsBuilder,
    };

    // A secret key with no passphrase, as some tools export them.
    let bare = SecretKeyParamsBuilder::default()
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(true)
        .primary_user_id("Anna <anna@example.net>".into())
        .subkeys(vec![SubkeyParamsBuilder::default()
            .key_type(KeyType::ECDH(
                pgp::crypto::ecc_curve::ECCCurve::Curve25519Legacy,
            ))
            .can_encrypt(EncryptionCaps::All)
            .build()
            .unwrap()])
        .build()
        .unwrap()
        .generate(rand::thread_rng())
        .unwrap();
    let p = pair("import");
    let text = format!(
        "Here are two keys:\n\n{}\n\n{}\n",
        bare.to_armored_string(ArmorOptions::default()).unwrap(),
        p.max.export_public(&p.max_fpr).unwrap()
    );

    let dir = TempDir::new("import-into");
    let ring = keyring(&dir, "ring");
    let report = ring.import(&text).unwrap();
    assert!(report.errors.is_empty(), "{report:?}");
    assert_eq!(report.imported.len(), 2);
    let anna = report.imported.iter().find(|k| k.secret).unwrap();
    assert!(
        anna.has_passphrase,
        "a bare key is given a passphrase of its own"
    );
    assert!(!anna.updated);

    let secret =
        std::fs::read_to_string(ring.dir().join(format!("{}.secret.asc", anna.fingerprint)))
            .unwrap();
    let (parsed, _) =
        <pgp::composed::SignedSecretKey as pgp::composed::Deserializable>::from_string(&secret)
            .unwrap();
    assert!(parsed.primary_key.secret_params().is_encrypted());

    let again = ring.import(&text).unwrap();
    assert!(again.imported.iter().all(|k| k.updated));

    let nonsense = ring.import("not a key at all").unwrap();
    assert!(nonsense.imported.is_empty());
    assert_eq!(nonsense.errors.len(), 1);
}

#[test]
fn encrypting_without_a_key_of_your_own_says_so() {
    // The copy in Sent is encrypted to the sender too; without a key of
    // their own they could never read it again.
    let p = pair("own-key");
    let dir = TempDir::new("own-key-sender");
    let only_max = keyring(&dir, "only-max");
    only_max
        .import(&p.max.export_public(&p.max_fpr).unwrap())
        .unwrap();
    let built = letter("x\n");
    let err = protect(
        &only_max,
        &built.rfc822,
        ERIKA,
        &[MAX.to_string()],
        &[],
        Protection {
            sign: false,
            encrypt: true,
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, PgpError::NoOwnKey(ref who) if who == ERIKA),
        "{err}"
    );
}
