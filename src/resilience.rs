//! Resilience guards for DuckDB reads.
//!
//! The Orbit graph is an on-disk DuckDB file that may be large, stale, or
//! adversarially crafted. The audit flagged that our reads had **no timeouts
//! and no query size guards**: a pathological table could make a full-scan
//! query hang or balloon memory indefinitely.
//!
//! This module closes that gap using the shared `resilient-call` crate:
//!
//! - [`MAX_ROWS`] is the query size guard — every unbounded scan should be
//!   capped with a `LIMIT` so a runaway table cannot exhaust memory.
//! - [`with_read_timeout`] enforces a real wall-clock deadline on a blocking
//!   DuckDB read. The deadline itself is driven by
//!   [`resilient_call::with_timeout`] on a minimal current-thread runtime; a
//!   watchdog cancels the in-flight query via DuckDB's `InterruptHandle` if the
//!   deadline elapses, since a blocking native call would never yield on its
//!   own.

use anyhow::Result;
use duckdb::InterruptHandle;
use resilient_call::{with_timeout, ResilienceError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Upper bound on rows materialized by any single graph scan. Acts as the
/// query size guard the audit asked for: queries append `LIMIT {MAX_ROWS}` so
/// a pathologically large table cannot exhaust memory. Sized well above any
/// realistic Orbit graph.
pub const MAX_ROWS: usize = 5_000_000;

/// Wall-clock deadline for a single DuckDB read.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Run a blocking DuckDB read `op` under a wall-clock [`READ_TIMEOUT`].
///
/// `op` is the existing synchronous query closure (it borrows the connection
/// and runs on the calling thread, so DuckDB's single-threaded usage is
/// preserved). `interrupt` is the connection's [`InterruptHandle`]
/// (`conn.interrupt_handle()`), used by an internal watchdog to cancel the
/// in-flight query if the deadline elapses.
///
/// On success the closure's value is returned unchanged. On timeout the query
/// is interrupted and surfaces a DuckDB error, which is mapped to a clear
/// timeout error so the caller fails fast instead of hanging.
pub fn with_read_timeout<T, F>(interrupt: Arc<InterruptHandle>, op: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    // Watchdog: the async `with_timeout` guard owns the deadline. When it
    // fires we flip `fired` and interrupt the connection; the blocking `op` on
    // the calling thread then returns a DuckDB interruption error promptly.
    let fired = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));

    let watchdog = {
        let fired = Arc::clone(&fired);
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => return,
            };
            let _ = rt.block_on(with_timeout::<(), std::convert::Infallible, _>(
                async {
                    // Resolve early (cleanly) if the read finishes first.
                    loop {
                        if done.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                },
                READ_TIMEOUT,
            ))
            .map_err(|e| {
                if matches!(e, ResilienceError::Timeout(_)) {
                    fired.store(true, Ordering::SeqCst);
                    interrupt.interrupt();
                }
            });
        })
    };

    // Run the read on this thread, preserving DuckDB's single-threaded model.
    let result = op();

    // Signal completion and let the watchdog wind down.
    done.store(true, Ordering::SeqCst);
    let _ = watchdog.join();

    if fired.load(Ordering::SeqCst) {
        return Err(anyhow::anyhow!(
            "DuckDB read exceeded {READ_TIMEOUT:?} timeout and was interrupted; \
             the Orbit graph may be too large or the query unbounded"
        ));
    }

    result
}
