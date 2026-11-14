//! The voice thread: LiveKit (Tokio) plus the audio devices (cpal). Bevy talks to it only through
//! `ToVoice` / `FromVoice` channels and never waits on it.
//!
//! Audio path (docs/knowledge/12, "Microphone capture" and "Playback"):
//! - mic (cpal) → WebRTC `AudioProcessingModule` (echo cancellation, noise suppression, gain
//!   control) → `NativeAudioSource` → the published microphone track;
//! - each remote track → `NativeAudioStream` → `Mixer` (per-speaker gain and pan) → one cpal output
//!   stream. The mix is also the APM's far-end (reverse) signal, which is what lets it cancel echo.
//!
//! The SDK's `PlatformAudio` mode would capture with WebRTC's own device module, but it plays every
//! remote track itself with no per-track volume, so proximity could only switch speakers on and off.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::StreamExt;
use game_shared::rules::PlayerId;
use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use livekit::webrtc::native::apm::AudioProcessingModule;
use tokio::sync::mpsc as tokio_mpsc;

/// Commands from the game.
pub enum ToVoice {
    Connect {
        url: String,
        token: String,
    },
    Disconnect,
    /// Microphone open (push-to-talk held, or open mic) or closed.
    Mic(bool),
    /// Run the OS microphone stream or stop it (push-to-talk: only while V is held).
    Capture(bool),
    /// Every remote speaker's playback, about every 100 ms.
    Mix(Vec<Speaker>),
    /// Input device by name; `None` is the system default.
    Device(Option<String>),
    ListDevices,
}

/// How one remote player is heard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speaker {
    pub id: PlayerId,
    /// 0..=1, everything included (proximity, volume, local mute).
    pub gain: f32,
    /// -1 (left) ..= 1 (right).
    pub pan: f32,
    pub subscribed: bool,
}

/// News for the game.
pub enum FromVoice {
    Connected,
    /// Voice could not start or stopped; the game shows it once and carries on.
    Failed(String),
    /// Connected, but something is missing (e.g. no microphone): listen-only.
    Warning(String),
    Speaking(Vec<PlayerId>),
    Devices(Vec<String>),
}

/// 10 ms frames: what WebRTC and its APM work in.
const FRAMES_PER_SECOND: u32 = 100;
/// Remote audio queued per speaker before the oldest is dropped, so latency stays bounded.
const MAX_QUEUE_SECS: f32 = 0.2;
/// How fast a speaker's gain follows a new target, per output sample (smooths 100 ms steps).
const GAIN_SMOOTHING: f32 = 0.002;
/// `NativeAudioSource` buffer; 0 would demand exact 10 ms frames on a fast path.
const SOURCE_QUEUE_MS: u32 = 50;
/// Debug only: a generated tone instead of the microphone (`GAME_VOICE_TEST=tone`).
const TEST_TONE_HZ: f32 = 440.0;
const TEST_TONE_RATE: u32 = 48_000;
const TEST_TONE_LEVEL: f32 = 0.3;
/// How often the voice thread logs what it plays (debug aid for the end-to-end check).
const STATS_INTERVAL: Duration = Duration::from_secs(2);
const MIC_TRACK_NAME: &str = "microphone";

/// Starts the voice thread. It lives as long as the game; it touches no device until `Connect`.
pub fn spawn(
    test_tone: bool,
) -> (
    tokio_mpsc::UnboundedSender<ToVoice>,
    mpsc::Receiver<FromVoice>,
) {
    let (to_voice, commands) = tokio_mpsc::unbounded_channel();
    let (events, from_voice) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("voice".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => runtime.block_on(run(commands, events, test_tone)),
                Err(e) => {
                    let _ = events.send(FromVoice::Failed(e.to_string()));
                }
            }
        });
    if let Err(e) = spawned {
        bevy::log::error!("voice thread: {e}");
    }
    (to_voice, from_voice)
}

/// Remote audio waiting to be played, and how to play it.
#[derive(Default)]
pub struct Mixer {
    tracks: HashMap<PlayerId, Track>,
    max_queue: usize,
}

