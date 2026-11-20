//! The app's stdin framing under arbitrary bytes: line splitting, the line cap, the \r and
//! whitespace-only skips, and the JSON decode into `ClientMessage` (which carries `Proposal`)
//! — the never-panics minimum for the wire decoder. A panic here is a red-first bug.
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().build().unwrap())
}

fuzz_target!(|data: &[u8]| {
    rt().block_on(async {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        voice_session::framing::read_loop(data, tx, voice_session::framing::MAX_LINE).await;
        drain.await.unwrap();
    });
});
