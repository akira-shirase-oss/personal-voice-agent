//! The on-disk record decoder under arbitrary bytes: CRC field, JSON envelope, and the typed
//! `Record`/`ConsentRecord` decode — a tampered or torn log must be named, never panic the
//! store. A panic here is a red-first bug.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Both record types against the same bytes: the shared envelope checks plus each typed
    // payload decode.
    let _ = voice_session::store::decode_line::<voice_domain::record::Record>(data);
    let _ = voice_session::store::decode_line::<voice_domain::consent::ConsentRecord>(data);
});