#[derive(Default)]
struct Track {
    queue: VecDeque<f32>,
    gain: f32,
    pan: f32,
    /// Smoothed left/right gains actually applied.
    left: f32,
    right: f32,
    /// Peak level of the last stats period, before and after the gain (for the debug log).
    peak_in: f32,
    peak_out: f32,
}

/// Left and right gains for `gain` at `pan`: the far side drops to `1 - |pan|`, the near side
/// stays at full; with `VOICE_PAN` < 1 a speaker is never one-sided.
pub fn pan_gains(gain: f32, pan: f32) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    (gain * (1.0 - pan.max(0.0)), gain * (1.0 + pan.min(0.0)))
}

impl Mixer {
    fn new(sample_rate: u32) -> Self {
        Self {
            tracks: HashMap::new(),
            max_queue: (sample_rate as f32 * MAX_QUEUE_SECS) as usize,
        }
    }

    fn push(&mut self, id: PlayerId, samples: &[i16]) {
        let max = self.max_queue;
        let track = self.tracks.entry(id).or_default();
        track
            .queue
            .extend(samples.iter().map(|&s| f32::from(s) / f32::from(i16::MAX)));
        let excess = track.queue.len().saturating_sub(max);
        track.queue.drain(..excess);
    }

    fn set(&mut self, speakers: &[Speaker]) {
        for speaker in speakers {
            let track = self.tracks.entry(speaker.id).or_default();
            track.gain = speaker.gain;
            track.pan = speaker.pan;
        }
    }

    /// Fills interleaved `out` (`channels` per frame, first two are left/right) and returns the
    /// mono mix for the echo canceller.
    fn mix(&mut self, out: &mut [f32], channels: usize, mono: &mut Vec<f32>) {
        out.fill(0.0);
        mono.clear();
        let frames = out.len() / channels.max(1);
        mono.resize(frames, 0.0);
        for track in self.tracks.values_mut() {
            let (left, right) = pan_gains(track.gain, track.pan);
            for (frame, mono) in out.chunks_mut(channels.max(1)).zip(mono.iter_mut()) {
                track.left += (left - track.left) * GAIN_SMOOTHING;
                track.right += (right - track.right) * GAIN_SMOOTHING;
                let Some(sample) = track.queue.pop_front() else {
                    continue;
                };
                track.peak_in = track.peak_in.max(sample.abs());
                track.peak_out = track
                    .peak_out
                    .max((sample * track.left.max(track.right)).abs());
                match frame {
                    [l, r, ..] => {
                        *l += sample * track.left;
                        *r += sample * track.right;
                    }
                    [only] => *only += sample * (track.left + track.right) / 2.0,
                    [] => {}
                }
                *mono += sample * (track.left + track.right) / 2.0;
            }
        }
        for sample in out.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }

    fn remove(&mut self, id: PlayerId) {
        self.tracks.remove(&id);
    }
}

/// Splits a continuous stream into 10 ms `i16` chunks.
struct Chunker {
    size: usize,
    buf: Vec<i16>,
}

impl Chunker {
    fn new(sample_rate: u32) -> Self {
        Self {
            size: (sample_rate / FRAMES_PER_SECOND) as usize,
            buf: Vec::new(),
        }
    }

