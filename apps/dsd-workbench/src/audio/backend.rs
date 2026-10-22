//! The audio output device. With the `playback` feature it plays through `rodio`;
//! without it the same API exists but reports that this build cannot play sound, so the
//! rest of the app (and a headless build) works unchanged.

pub use imp::{Output, Sink};

#[cfg(feature = "playback")]
mod imp {
    use std::time::{Duration, Instant};

    use rodio::{OutputStream, OutputStreamHandle, Source};

    use crate::audio::player::PcmSource;
    use crate::error::{Error, Result};

    /// The default output device, opened on first use.
    pub struct Output {
        stream: Option<(OutputStream, OutputStreamHandle)>,
    }

    impl Output {
        pub fn new() -> Self {
            Self { stream: None }
        }

        /// A fresh sink on the device; opening failures are retried on the next call.
        pub fn open_sink(&mut self) -> Result<Sink> {
            if self.stream.is_none() {
                let started = Instant::now();
                eprintln!("[audio] opening output device…");
                let pair = OutputStream::try_default()
                    .map_err(|e| Error::Audio(format!("no audio output: {e}")))?;
                eprintln!("[audio] ready in {} ms", started.elapsed().as_millis());
                self.stream = Some(pair);
            }
            let (_, handle) = self
                .stream
                .as_ref()
                .ok_or_else(|| Error::Audio("no audio output".to_owned()))?;
            rodio::Sink::try_new(handle)
                .map(Sink)
                .map_err(|e| Error::Audio(format!("audio output: {e}")))
        }
    }

    pub struct Sink(rodio::Sink);

    impl Sink {
        pub fn append(&self, source: PcmSource) {
            self.0.append(source);
        }
        pub fn is_paused(&self) -> bool {
            self.0.is_paused()
        }
        pub fn play(&self) {
            self.0.play();
        }
        pub fn pause(&self) {
            self.0.pause();
        }
        pub fn set_volume(&self, volume: f32) {
            self.0.set_volume(volume);
        }
        pub fn empty(&self) -> bool {
            self.0.empty()
        }
        pub fn stop(&self) {
            self.0.stop();
        }
    }

    impl Source for PcmSource {
        fn current_frame_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> u16 {
            self.channels()
        }
        fn sample_rate(&self) -> u32 {
            self.rate()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }
}

#[cfg(not(feature = "playback"))]
mod imp {
    use crate::audio::player::PcmSource;
    use crate::error::{Error, Result};

    /// Placeholder for a build without audio output.
    pub struct Output(());

    impl Output {
        pub fn new() -> Self {
            Self(())
        }

        pub fn open_sink(&mut self) -> Result<Sink> {
            Err(Error::Audio(
                "this build of the app has no audio output (built without the `playback` feature)"
                    .to_owned(),
            ))
        }
    }

    pub struct Sink;

    impl Sink {
        pub fn append(&self, _source: PcmSource) {}
        pub fn is_paused(&self) -> bool {
            false
        }
        pub fn play(&self) {}
        pub fn pause(&self) {}
        pub fn set_volume(&self, _volume: f32) {}
        pub fn empty(&self) -> bool {
            true
        }
        pub fn stop(&self) {}
    }
}
