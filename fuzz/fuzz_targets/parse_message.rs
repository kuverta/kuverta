//! Every message synced from a server goes through here first, and then on to
//! deduplication and the classifier.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(parsed) = core_proto::parse::parse_message(data, Some(data.len() as i64)) {
        let _ = core_store::dedup_key(&parsed.message);
        let _ = parsed.facts.as_message_facts();
    }
});