    fn feed(&mut self, sample: f32, mut emit: impl FnMut(Vec<i16>)) {
        self.buf
            .push((sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16);
        if self.buf.len() == self.size {
            emit(std::mem::take(&mut self.buf));
        }
    }
}

fn player_of(identity: &ParticipantIdentity) -> Option<PlayerId> {
    identity.as_str().parse().ok().map(PlayerId)
}

/// One 10 ms mono chunk at `rate`, from the microphone or the test tone.
type Chunk = (Vec<i16>, u32);

/// The speakers: one output stream for the whole session.
fn open_output(
    mixer: Arc<Mutex<Mixer>>,
    reverse: tokio_mpsc::UnboundedSender<Chunk>,
) -> Result<(cpal::Stream, u32), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no audio output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!(
            "unsupported output format {:?}",
            supported.sample_format()
        ));
    }
    let config: cpal::StreamConfig = supported.into();
    let (rate, channels) = (config.sample_rate, usize::from(config.channels));
    let mut chunker = Chunker::new(rate);
    let mut mono = Vec::new();
    let stream = device
        .build_output_stream(
            &config,
            move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Never block the audio thread: a busy mixer means one silent buffer.
                let Ok(mut mixer) = mixer.try_lock() else {
                    out.fill(0.0);
                    return;
                };
                mixer.mix(out, channels, &mut mono);
                drop(mixer);
                for &sample in &mono {
                    chunker.feed(sample, |chunk| {
                        let _ = reverse.send((chunk, rate));
                    });
                }
            },
            |e| bevy::log::warn!("voice output: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

/// The microphone, mixed down to mono, in 10 ms chunks.
fn open_input(
    name: Option<&str>,
    chunks: tokio_mpsc::UnboundedSender<Chunk>,
) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = match name {
        Some(name) => host
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.description().is_ok_and(|desc| desc.name() == name))
            .ok_or_else(|| format!("microphone {name} not found"))?,
        None => host.default_input_device().ok_or("no microphone")?,
    };
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let (rate, channels) = (config.sample_rate, usize::from(config.channels).max(1));
    let mut chunker = Chunker::new(rate);
    let mut feed = move |frame: &mut dyn Iterator<Item = f32>| {
        let sample = frame.sum::<f32>() / channels as f32;
        chunker.feed(sample, |chunk| {
            let _ = chunks.send((chunk, rate));
        });
    };
    let error = |e| bevy::log::warn!("voice input: {e}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                for frame in data.chunks(channels) {
                    feed(&mut frame.iter().copied());
                }
            },
            error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                for frame in data.chunks(channels) {
                    feed(&mut frame.iter().map(|&s| f32::from(s) / f32::from(i16::MAX)));
                }
            },
            error,
            None,
        ),
        other => return Err(format!("unsupported microphone format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Debug only: a sine tone in 10 ms chunks, in real time.
fn spawn_test_tone(chunks: tokio_mpsc::UnboundedSender<Chunk>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let per_chunk = (TEST_TONE_RATE / FRAMES_PER_SECOND) as usize;
        let step = std::f32::consts::TAU * TEST_TONE_HZ / TEST_TONE_RATE as f32;
        let mut phase = 0.0f32;
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        loop {
            tick.tick().await;
            let chunk = (0..per_chunk)
                .map(|_| {
                    phase = (phase + step) % std::f32::consts::TAU;
                    (phase.sin() * TEST_TONE_LEVEL * f32::from(i16::MAX)) as i16
                })
                .collect();
            if chunks.send((chunk, TEST_TONE_RATE)).is_err() {
                return;
            }
        }
    })
}

/// A connected room and everything that lives only while connected.
struct Session {
    room: Room,
    mic: Option<LocalTrackPublication>,
    /// Created on the first chunk, at that chunk's rate.
    source: Option<NativeAudioSource>,
    _output: cpal::Stream,
    input: Option<cpal::Stream>,
    tone: Option<tokio::task::JoinHandle<()>>,
    output_rate: u32,
    players: HashMap<PlayerId, tokio::task::JoinHandle<()>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        for task in self.players.values() {
            task.abort();
        }
        if let Some(tone) = &self.tone {
            tone.abort();
        }
    }
}

struct Engine {
    events: mpsc::Sender<FromVoice>,
    test_tone: bool,
    mixer: Arc<Mutex<Mixer>>,
    mic_open: bool,
    /// The OS microphone stream should run (`ToVoice::Capture`).
    capture: bool,
    device: Option<String>,
    /// Who should be subscribed, as last told by the game.
    wanted: HashMap<PlayerId, bool>,
    chunks: tokio_mpsc::UnboundedSender<Chunk>,
    reverse: tokio_mpsc::UnboundedSender<Chunk>,
    apm: AudioProcessingModule,
}

impl Engine {
    fn say(&self, event: FromVoice) {
        let _ = self.events.send(event);
    }

