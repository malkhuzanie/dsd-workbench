# dsdiff

Read the header of DSDIFF (`.dff`) files: sampling frequency, channel count, and where the audio is.

```rust
let mut file = std::fs::File::open("album.dff")?;
let info = dsdiff::read_info(&mut file)?;
println!("{} Hz, {} ch, {} samples", info.sample_rate, info.channel_count, info.sample_count);
```

- Audio is uncompressed DSD: interleaved one byte per channel, most significant bit first in time.
- DST-compressed files return `Error::Compressed`. ID3 and edit-master chunks are ignored.
- No `unsafe`, no panics. Parsing only; reading audio bytes is up to the caller (`info.audio_offset`, `info.audio_len`).

