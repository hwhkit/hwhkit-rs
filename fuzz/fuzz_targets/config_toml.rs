//! Fuzz target for the TOML config loader. Feeds arbitrary bytes through
//! `toml::from_str` (the same path used by `read_toml_patch`) and asserts
//! that no input causes a panic — only `Result::Err`.

#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // The production loader uses `toml::from_str::<toml::Value>` and
        // then converts to `serde_json::Value`. We exercise both legs so
        // the fuzzer reaches the same code paths as the loader.
        if let Ok(parsed) = toml::from_str::<toml::Value>(s) {
            let _ = serde_json::to_value(parsed);
        }
    }
});