    fn mixer(&self) -> std::sync::MutexGuard<'_, Mixer> {
        // A poisoned mixer only means an audio callback panicked; the data is still usable.
        self.mixer.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn connect(&mut self, url: &str, token: &str) -> Result<(Session, RoomEvents), String> {
        let (output, output_rate) = open_output(self.mixer.clone(), self.reverse.clone())?;
        *self.mixer() = Mixer::new(output_rate);
        let (room, events) = Room::connect(url, token, RoomOptions::default())
            .await
            .map_err(|e| e.to_string())?;
        bevy::log::info!(
            "voice: connected to {} as {}",
            room.name(),
            room.local_participant().identity().as_str()
        );
        let mut session = Session {
            room,
            mic: None,
            source: None,
            _output: output,
            input: None,
            tone: None,
            output_rate,
            players: HashMap::new(),
        };
        if self.test_tone {
            session.tone = Some(spawn_test_tone(self.chunks.clone()));
        } else if self.capture {
            self.start_capture(&mut session);
        }
        Ok((session, events))
    }

    /// Opens the OS microphone (no-op with the test tone or when it already runs).
    fn start_capture(&self, session: &mut Session) {
        if self.test_tone || session.input.is_some() {
            return;
        }
        match open_input(self.device.as_deref(), self.chunks.clone()) {
            Ok(input) => session.input = Some(input),
            Err(e) => self.say(FromVoice::Warning(format!("No microphone: {e}"))),
        }
    }

    /// Closes the OS microphone and drops whatever it captured that is not sent yet, so nothing
    /// from this press reaches the next one. The echo canceller keeps its state: it is fed the
    /// far end all along (the reverse branch in `run`), so it is warm when capture restarts.
    fn stop_capture(
        &self,
        session: &mut Session,
        chunks: &mut tokio_mpsc::UnboundedReceiver<Chunk>,
    ) {
        session.input = None;
        while chunks.try_recv().is_ok() {}
        if let Some(source) = &session.source {
            source.clear_buffer();
        }
    }

    /// The first chunk fixes the source's rate; the track is published then, muted until the
    /// microphone opens.
    async fn publish(&mut self, session: &mut Session, rate: u32) {
        let source =
            NativeAudioSource::new(AudioSourceOptions::default(), rate, 1, SOURCE_QUEUE_MS);
        let track = LocalAudioTrack::create_audio_track(
            MIC_TRACK_NAME,
            RtcAudioSource::Native(source.clone()),
        );
        let options = TrackPublishOptions {
            source: TrackSource::Microphone,
            ..TrackPublishOptions::default()
        };
        session.source = Some(source);
        match session
            .room
            .local_participant()
            .publish_track(LocalTrack::Audio(track), options)
            .await
        {
            Ok(publication) => {
                if !self.mic_open {
                    publication.mute();
                }
                bevy::log::info!("voice: microphone track published at {rate} Hz");
                session.mic = Some(publication);
            }
            Err(e) => self.say(FromVoice::Warning(format!("Cannot talk: {e}"))),
        }
    }

    async fn on_chunk(&mut self, session: &mut Session, (mut chunk, rate): Chunk) {
        if session.source.is_none() {
            self.publish(session, rate).await;
        }
        if !self.mic_open {
            return;
        }
        // The tone is a test signal: noise suppression would treat a steady tone as noise.
        if !self.test_tone
            && let Err(e) = self.apm.process_stream(&mut chunk, rate as i32, 1)
        {
            bevy::log::warn!("voice: echo canceller: {e}");
        }
        let Some(source) = &session.source else {
            return;
        };
        let frame = AudioFrame {
            samples_per_channel: chunk.len() as u32,
            data: chunk.into(),
            sample_rate: rate,
            num_channels: 1,
        };
        if let Err(e) = source.capture_frame(&frame).await {
            bevy::log::warn!("voice: capture: {e}");
        }
    }

    fn subscribe_wanted(&self, session: &Session) {
        for participant in session.room.remote_participants().values() {
            let Some(id) = player_of(&participant.identity()) else {
                continue;
            };
            let want = self.wanted.get(&id).copied().unwrap_or(true);
            for publication in participant.track_publications().values() {
                if publication.is_subscribed() != want {
                    bevy::log::info!(
                        "voice: {} player {}",
                        if want {
                            "subscribing to"
                        } else {
                            "unsubscribing from"
                        },
                        id.0
                    );
                    publication.set_subscribed(want);
                }
            }
        }
    }

