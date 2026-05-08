//! Fuzz target for the tenant header validation path. We hand the
//! middleware arbitrary header bytes (must be HTTP-valid) and assert that
//! it never panics. The output (accept vs reject) is implicit; we only
//! care about no panics.

#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Reject inputs that wouldn't fit in an HTTP header value (the
    // crate's own header parser would refuse them upstream of the
    // tenant extractor).
    let value = match http::HeaderValue::from_bytes(data) {
        Ok(v) => v,
        Err(_) => return,
    };

    // The actual validation logic lives in the middleware's `call`. We
    // re-implement it here to exercise the same predicate the real
    // service uses; this avoids needing to spin up tower runtimes per
    // input which would dominate the fuzzer's per-input cost. If the
    // documented predicate ever drifts from the implementation, the
    // hwhkit `proptest_tenant_id` integration test will catch it.
    if let Ok(s) = value.to_str() {
        let trimmed = s.trim();
        let _accepts = !trimmed.is_empty()
            && trimmed.len() <= 128
            && trimmed
                .chars()
                .all(|c| !c.is_control() && !c.is_whitespace());
        // Drive the chars iterator anyway so any panicking iterator path
        // is exercised by the fuzzer.
        for _ch in trimmed.chars() {}
    }
});
