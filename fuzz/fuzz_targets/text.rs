//! Small string functions fed from outside: Message-IDs from headers, and a
//! model's transcript of a scanned letter.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = core_store::normalize_message_id(text);
    let (_, _) = core_ai::cut_repetition(text);
});
