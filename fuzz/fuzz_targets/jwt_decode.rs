//! Fuzz target for the JWT verifier path. We feed arbitrary base64-ish
//! bytes to `JwtVerifier::verify` (HMAC mode, well-known secret). Goal:
//! malformed tokens always surface as `JwtError`, never as a panic.

#![no_main]
use libfuzzer_sys::fuzz_target;

use hwhkit_core::jwt::{JwtVerifier, JwtVerifierConfig};

fuzz_target!(|data: &[u8]| {
    // Best-effort UTF-8 conversion — JWTs are always ASCII, but the
    // fuzzer doesn't know that, so we silently drop non-UTF-8 inputs.
    let token = match std::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return,
    };

    let cfg = JwtVerifierConfig {
        algorithms: vec!["HS256".to_string()],
        ..Default::default()
    };
    let verifier = JwtVerifier::from_hmac(cfg, b"fuzz-secret".to_vec());

    // Block on a tiny single-thread runtime — `verify` is async but
    // doesn't await any I/O in HMAC mode.
    let rt = match tokio::runtime::Builder::new_current_thread().build() {
        Ok(r) => r,
        Err(_) => return,
    };
    rt.block_on(async {
        // We don't care which error variant comes back — only that we
        // get a `Result::Err` rather than a panic.
        let _: Result<serde_json::Value, _> = verifier.verify(token).await;
    });
});
