//! Opening a message that claims to be PGP: MIME, armor and packet parsing
//! of whatever the sender put there. Without a keyring nothing decrypts, but
//! every packet is still read to report what it was encrypted to or signed by.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = core_pgp::open_message(data, None);
});
