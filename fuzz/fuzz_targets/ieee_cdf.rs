//! Malformed-input fuzzing of the IEEE Common Data Format reader.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // CDF is a fixed-column text format, but arbitrary bytes are intentional:
    // invalid encodings and truncated records must fail as structured errors,
    // not panic while the reader walks field boundaries.
    let Ok(source) = powerio_core::Source::from_memory("input.cdf", data.to_vec()) else {
        return;
    };
    let Ok(format) = powerio_core::FormatId::new("ieee-cdf") else {
        return;
    };
    let _ = powerio::parse(source.with_format(format));
});
