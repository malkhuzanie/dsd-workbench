//! Audio output. The UI thread only sends commands and reads a snapshot;
//! the output device, file access and filter design live on a dedicated audio thread.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::backend::{Output, Sink};
use super::settings::Settings;
use super::stream::PcmStream;
use crate::source;

const QUEUE_CHUNKS: usize = 6;
const FEED_FRAMES: usize = 256;
const RING_FRAMES: usize = 8192;
const DEFAULT_VOLUME: f32 = 0.8;
const TICK: Duration = Duration::from_millis(25);

/// What to play: a time range of a source file.
#[derive(Debug, Clone)]
pub struct Request {
    pub path: PathBuf,
    pub start_seconds: f64,
    /// `None` plays to the end of the file.
    pub end_seconds: Option<f64>,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[derive(Default)]
struct Shared {
    frames: AtomicU64,
    stop: AtomicBool,
    error: Mutex<Option<String>>,
    ring: Mutex<VecDeque<f32>>,
}

/// State published by the audio thread for the UI.
#[derive(Default)]
struct Snapshot {
    /// A play request is being set up (device opening, file reading).
    pending: bool,
    active: bool,
    paused: bool,
    label: String,
    duration: f64,
    /// Seconds from the request's start at which decoding began.
    offset: f64,
    rate: u32,
    shared: Option<Arc<Shared>>,
    error: Option<String>,
}

type State = Arc<Mutex<Snapshot>>;

enum Command {
    Play {
        request: Request,
        offset: f64,
        settings: Settings,
    },
    Seek(f64),
    Stop,
    TogglePause,
    Volume(f32),
}

// ---- UI side -----------------------------------------------------------

pub struct Player {
    tx: Option<Sender<Command>>,
    state: State,
    volume: f32,
    settings: Settings,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            tx: None,
            state: Arc::default(),
            volume: DEFAULT_VOLUME,
            settings: Settings::default(),
        }
    }
}

impl Player {
    fn read<R: Default>(&self, f: impl FnOnce(&Snapshot) -> R) -> R {
        self.state.lock().map(|s| f(&s)).unwrap_or_default()
    }

    fn write(&self, f: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut s) = self.state.lock() {
            f(&mut s);
        }
    }

    fn fail(&mut self, message: &str) {
        self.write(|s| {
            s.error = Some(message.to_owned());
            s.pending = false;
            s.active = false;
        });
    }

    fn send(&mut self, command: Command) {
        if self.tx.is_none() {
            self.tx = spawn_thread(Arc::clone(&self.state));
            if let Some(tx) = &self.tx {
                let _ = tx.send(Command::Volume(self.volume));
            }
        }
        let Some(tx) = &self.tx else {
            self.fail("could not start the audio thread");
            return;
        };
        if tx.send(command).is_err() {
            self.tx = None;
            self.fail("the audio thread stopped unexpectedly");
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// Applies to the next [`Player::play`].
    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
    }

    pub fn play(&mut self, request: Request) {
        let label = request.label.clone();
        self.write(|s| {
            s.pending = true;
            s.label = label;
            s.rate = 0;
        });
        let settings = self.settings;
        self.send(Command::Play {
            request,
            offset: 0.0,
            settings,
        });
    }

    /// Restarts playback `seconds` after the start of the current request.
    pub fn seek(&mut self, seconds: f64) {
        if self.is_active() {
            self.send(Command::Seek(seconds));
        }
    }

    pub fn stop(&mut self) {
        self.write(|s| {
            s.pending = false;
            s.active = false;
            s.paused = false;
            s.shared = None;
        });
        if self.tx.is_some() {
            self.send(Command::Stop);
        }
    }

    pub fn toggle_pause(&mut self) {
        if self.is_active() {
            self.send(Command::TogglePause);
        }
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.state.lock().ok().and_then(|mut s| s.error.take())
    }

    pub fn status(&self) -> Status {
        self.read(|s| match (s.active, s.paused) {
            (false, _) => Status::Stopped,
            (true, true) => Status::Paused,
            (true, false) => Status::Playing,
        })
    }

    /// True while playing, paused, or setting up a request.
    pub fn is_active(&self) -> bool {
        self.read(|s| s.pending || s.active)
    }

    pub fn label(&self) -> Option<String> {
        self.read(|s| (s.pending || s.active).then(|| s.label.clone()))
    }

    /// Length of the current request in seconds (0 when stopped).
    pub fn duration_seconds(&self) -> f64 {
        self.read(|s| if s.active { s.duration } else { 0.0 })
    }

    pub fn position_seconds(&self) -> f64 {
        self.read(|s| match (&s.shared, s.active) {
            (Some(shared), true) => {
                let played =
                    shared.frames.load(Ordering::Relaxed) as f64 / f64::from(s.rate.max(1));
                (s.offset + played).min(s.duration)
            }
            _ => 0.0,
        })
    }

    /// Output sample rate of the current playback (0 while setting up).
    pub fn sample_rate(&self) -> u32 {
        self.read(|s| if s.active { s.rate } else { 0 })
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if self.tx.is_some() {
            let volume = self.volume;
            self.send(Command::Volume(volume));
        }
    }

    /// The most recent mono samples that reached the output, oldest first.
    pub fn recent_samples(&self) -> Vec<f32> {
        let shared = self.read(|s| s.shared.clone());
        shared
            .and_then(|shared| {
                shared
                    .ring
                    .lock()
                    .ok()
                    .map(|ring| ring.iter().copied().collect::<Vec<f32>>())
            })
            .unwrap_or_default()
    }
}