    fn on_room_event(&mut self, session: &mut Session, event: RoomEvent) -> Option<String> {
        match event {
            RoomEvent::TrackSubscribed {
                track: RemoteTrack::Audio(track),
                participant,
                ..
            } => {
                let id = player_of(&participant.identity())?;
                bevy::log::info!(
                    "voice: subscribed to player {}'s track {}",
                    id.0,
                    track.sid()
                );
                let mut stream =
                    NativeAudioStream::new(track.rtc_track(), session.output_rate as i32, 1);
                let mixer = self.mixer.clone();
                let task = tokio::spawn(async move {
                    while let Some(frame) = stream.next().await {
                        if let Ok(mut mixer) = mixer.lock() {
                            mixer.push(id, &frame.data);
                        }
                    }
                });
                if let Some(old) = session.players.insert(id, task) {
                    old.abort();
                }
            }
            RoomEvent::TrackUnsubscribed { participant, .. }
            | RoomEvent::ParticipantDisconnected(participant) => {
                let id = player_of(&participant.identity())?;
                bevy::log::info!("voice: player {} no longer heard", id.0);
                if let Some(task) = session.players.remove(&id) {
                    task.abort();
                }
                self.mixer().remove(id);
            }
            RoomEvent::TrackPublished { .. } => self.subscribe_wanted(session),
            RoomEvent::ActiveSpeakersChanged { speakers } => {
                let ids = speakers.iter().filter_map(|p| player_of(&p.identity()));
                self.say(FromVoice::Speaking(ids.collect()));
            }
            RoomEvent::Disconnected { reason } => return Some(format!("{reason:?}")),
            _ => {}
        }
        None
    }

    fn log_stats(&self) {
        let mut mixer = self.mixer();
        for (id, track) in mixer.tracks.iter_mut() {
            if track.peak_in > 0.0 {
                bevy::log::info!(
                    "voice: player {} peak in {:.3} out {:.3} (gain {:.2}, pan {:+.2})",
                    id.0,
                    track.peak_in,
                    track.peak_out,
                    track.gain,
                    track.pan
                );
            }
            track.peak_in = 0.0;
            track.peak_out = 0.0;
        }
    }
}

type RoomEvents = tokio_mpsc::UnboundedReceiver<RoomEvent>;

async fn next_room_event(events: &mut Option<RoomEvents>) -> Option<RoomEvent> {
    match events {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    }
}

fn input_device_names() -> Vec<String> {
    let Ok(devices) = cpal::default_host().input_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|d| d.description().ok().map(|desc| desc.name().to_owned()))
        .collect()
}

