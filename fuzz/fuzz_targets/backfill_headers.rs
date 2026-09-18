//! Stored raw mail read back for headers, with its own header/body split.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = core_proto::parse::backfill_headers(data);
});
