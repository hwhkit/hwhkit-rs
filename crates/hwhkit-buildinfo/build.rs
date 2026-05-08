use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Re-run when these change.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=HWHKIT_GIT_SHA");
    println!("cargo:rerun-if-env-changed=HWHKIT_BUILD_TIME");

    let git_sha = std::env::var("HWHKIT_GIT_SHA")
        .ok()
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short=12", "HEAD"])
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                    } else {
                        None
                    }
                })
        })
        .unwrap_or_else(|| "unknown".to_string());

    let build_time = std::env::var("HWHKIT_BUILD_TIME").unwrap_or_else(|_| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| format!("{}", d.as_secs()))
            .unwrap_or_else(|_| "0".to_string())
    });

    let rust_version = Command::new("rustc")
        .arg("-V")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=HWHKIT_GIT_SHA={}", git_sha);
    println!("cargo:rustc-env=HWHKIT_BUILD_TIME_UNIX={}", build_time);
    println!("cargo:rustc-env=HWHKIT_RUST_VERSION={}", rust_version);
}
