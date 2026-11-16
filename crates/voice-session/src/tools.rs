//! Tool execution boundary. The first release's app registers no tools ([`NoTools`]);
//! tests and load runs supply fixture executors. A tool's result is data: it can only be
//! recorded and spoken, never turned into another action.

use std::future::Future;
use voice_domain::proposal::Action;
use voice_domain::tools::ToolOutcome;

pub trait ToolExecutor: Send + Sync + 'static {
    fn execute(&self, action: Action) -> impl Future<Output = ToolOutcome> + Send;
}

/// The first release: no tools exist, so nothing can run.
pub struct NoTools;

impl ToolExecutor for NoTools {
    fn execute(&self, _action: Action) -> impl Future<Output = ToolOutcome> + Send {
        std::future::ready(ToolOutcome::Failed {
            code: "no_tools".into(),
        })
    }
}
