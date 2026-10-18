#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Any input must produce an error or a self-consistent `Info`, never a panic.
    if let Ok(info) = dsdiff::read_info(&mut Cursor::new(data)) {
        assert!(info.channel_count > 0);
        assert!(info.audio_offset.saturating_add(info.audio_len) <= data.len() as u64);
    }
});