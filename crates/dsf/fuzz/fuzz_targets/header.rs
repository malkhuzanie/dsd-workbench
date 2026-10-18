#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Must return an error or a header, never panic; a parsed header must round-trip.
    if let Ok(header) = dsf::DsfHeader::read(data) {
        let mut out = Vec::new();
        if header.write(&mut out).is_ok() {
            assert_eq!(dsf::DsfHeader::read(out.as_slice()).ok(), Some(header));
        }
    }
});