async fn run(
    mut commands: tokio_mpsc::UnboundedReceiver<ToVoice>,
    events: mpsc::Sender<FromVoice>,
    test_tone: bool,
) {
    let (chunks, mut chunk_rx) = tokio_mpsc::unbounded_channel::<Chunk>();
    let (reverse, mut reverse_rx) = tokio_mpsc::unbounded_channel::<Chunk>();
    let mut engine = Engine {
        events,
        test_tone,
        mixer: Arc::default(),
        mic_open: false,
        capture: false,
        device: None,
        wanted: HashMap::new(),
        chunks,
        reverse,
        // Echo cancellation, gain control, high-pass filter, noise suppression.
        apm: AudioProcessingModule::new(true, true, true, true),
    };
    let mut session: Option<Session> = None;
    let mut room_events: Option<RoomEvents> = None;
    let mut stats = tokio::time::interval(STATS_INTERVAL);
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    ToVoice::Connect { url, token } => {
                        session = None;
                        room_events = None;
                        match engine.connect(&url, &token).await {
                            Ok((connected, events)) => {
                                session = Some(connected);
                                room_events = Some(events);
                                engine.say(FromVoice::Connected);
                            }
                            Err(e) => engine.say(FromVoice::Failed(e)),
                        }
                    }
                    ToVoice::Disconnect => {
                        if let Some(old) = session.take() {
                            let _ = old.room.close().await;
                            bevy::log::info!("voice: disconnected");
                        }
                        room_events = None;
                        *engine.mixer() = Mixer::default();
                    }
                    ToVoice::Mic(open) => {
                        engine.mic_open = open;
                        if let Some(mic) = session.as_ref().and_then(|s| s.mic.as_ref()) {
                            if open { mic.unmute() } else { mic.mute() }
                        }
                    }
                    ToVoice::Capture(on) => {
                        engine.capture = on;
                        if let Some(s) = session.as_mut() {
                            if on {
                                engine.start_capture(s);
                            } else {
                                engine.stop_capture(s, &mut chunk_rx);
                            }
                        }
                    }
                    ToVoice::Mix(speakers) => {
                        engine.mixer().set(&speakers);
                        let changed = speakers
                            .iter()
                            .any(|s| engine.wanted.insert(s.id, s.subscribed) != Some(s.subscribed));
                        if changed && let Some(session) = &session {
                            engine.subscribe_wanted(session);
                        }
                    }
                    ToVoice::Device(name) => {
                        engine.device = name;
                        // Reopen on the new device only if the microphone is running now.
                        if let Some(s) = session.as_mut().filter(|s| s.input.is_some()) {
                            engine.stop_capture(s, &mut chunk_rx);
                            engine.start_capture(s);
                        }
                    }
                    ToVoice::ListDevices => engine.say(FromVoice::Devices(input_device_names())),
                }
            }
            event = next_room_event(&mut room_events) => {
                let ended = match (event, session.as_mut()) {
                    (Some(event), Some(s)) => engine.on_room_event(s, event),
                    _ => Some("connection closed".to_owned()),
                };
                if let Some(reason) = ended {
                    session = None;
                    room_events = None;
                    engine.say(FromVoice::Failed(format!("Voice disconnected ({reason})")));
                }
            }
            Some(chunk) = chunk_rx.recv() => {
                // Far end first, so the canceller knows what the speakers just played.
                while let Ok((mut far, rate)) = reverse_rx.try_recv() {
                    let _ = engine.apm.process_reverse_stream(&mut far, rate as i32, 1);
                }
                if let Some(s) = session.as_mut() {
                    engine.on_chunk(s, chunk).await;
                }
            }
            // The far end also while the microphone is closed: keeps the echo canceller in step
            // with the speakers, and the queue from growing between push-to-talk presses.
            Some((mut far, rate)) = reverse_rx.recv() => {
                let _ = engine.apm.process_reverse_stream(&mut far, rate as i32, 1);
            }
            _ = stats.tick() => engine.log_stats(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speaker(gain: f32, pan: f32) -> Speaker {
        Speaker {
            id: PlayerId(1),
            gain,
            pan,
            subscribed: true,
        }
    }

    /// Mixes enough frames that the smoothed gains have settled, returns the last (left, right).
    fn settled(gain: f32, pan: f32) -> (f32, f32) {
        let mut mixer = Mixer::new(48_000);
        mixer.set(&[speaker(gain, pan)]);
        let mut out = vec![0.0; 2 * 4800];
        mixer.push(PlayerId(1), &[i16::MAX / 2; 4800]);
        mixer.mix(&mut out, 2, &mut Vec::new());
        (out[out.len() - 2], out[out.len() - 1])
    }

    #[test]
    fn the_mixer_applies_gain_and_pan() {
        let (l, r) = settled(1.0, 0.0);
        assert!((l - 0.5).abs() < 0.01 && (r - 0.5).abs() < 0.01, "{l} {r}");
        assert_eq!(settled(0.0, 0.0), (0.0, 0.0));
        // A speaker to the right: right at full, left softer but never silent.
        let (l, r) = settled(1.0, 0.6);
        assert!((r - 0.5).abs() < 0.01 && (l - 0.2).abs() < 0.01, "{l} {r}");
    }

    #[test]
    fn pan_never_silences_a_side_within_voice_pan() {
        assert_eq!(pan_gains(1.0, 0.0), (1.0, 1.0));
        let (l, r) = pan_gains(1.0, -0.6);
        assert!((l - 1.0).abs() < 1e-6 && (r - 0.4).abs() < 1e-6);
    }
}