fn spawn_thread(state: State) -> Option<Sender<Command>> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("dsf-audio".to_owned())
        .spawn(move || run(&rx, state))
        .ok()?;
    Some(tx)
}

// ---- audio thread ------------------------------------------------------

struct Playback {
    sink: Sink,
    shared: Arc<Shared>,
    /// The request with `end_seconds` resolved.
    request: Request,
    settings: Settings,
}

struct Audio {
    output: Output,
    current: Option<Playback>,
    volume: f32,
    state: State,
}

fn run(rx: &Receiver<Command>, state: State) {
    let mut audio = Audio {
        output: Output::new(),
        current: None,
        volume: DEFAULT_VOLUME,
        state,
    };
    loop {
        match rx.recv_timeout(TICK) {
            Ok(command) => audio.handle(command),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        audio.tick();
    }
    audio.stop();
}

impl Audio {
    fn publish(&self, f: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut s) = self.state.lock() {
            f(&mut s);
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Play {
                request,
                offset,
                settings,
            } => self.begin(request, offset, settings),
            Command::Seek(seconds) => {
                if let Some(current) = &self.current {
                    let (request, settings) = (current.request.clone(), current.settings);
                    self.begin(request, seconds, settings);
                }
            }
            Command::Stop => self.stop(),
            Command::TogglePause => {
                if let Some(current) = &self.current {
                    if current.sink.is_paused() {
                        current.sink.play();
                    } else {
                        current.sink.pause();
                    }
                    let paused = current.sink.is_paused();
                    self.publish(|s| s.paused = paused);
                }
            }
            Command::Volume(volume) => {
                self.volume = volume;
                if let Some(current) = &self.current {
                    current.sink.set_volume(volume);
                }
            }
        }
    }

    /// Detects the end of playback and decoder errors.
    fn tick(&mut self) {
        let failure = self
            .current
            .as_ref()
            .and_then(|c| c.shared.error.lock().ok().and_then(|mut slot| slot.take()));
        if let Some(message) = failure {
            self.stop();
            self.publish(|s| s.error = Some(message));
            return;
        }
        if self.current.as_ref().is_some_and(|c| c.sink.empty()) {
            self.stop();
        }
    }

    fn stop(&mut self) {
        if let Some(playback) = self.current.take() {
            playback.shared.stop.store(true, Ordering::Relaxed);
            playback.sink.stop();
        }
        // `pending` belongs to the request flow; only `begin` clears it.
        self.publish(|s| {
            s.active = false;
            s.paused = false;
            s.shared = None;
        });
    }

    fn begin(&mut self, request: Request, offset: f64, settings: Settings) {
        let result = self.start(request, offset, settings);
        self.publish(|s| s.pending = false);
        if let Err(message) = result {
            self.publish(|s| s.error = Some(message));
        }
    }

    fn start(
        &mut self,
        mut request: Request,
        offset: f64,
        settings: Settings,
    ) -> Result<(), String> {
        self.stop();

        let source = source::open(&request.path).map_err(|e| e.to_string())?;
        let info = source.info().clone();
        if info.sample_rate == 0 {
            return Err("the source file reports a zero sample rate".to_owned());
        }
        let rate = f64::from(info.sample_rate);
        let total = info.sample_count as f64 / rate;
        let end = request.end_seconds.unwrap_or(total).min(total);
        request.end_seconds = Some(end);
        let from = (request.start_seconds + offset).max(0.0).min(end);

        let stream = PcmStream::new(
            source,
            (from * rate).round() as u64,
            (end * rate).round() as u64,
            &settings,
        )?;
        let (out_rate, channels) = (stream.out_rate(), stream.channels() as u16);

        let sink = self.output.open_sink()?;
        sink.set_volume(self.volume);

        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::sync_channel(QUEUE_CHUNKS);
        let flag = Arc::clone(&shared);
        thread::Builder::new()
            .name("dsf-decode".to_owned())
            .spawn(move || decode_loop(stream, &tx, &flag))
            .map_err(|e| format!("starting decoder: {e}"))?;
        sink.append(PcmSource::new(rx, channels, out_rate, Arc::clone(&shared)));

        let duration = (end - request.start_seconds).max(0.0);
        let label = request.label.clone();
        let started_at = from - request.start_seconds;
        let published = Arc::clone(&shared);
        self.current = Some(Playback {
            sink,
            shared,
            request,
            settings,
        });
        self.publish(|s| {
            s.active = true;
            s.paused = false;
            s.label = label;
            s.duration = duration;
            s.offset = started_at;
            s.rate = out_rate;
            s.shared = Some(published);
        });
        Ok(())
    }
}

