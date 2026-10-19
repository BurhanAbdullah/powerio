//! Fuzz the separate PSS/E RAWX 35 JSON-table reader and its writer.
//!
//! RAWX has its own field-array and row-shape translation before electrical
//! conversion. On accepted inputs, exercise the stored-IR round trip as well;
//! if fresh RAWX emission succeeds, its output must be readable again.
#![no_main]

use libfuzzer_sys::fuzz_target;
use powerio::{Destination, EmittedOutput, Source};

fuzz_target!(|data: &[u8]| {
    let Ok(source) = Source::from_memory("fuzz.rawx", data.to_vec()) else {
        return;
    };
    let format = powerio::FormatId::new("psse-rawx")
        .expect("the canonical PSS/E RAWX format ID must be valid");
    let Ok(module) = powerio::parse(source.with_format(format)) else {
        return;
    };

    // A module accepted by the parser must be representable in PowerIO's IR,
    // and the emitted IR document must be accepted by the matching reader.
    let stored = powerio::serialize(
        &module,
        Destination::memory("fuzz.pio.json").expect("a memory destination must be valid"),
    )
    .expect("an accepted RAWX module must serialize to PowerIO IR");
    let EmittedOutput::Memory { artifacts } = stored.into_output() else {
        panic!("a memory destination must return memory artifacts");
    };
    let stored = artifacts
        .into_iter()
        .next()
        .expect("serialization must produce an artifact");
    let stored_source = Source::from_memory("fuzz.pio.json", stored.into_bytes())
        .expect("serialized IR bytes must form a source");
    let restored = powerio::deserialize(stored_source)
        .expect("serialized IR from an accepted RAWX module must deserialize");

    // Some parsed records may be intentionally unrepresentable by the writer;
    // test reader/writer consistency whenever emission claims success.
    let Ok(emitted) = powerio::emit(
        &restored,
        "psse-rawx",
        Destination::memory("roundtrip.rawx").expect("a memory destination must be valid"),
    ) else {
        return;
    };
    let EmittedOutput::Memory { artifacts } = emitted.into_output() else {
        panic!("RAWX memory emission must return memory artifacts");
    };
    let emitted = artifacts
        .into_iter()
        .next()
        .expect("successful RAWX emission must produce an artifact");
    let emitted_source = Source::from_memory("roundtrip.rawx", emitted.into_bytes())
        .expect("emitted RAWX bytes must form a source")
        .with_format(
            powerio::FormatId::new("psse-rawx")
                .expect("the canonical PSS/E RAWX format ID must be valid"),
        );
    powerio::parse(emitted_source)
        .expect("freshly emitted PSS/E RAWX must be readable by the RAWX reader");
});
