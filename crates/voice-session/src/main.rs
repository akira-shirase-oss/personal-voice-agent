//! `voice-session --data-dir <dir>`: the app's child process. Speaks JSON lines on
//! stdin/stdout (docs/PROTOCOL.md) and logs content-free lines to stderr.
//!
//! Exit codes: 0 session ended, 2 usage, 3 ledger damaged, 4 ledger I/O, 5 runtime.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use voice_session::log::Log;
use voice_session::store::StoreError;
use voice_session::tools::NoTools;
use voice_session::{Config, ServiceError, run};

fn usage() -> ExitCode {
    eprintln!("usage: voice-session --data-dir <dir>");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut data_dir: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => data_dir = args.next().map(PathBuf::from),
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
    let config = Config::new(data_dir, NoTools, log);
    let result = runtime.block_on(run(tokio::io::stdin(), tokio::io::stdout(), config));
    // Tokio's stdin read sits on a blocking thread; do not wait for it at exit.
    runtime.shutdown_timeout(Duration::from_millis(100));
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(ServiceError::Store(StoreError::Corrupt { .. })) => ExitCode::from(3),
        Err(ServiceError::Store(StoreError::Io { .. })) => ExitCode::from(4),
        Err(ServiceError::Join) => ExitCode::from(5),
    }
}
