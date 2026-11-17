//! `voice-session --data-dir <dir>`: the app's child process. Speaks JSON lines on
//! stdin/stdout (docs/PROTOCOL.md) and logs content-free lines to stderr.
//!
//! `--retention-days N` deletes closed session logs older than N days at start (default 30;
//! 0 keeps none). `--limit name=value` (repeatable) overrides one field of `voice_domain::Limits`, e.g.
//! `--limit lm_first_ms=200` for drills and integration tests that need short deadlines.
//!
//! Exit codes: 0 session ended, 2 usage, 3 ledger damaged, 4 ledger I/O, 5 runtime,
//! 6 another service is already using the data directory.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use voice_session::log::Log;
use voice_session::store::StoreError;
use voice_session::tools::NoTools;
use voice_session::{Config, ServiceError, run};

fn usage() -> ExitCode {
    eprintln!("usage: voice-session --data-dir <dir> [--retention-days N] [--limit name=value]...");
    ExitCode::from(2)
}

/// Applies `name=value` to the limits through their serialized form, so every field is
/// settable by its own name and an unknown name or a bad value is refused, not ignored.
fn apply_limit(limits: &mut voice_domain::Limits, spec: &str) -> Result<(), String> {
    let (name, value) = spec
        .split_once('=')
        .ok_or_else(|| format!("expected name=value, got {spec}"))?;
    let mut fields = serde_json::to_value(&*limits).map_err(|e| e.to_string())?;
    let slot = fields.get_mut(name).ok_or_else(|| format!("unknown limit {name}"))?;
    *slot = serde_json::from_str(value).map_err(|_| format!("bad value for {name}: {value}"))?;
    *limits = serde_json::from_value(fields).map_err(|_| format!("bad value for {name}: {value}"))?;
    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut data_dir: Option<PathBuf> = None;
    let mut limits = voice_domain::Limits::default();
    let mut retention_days: Option<u64> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => data_dir = args.next().map(PathBuf::from),
            "--retention-days" => match args.next().and_then(|v| v.parse().ok()) {
                Some(days) => retention_days = Some(days),
                None => return usage(),
            },
            "--limit" => {
                let Some(spec) = args.next() else { return usage() };
                if let Err(e) = apply_limit(&mut limits, &spec) {
                    eprintln!("error: {e}");
                    return usage();
                }
            }
            "--version" => {
                println!(
                    "voice-session {} (protocol {})",
                    env!("CARGO_PKG_VERSION"),
                    voice_domain::protocol::PROTOCOL_VERSION
                );
                return ExitCode::SUCCESS;
            }
            _ => return usage(),
        }
    }
    let Some(data_dir) = data_dir else { return usage() };

    // One session at a time needs one thread; blocking work (store open, writer threads)
    // runs off it.
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        return ExitCode::from(5);
    };
    let log = Log::stderr();
    let mut config = Config::new(data_dir, NoTools, log);
    config.limits = limits;
    if let Some(days) = retention_days {
        config.retention = Some(Duration::from_secs(days.saturating_mul(86_400)));
    }
    let result = runtime.block_on(run(tokio::io::stdin(), tokio::io::stdout(), config));
    // Tokio's stdin read sits on a blocking thread; do not wait for it at exit.
    runtime.shutdown_timeout(Duration::from_millis(100));
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(ServiceError::Store(StoreError::Corrupt { .. })) => ExitCode::from(3),
        Err(ServiceError::Store(StoreError::Io { .. })) => ExitCode::from(4),
        Err(ServiceError::Store(StoreError::Locked { .. })) => ExitCode::from(6),
        Err(ServiceError::Join) => ExitCode::from(5),
    }
}

#[cfg(test)]
mod tests {
    use super::apply_limit;
    use voice_domain::Limits;

    #[test]
    fn limits_are_set_by_name_and_bad_specs_are_refused() {
        let mut l = Limits::default();
        apply_limit(&mut l, "lm_first_ms=200").unwrap();
        apply_limit(&mut l, "min_confidence=0.9").unwrap();
        assert_eq!((l.lm_first_ms, l.min_confidence), (200, 0.9));
        assert!(apply_limit(&mut l, "no_such=1").is_err());
        assert!(apply_limit(&mut l, "lm_first_ms=-5").is_err());
        assert!(apply_limit(&mut l, "lm_first_ms").is_err());
        assert_eq!(l.lm_first_ms, 200, "a refused spec changes nothing");
    }
}
