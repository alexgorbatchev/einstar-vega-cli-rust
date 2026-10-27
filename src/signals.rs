use einstar_vega::CancellationToken;
use std::io;

/// Native handlers are registered before an operation announces readiness.
pub struct Listener {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(windows)]
    interrupt: tokio::signal::windows::CtrlC,
}

impl Listener {
    /// Registers handlers on the current Tokio runtime.
    pub fn new() -> io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                interrupt: signal(SignalKind::interrupt())?,
                terminate: signal(SignalKind::terminate())?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                interrupt: tokio::signal::windows::ctrl_c()?,
            })
        }
    }

    /// Observes cancellation, Ctrl-C, or Unix SIGTERM and then cancels the token.
    pub async fn wait(mut self, token: CancellationToken) -> io::Result<()> {
        #[cfg(unix)]
        let result = tokio::select! { biased;
            _ = token.cancelled() => Ok(()),
            result = self.interrupt.recv() => received(result),
            result = self.terminate.recv() => received(result),
        };
        #[cfg(windows)]
        let result = tokio::select! { biased; _ = token.cancelled() => Ok(()), result = self.interrupt.recv() => received(result) };
        token.cancel();
        result
    }
}

fn received(result: Option<()>) -> io::Result<()> {
    result
        .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "interrupt signal stream closed"))
}
