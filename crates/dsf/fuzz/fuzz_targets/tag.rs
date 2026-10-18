#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Arbitrary bytes as a whole file: reading the tag must never panic.
    let _ = dsf::read_tag(&mut Cursor::new(data));
});