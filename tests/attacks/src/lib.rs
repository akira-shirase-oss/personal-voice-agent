//! Attack lab for `voice-session`. The tests live in `tests/` and drive the real session
//! binary over stdin/stdout pipes — the model is `labs/minibtc/tests/attacks`: one automated
//! test per attack path, each asserting the refusal or the bounded end. The cargo-fuzz
//! targets live beside this crate in `fuzz/` as their own package (cargo-fuzz requires it).
