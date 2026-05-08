//! Fuzz target for the cron parser. Goal: arbitrary bytes never panic;
//! the parser only ever returns `Ok(spec)` or `Err(Error::Cron(_))`.

#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // We don't care what the parser returns — we only care that it
        // doesn't panic.
        let _ = hwhkit_scheduler::cron::parse(s);
    }
});
