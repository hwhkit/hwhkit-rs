//! Process-level Prometheus metrics powered by `sysinfo`.
//!
//! Samples the running process every [`SAMPLE_INTERVAL`] and updates
//! gauges/counter exposed via the existing `/metrics` recorder:
//!
//! - `process_resident_memory_bytes` (gauge)
//! - `process_virtual_memory_bytes` (gauge)
//! - `process_cpu_seconds_total` (counter — monotonic seconds of CPU time)
//! - `process_open_fds` (gauge; Linux/macOS only — skipped on Windows)
//! - `process_threads` (gauge)
//!
//! The sampler runs as a tokio task and observes [`hwhkit_core::ShutdownToken`]
//! so it joins cleanly when the server drains.

use std::time::Duration;

use hwhkit_core::ShutdownToken;
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// Spawn the background sampler. Idempotent — calling twice will spawn a
/// second task; metrics readings are last-write-wins, so this is harmless
/// but wasteful. The bootstrap path calls this exactly once.
pub fn spawn(shutdown: ShutdownToken) {
    tokio::spawn(async move {
        run(shutdown).await;
    });
}

async fn run(shutdown: ShutdownToken) {
    let pid = match get_self_pid() {
        Some(p) => p,
        None => {
            tracing::warn!("process_metrics: unable to determine current pid; sampler disabled");
            return;
        }
    };

    let refresh = RefreshKind::new().with_processes(ProcessRefreshKind::everything());
    let mut sys = System::new_with_specifics(refresh);
    let mut state = SamplerState::default();

    loop {
        sample_once(&mut sys, refresh, pid, &mut state);

        tokio::select! {
            _ = tokio::time::sleep(SAMPLE_INTERVAL) => {}
            _ = shutdown.cancelled() => {
                tracing::debug!("process_metrics sampler stopping");
                // N9: take one final sample on the way out so the last
                // window of CPU/mem isn't lost — graceful shutdown logs
                // are otherwise the period most worth watching.
                sample_once(&mut sys, refresh, pid, &mut state);
                break;
            }
        }
    }
}

#[derive(Default)]
struct SamplerState {
    /// Total CPU milliseconds emitted to Prometheus so far.
    cpu_total_ms: u64,
    /// Sub-millisecond CPU fraction that hasn't yet been emitted. We
    /// accumulate until we've crossed at least 1ms of CPU then bump the
    /// counter — N10 fix for samples shorter than one ms going to /dev/null.
    cpu_residual_ms: f64,
}

fn sample_once(sys: &mut System, refresh: RefreshKind, pid: Pid, state: &mut SamplerState) {
    sys.refresh_specifics(refresh);
    let Some(proc) = sys.process(pid) else { return };

    let resident = proc.memory();
    let virt = proc.virtual_memory();
    metrics::gauge!("process_resident_memory_bytes").set(resident as f64);
    metrics::gauge!("process_virtual_memory_bytes").set(virt as f64);

    // CPU usage % over the sample window → integrate into a monotonic
    // counter. `cpu_usage` is normalised against a single core (0..=100*N
    // for an N-core box), so divide back out.
    #[allow(clippy::cast_possible_truncation)]
    let cpu_pct = proc.cpu_usage() as f64;
    let core_count = sys.cpus().len().max(1) as f64;
    let elapsed_ms = SAMPLE_INTERVAL.as_millis() as f64;
    // cpu_pct is the percent of a single core spent running this process
    // during the last refresh interval. Convert to seconds:
    // pct/100 * elapsed_secs gives core-seconds across all cores.
    let core_seconds_window = (cpu_pct / 100.0) * elapsed_ms / 1000.0 / core_count;
    let added_ms_f64 = core_seconds_window * 1000.0;
    state.cpu_residual_ms += added_ms_f64;
    if state.cpu_residual_ms >= 1.0 {
        let bumps = state.cpu_residual_ms.floor() as u64;
        state.cpu_total_ms = state.cpu_total_ms.saturating_add(bumps);
        state.cpu_residual_ms -= bumps as f64;
        metrics::counter!("process_cpu_seconds_total").absolute(state.cpu_total_ms / 1000);
    }

    #[cfg(target_os = "linux")]
    if let Some(fd_count) = read_open_fds_linux() {
        metrics::gauge!("process_open_fds").set(fd_count as f64);
    }
    #[cfg(target_os = "macos")]
    if let Some(fd_count) = read_open_fds_macos() {
        metrics::gauge!("process_open_fds").set(fd_count as f64);
    }

    // sysinfo exposes thread count via tasks() on Linux; fall back to
    // the OS thread count probed by the runtime.
    #[cfg(target_os = "linux")]
    if let Some(n) = read_thread_count_linux() {
        metrics::gauge!("process_threads").set(n as f64);
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Best-effort fallback: number of currently-active tokio workers
        // is not the same as OS threads, but it's the closest cheap
        // signal we have here.
        let n = std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1);
        metrics::gauge!("process_threads").set(n as f64);
    }
}

fn get_self_pid() -> Option<Pid> {
    // sysinfo's Pid is a thin wrapper around the platform pid type. The
    // crate-provided `current_pid()` shortcut.
    sysinfo::get_current_pid().ok()
}

#[cfg(target_os = "linux")]
fn read_open_fds_linux() -> Option<usize> {
    std::fs::read_dir("/proc/self/fd").ok().map(|it| it.count())
}

#[cfg(target_os = "macos")]
fn read_open_fds_macos() -> Option<usize> {
    // proc_pidinfo with PROC_PIDLISTFDS is the canonical way; spawning
    // `lsof` would be too heavy. Use the darwin-specific syscall via libc.
    use std::os::raw::c_int;
    extern "C" {
        fn proc_pidinfo(
            pid: c_int,
            flavor: c_int,
            arg: u64,
            buffer: *mut std::ffi::c_void,
            buffersize: c_int,
        ) -> c_int;
    }
    const PROC_PIDLISTFDS: c_int = 1;
    // First call with NULL buffer to obtain the required byte count.
    let pid = unsafe { libc::getpid() };
    let needed = unsafe { proc_pidinfo(pid, PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return None;
    }
    // The byte total is `count * size_of::<proc_fdinfo>`. Use libc's
    // canonical type size rather than a hard-coded magic constant —
    // safer if Apple ever extends the layout. (N11.)
    let entry_size = std::mem::size_of::<libc::proc_fdinfo>();
    if entry_size == 0 {
        return None;
    }
    Some(needed as usize / entry_size)
}

#[cfg(target_os = "linux")]
fn read_thread_count_linux() -> Option<usize> {
    std::fs::read_dir("/proc/self/task")
        .ok()
        .map(|it| it.count())
}
