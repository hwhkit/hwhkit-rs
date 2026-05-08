//! Compile-time build information for services built on hwhkit.
//!
//! Values are populated by this crate's `build.rs` from git + rustc. Override
//! at build time with `HWHKIT_GIT_SHA` and `HWHKIT_BUILD_TIME` environment
//! variables (useful for reproducible builds in CI).

use serde::Serialize;

/// Short git commit hash (or `"unknown"` if not in a git checkout).
pub const GIT_SHA: &str = env!("HWHKIT_GIT_SHA");
/// Build time as a Unix epoch second string. Stringly to keep the build
/// script trivially copy-pastable across platforms.
pub const BUILD_TIME_UNIX: &str = env!("HWHKIT_BUILD_TIME_UNIX");
/// Output of `rustc -V` at build time.
pub const RUST_VERSION: &str = env!("HWHKIT_RUST_VERSION");

#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct BuildInfo {
    pub git_sha: &'static str,
    pub build_time_unix: &'static str,
    pub rust_version: &'static str,
    /// Crate version of the consuming service (read at runtime via macro
    /// when this crate is compiled into the consumer; defaults to the
    /// hwhkit-buildinfo crate version).
    pub cargo_version: &'static str,
}

impl BuildInfo {
    /// Construct a [`BuildInfo`] from explicit fields. Hidden helper used by
    /// the [`build_info!`] macro so the macro stays compatible with the
    /// `#[non_exhaustive]` attribute on the struct (struct expression
    /// construction is forbidden across crate boundaries on
    /// `#[non_exhaustive]` types). Callers should generally use the macro
    /// or [`current`].
    #[doc(hidden)]
    pub const fn __from_parts(
        git_sha: &'static str,
        build_time_unix: &'static str,
        rust_version: &'static str,
        cargo_version: &'static str,
    ) -> Self {
        Self {
            git_sha,
            build_time_unix,
            rust_version,
            cargo_version,
        }
    }
}

/// Build a [`BuildInfo`] for the calling crate. Wires `CARGO_PKG_VERSION` of
/// the caller into `cargo_version`.
#[macro_export]
macro_rules! build_info {
    () => {
        $crate::BuildInfo::__from_parts(
            $crate::GIT_SHA,
            $crate::BUILD_TIME_UNIX,
            $crate::RUST_VERSION,
            env!("CARGO_PKG_VERSION"),
        )
    };
}

/// Returns a [`BuildInfo`] keyed to the hwhkit-buildinfo crate version
/// (use the `build_info!()` macro for caller-crate version).
pub fn current() -> BuildInfo {
    BuildInfo::__from_parts(
        GIT_SHA,
        BUILD_TIME_UNIX,
        RUST_VERSION,
        env!("CARGO_PKG_VERSION"),
    )
}