fn decode_loop(mut stream: PcmStream, tx: &SyncSender<Vec<f32>>, shared: &Shared) {
    while !shared.stop.load(Ordering::Relaxed) {
        match stream.next_chunk() {
            Ok(Some(chunk)) => {
                if tx.send(chunk).is_err() {
                    break; // the output side is gone
                }
            }
            Ok(None) => break,
            Err(message) => {
                if let Ok(mut slot) = shared.error.lock() {
                    *slot = Some(message);
                }
                break;
            }
        }
    }
}

/// Feeds decoded chunks to the output. Never blocks: an empty queue plays silence.
pub(super) struct PcmSource {
    rx: Receiver<Vec<f32>>,
    chunk: std::vec::IntoIter<f32>,
    channels: u16,
    // Only the real audio backend asks for the rate.
    #[cfg_attr(not(feature = "playback"), allow(dead_code))]
    rate: u32,
    shared: Arc<Shared>,
    frame_pos: u16,
    frame_sum: f32,
    pending: Vec<f32>,
    silence_left: u16,
}

impl PcmSource {
    fn new(rx: Receiver<Vec<f32>>, channels: u16, rate: u32, shared: Arc<Shared>) -> Self {
        Self {
            rx,
            chunk: Vec::new().into_iter(),
            channels: channels.max(1),
            rate,
            shared,
            frame_pos: 0,
            frame_sum: 0.0,
            pending: Vec::with_capacity(FEED_FRAMES),
            silence_left: 0,
        }
    }

    #[cfg_attr(not(feature = "playback"), allow(dead_code))]
    pub(super) fn channels(&self) -> u16 {
        self.channels
    }

    #[cfg_attr(not(feature = "playback"), allow(dead_code))]
    pub(super) fn rate(&self) -> u32 {
        self.rate
    }

    /// Publishes played frames to the position counter and the spectrum ring.
    fn publish(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        self.shared
            .frames
            .fetch_add(self.pending.len() as u64, Ordering::Relaxed);
        if let Ok(mut ring) = self.shared.ring.try_lock() {
            ring.extend(self.pending.iter().copied());
            while ring.len() > RING_FRAMES {
                ring.pop_front();
            }
        }
        self.pending.clear();
    }
}

impl Iterator for PcmSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.silence_left > 0 {
            self.silence_left -= 1;
            return Some(0.0);
        }
        let sample = loop {
            if let Some(sample) = self.chunk.next() {
                break sample;
            }
            match self.rx.try_recv() {
                Ok(next) => self.chunk = next.into_iter(),
                Err(TryRecvError::Empty) => {
                    self.silence_left = self.channels - 1;
                    return Some(0.0);
                }
                Err(TryRecvError::Disconnected) => {
                    self.publish();
                    return None;
                }
            }
        };

        self.frame_sum += sample;
        self.frame_pos += 1;
        if self.frame_pos == self.channels {
            self.pending.push(self.frame_sum / f32::from(self.channels));
            self.frame_sum = 0.0;
            self.frame_pos = 0;
            if self.pending.len() >= FEED_FRAMES {
                self.publish();
            }
        }
        Some(sample)
    }
}
