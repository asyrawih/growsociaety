//! Voice chat (docs/knowledge/12): proximity voice with the players in the same world, through
//! LiveKit. A view: it reads positions from `Game` and never changes it. Off until the player turns
//! it on in the settings' Voice tab (N); offline there is no voice (no server to issue a token).
//!
//! The LiveKit and audio work runs on its own thread (`engine.rs`); this file decides what it should
//! do (connect, open the mic, how loud each speaker is) and shows what it reports.

mod engine;
mod panel;

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use bevy::prelude::*;
use game_shared::TILE_SIZE;
use game_shared::rules::PlayerId;
use tokio::sync::mpsc as tokio_mpsc;

use crate::hud::Notice;
use crate::net::Net;
use crate::screenshot;
use crate::settings::{MAX_VOLUME, MicDevices, Settings, Unsaved};
use crate::state::{Game, Phase};
use engine::{FromVoice, Speaker, ToVoice};

/// Full volume inside this distance, in tiles (Tuning, doc 12).
pub const VOICE_NEAR: f32 = 6.0;
/// Volume falls linearly to 0 here.
pub const VOICE_FAR: f32 = 16.0;
/// Beyond this a speaker's track is unsubscribed (no bandwidth).
pub const VOICE_UNSUBSCRIBE: f32 = 18.0;
/// Closer than this it is subscribed again; the gap stops flapping at the edge.
pub const VOICE_RESUBSCRIBE: f32 = 16.0;
/// Stereo pan at `VOICE_FAR` horizontally; never fully one-sided.
pub const VOICE_PAN: f32 = 0.6;
/// How often speaker gains go to the voice thread.
const MIX_INTERVAL: Duration = Duration::from_millis(100);
/// Hold to talk in push-to-talk mode.
const PUSH_TO_TALK_KEY: KeyCode = KeyCode::KeyV;
/// A gain change worth a log line (end-to-end check).
const GAIN_LOG_STEP: f32 = 0.1;
/// Debug builds: `GAME_VOICE_TEST=listen` turns voice on, `tone` also sends a test tone on open mic.
const TEST_ENV: &str = "GAME_VOICE_TEST";

/// Gain (0..=1) and pan (-1..=1, + is right) of a speaker `dx_tiles`, `dy_tiles` from the listener.
pub fn proximity(dx_tiles: f32, dy_tiles: f32) -> (f32, f32) {
    let distance = dx_tiles.hypot(dy_tiles);
    let gain = ((VOICE_FAR - distance) / (VOICE_FAR - VOICE_NEAR)).clamp(0.0, 1.0);
    let pan = (dx_tiles / VOICE_FAR).clamp(-1.0, 1.0) * VOICE_PAN;
    (gain, pan)
}

/// Whether a speaker at `distance` tiles stays (or becomes) subscribed, with hysteresis.
pub fn keep_subscribed(subscribed: bool, distance: f32) -> bool {
    if subscribed {
        distance <= VOICE_UNSUBSCRIBE
    } else {
        distance < VOICE_RESUBSCRIBE
    }
}

/// What the microphone does: `capture` runs the OS input stream, `on_air` unmutes the track.
/// Open mic keeps the stream running while connected (talking still needs `allowed`: not muted by
/// the player or the world owner); push-to-talk runs it only while V is held and allowed
/// (owner decision: the OS mic opens only then; a short start-up delay is accepted).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MicPlan {
    pub capture: bool,
    pub on_air: bool,
}

pub fn mic_plan(connected: bool, open_mic: bool, v_held: bool, allowed: bool) -> MicPlan {
    let on_air = connected && allowed && (open_mic || v_held);
    let capture = if open_mic { connected } else { on_air };
    MicPlan { capture, on_air }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Link {
    #[default]
    Off,
    Connecting,
    Connected,
    /// Shown once; turning voice off and on again retries.
    Failed(String),
}

/// What voice is doing, for the panel and the indicators.
#[derive(Resource, Default)]
pub struct VoiceState {
    pub link: Link,
    /// The newest token from the server: (url, token).
    token: Option<(String, String)>,
    pub speaking: HashSet<PlayerId>,
    /// Muted by the world owner, for everyone.
    pub server_muted: HashSet<PlayerId>,
    /// Muted by this player, for this player only (not stored: ids change every session).
    pub local_muted: HashSet<PlayerId>,
    /// The track is live: "on air".
    pub mic_open: bool,
    /// The OS microphone stream runs.
    capture: bool,
    /// Online at all (offline has no voice).
    pub online: bool,
    /// The Voice tab wants the microphone list.
    pub request_devices: bool,
    hinted: bool,
}

/// The voice thread's ends of the channels. Online only.
#[derive(Resource)]
struct VoiceLink {
    to: tokio_mpsc::UnboundedSender<ToVoice>,
    from: Mutex<mpsc::Receiver<FromVoice>>,
}

impl VoiceLink {
    fn send(&self, command: ToVoice) {
        let _ = self.to.send(command);
    }
}

/// Subscription memory for the hysteresis, and the last logged gains.
#[derive(Resource, Default)]
struct Mix {
    subscribed: HashMap<PlayerId, bool>,
    logged: HashMap<PlayerId, f32>,
    timer: Timer,
}

/// Debug test mode from `GAME_VOICE_TEST`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TestMode {
    Listen,
    Tone,
}

fn test_mode() -> Option<TestMode> {
    if !cfg!(debug_assertions) {
        return None;
    }
    match std::env::var(TEST_ENV).ok()?.as_str() {
        "listen" => Some(TestMode::Listen),
        "tone" => Some(TestMode::Tone),
        _ => None,
    }
}

pub struct VoicePlugin;

impl Plugin for VoicePlugin {
    fn build(&self, app: &mut App) {
        let test = test_mode();
        if let Some(mode) = test {
            let mut settings = app.world_mut().resource_mut::<Settings>();
            settings.voice = true;
            settings.open_mic = mode == TestMode::Tone;
            app.insert_resource(Unsaved);
        }
        let online = app.world().contains_resource::<Net>();
        let mut state = VoiceState {
            online,
            ..VoiceState::default()
        };
        // A screenshot of the Voice tab also shows the speaking indicator over the player.
        if screenshot::ui_request().as_deref() == Some("settings-voice") {
            let me = app.world().resource::<Game>().1;
            state.speaking.insert(me);
        }
        if online {
            let (to, from) = engine::spawn(test == Some(TestMode::Tone));
            app.insert_resource(VoiceLink {
                to,
                from: Mutex::new(from),
            });
        }
        app.insert_resource(state)
            .insert_resource(Mix {
                timer: Timer::new(MIX_INTERVAL, TimerMode::Repeating),
                ..Mix::default()
            })
            .add_systems(
                Startup,
                panel::spawn_panel.after(crate::settings::spawn_settings),
            )
            .add_systems(Update, panel::click_panel.in_set(Phase::Input))
            .add_systems(
                Update,
                (
                    (sync_link, send_mix)
                        .chain()
                        .run_if(resource_exists::<VoiceLink>),
                    panel::refresh_panel,
                    panel::show_indicators,
                    follow_ignores.run_if(resource_exists::<Net>),
                )
                    .in_set(Phase::View),
            );
    }
}

/// Muting someone (the avatar menu's Mute = `/ignore`) also mutes their voice for me; unmuting
/// gives it back. Follows the server's ignore list (kept across logins), so it applies on join too.
fn follow_ignores(
    net: Res<Net>,
    mut state: ResMut<VoiceState>,
    mut last: Local<HashSet<PlayerId>>,
) {
    let now: HashSet<PlayerId> = net.ignoring().collect();
    if now == *last {
        return;
    }
    for id in now.difference(&last) {
        state.local_muted.insert(*id);
    }
    for id in last.difference(&now) {
        state.local_muted.remove(id);
    }
    *last = now;
}

/// Token in, connection decided, push-to-talk applied, news from the voice thread shown.
#[allow(clippy::too_many_arguments)] // a Bevy system: devices, three resources, two channels
fn sync_link(
    keys: Res<ButtonInput<KeyCode>>,
    game: Res<Game>,
    settings: Res<Settings>,
    mut net: ResMut<Net>,
    mut state: ResMut<VoiceState>,
    mut mics: ResMut<MicDevices>,
    link: Res<VoiceLink>,
    mut notices: MessageWriter<Notice>,
    mut last_device: Local<Option<Option<String>>>,
) {
    let me = game.1;
    if let Some(token) = net.take_voice_token() {
        state.token = Some(token);
        if !settings.voice && !state.hinted {
            state.hinted = true;
            notices.write(Notice("Voice chat is off: press N to set it up".into()));
        }
    }
    for (player, muted) in net.take_voice_muted() {
        if muted {
            state.server_muted.insert(player);
        } else {
            state.server_muted.remove(&player);
        }
        if player == me {
            let text = if muted {
                "The world owner muted your voice"
            } else {
                "The world owner unmuted your voice"
            };
            notices.write(Notice(text.into()));
        }
    }

    let want = settings.voice && state.token.is_some() && net.connected();
    match (&state.link, want) {
        (Link::Off, true) => {
            if let Some((url, token)) = state.token.clone() {
                link.send(ToVoice::Connect { url, token });
                state.link = Link::Connecting;
            }
        }
        (Link::Connecting | Link::Connected, false) => {
            link.send(ToVoice::Disconnect);
            state.link = Link::Off;
            state.mic_open = false;
            state.capture = false;
        }
        (Link::Failed(_), false) => state.link = Link::Off,
        _ => {}
    }
    if state.request_devices {
        state.request_devices = false;
        link.send(ToVoice::ListDevices);
    }
    if last_device.as_ref() != Some(&settings.mic) {
        link.send(ToVoice::Device(settings.mic.clone()));
        *last_device = Some(settings.mic.clone());
    }

    let plan = mic_plan(
        state.link == Link::Connected,
        settings.open_mic,
        keys.pressed(PUSH_TO_TALK_KEY),
        !settings.self_muted && !state.server_muted.contains(&me),
    );
    // Stream first on press (so the track is unmuted onto a running mic), track first on release.
    if plan.capture && !state.capture {
        state.capture = true;
        link.send(ToVoice::Capture(true));
    }
    if plan.on_air != state.mic_open {
        state.mic_open = plan.on_air;
        link.send(ToVoice::Mic(plan.on_air));
    }
    if !plan.capture && state.capture {
        state.capture = false;
        link.send(ToVoice::Capture(false));
    }

    let received: Vec<FromVoice> = match link.from.lock() {
        Ok(rx) => std::iter::from_fn(|| rx.try_recv().ok()).collect(),
        Err(_) => Vec::new(),
    };
    for event in received {
        match event {
            FromVoice::Connected => {
                state.link = Link::Connected;
                notices.write(Notice("Voice chat connected".into()));
            }
            FromVoice::Failed(reason) => {
                warn!("voice: {reason}");
                if state.link != Link::Off {
                    notices.write(Notice(format!("Voice unavailable: {reason}")));
                    state.link = Link::Failed(reason);
                }
                state.speaking.clear();
                state.mic_open = false;
                state.capture = false;
            }
            FromVoice::Warning(text) => {
                warn!("voice: {text}");
                notices.write(Notice(text));
            }
            FromVoice::Speaking(ids) => state.speaking = ids.into_iter().collect(),
            FromVoice::Devices(names) => mics.0 = names,
        }
    }
}

/// Every `MIX_INTERVAL`: each speaker's gain and pan from where the bodies are.
fn send_mix(
    time: Res<Time>,
    game: Res<Game>,
    settings: Res<Settings>,
    state: Res<VoiceState>,
    link: Res<VoiceLink>,
    mut mix: ResMut<Mix>,
) {
    if !mix.timer.tick(time.delta()).just_finished() || state.link != Link::Connected {
        return;
    }
    let Some(me) = game.me() else {
        return;
    };
    let (mx, my) = me.body.pos;
    let mut speakers = Vec::new();
    for player in game.0.players.values().filter(|p| p.id != me.id) {
        let (dx, dy) = (
            (player.body.pos.0 - mx) / TILE_SIZE,
            (player.body.pos.1 - my) / TILE_SIZE,
        );
        let (gain, pan) = proximity(dx, dy);
        let was = mix.subscribed.get(&player.id).copied().unwrap_or(true);
        let subscribed = keep_subscribed(was, dx.hypot(dy));
        mix.subscribed.insert(player.id, subscribed);
        let muted = state.local_muted.contains(&player.id);
        let volume = f32::from(settings.voice_volume) / f32::from(MAX_VOLUME);
        let gain = if muted { 0.0 } else { gain * volume };
        let logged = mix.logged.get(&player.id).copied();
        if logged.is_none_or(|g| (g - gain).abs() >= GAIN_LOG_STEP || (g > 0.0) != (gain > 0.0)) {
            info!(
                "voice: player {} at {:.1} tiles: gain {gain:.2}, pan {pan:+.2}",
                player.id.0,
                dx.hypot(dy)
            );
            mix.logged.insert(player.id, gain);
        }
        speakers.push(Speaker {
            id: player.id,
            gain,
            pan,
            subscribed,
        });
    }
    let present: HashSet<PlayerId> = speakers.iter().map(|s| s.id).collect();
    mix.subscribed.retain(|id, _| present.contains(id));
    mix.logged.retain(|id, _| present.contains(id));
    link.send(ToVoice::Mix(speakers));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_volume_up_close_silence_far_away_and_a_falling_line_between() {
        assert_eq!(proximity(0.0, 0.0).0, 1.0);
        assert_eq!(proximity(VOICE_NEAR, 0.0).0, 1.0);
        assert_eq!(proximity(0.0, -VOICE_NEAR).0, 1.0);
        assert_eq!(proximity(VOICE_FAR, 0.0).0, 0.0);
        assert_eq!(proximity(0.0, 40.0).0, 0.0);
        assert!(
            (proximity(11.0, 0.0).0 - 0.5).abs() < 1e-6,
            "halfway is half volume"
        );
        let mut last = 1.0;
        for step in 0..=200 {
            let d = step as f32 * 0.1;
            let (gain, _) = proximity(d * 0.6, d * 0.8);
            assert!(gain <= last + 1e-6, "monotonic at {d}");
            assert!((0.0..=1.0).contains(&gain));
            last = gain;
        }
    }

    #[test]
    fn pan_is_centred_symmetric_and_light() {
        assert_eq!(proximity(0.0, 5.0).1, 0.0);
        for step in 0..=300 {
            let dx = step as f32 * 0.1;
            let (_, right) = proximity(dx, 1.0);
            let (_, left) = proximity(-dx, 1.0);
            assert!((right + left).abs() < 1e-6, "symmetric at {dx}");
            assert!((0.0..=VOICE_PAN + 1e-6).contains(&right), "{right} at {dx}");
        }
        assert!((proximity(VOICE_FAR, 0.0).1 - VOICE_PAN).abs() < 1e-6);
    }

    #[test]
    fn push_to_talk_captures_only_while_v_is_held_and_open_mic_ignores_v() {
        let plan = |open_mic, v| mic_plan(true, open_mic, v, true);
        // Push-to-talk: press -> capture and on air; release -> both off.
        assert_eq!(plan(false, false), MicPlan::default());
        assert_eq!(
            plan(false, true),
            MicPlan {
                capture: true,
                on_air: true
            }
        );
        assert_eq!(plan(false, false), MicPlan::default());
        // Open mic: V changes nothing.
        assert_eq!(plan(true, false), plan(true, true));
        assert!(plan(true, false).capture && plan(true, false).on_air);
        // Muted: never on air; push-to-talk does not even open the OS mic.
        assert_eq!(mic_plan(true, false, true, false), MicPlan::default());
        assert!(!mic_plan(true, true, false, false).on_air);
        // Not connected: nothing.
        assert_eq!(mic_plan(false, true, true, true), MicPlan::default());
    }

    #[test]
    fn a_speaker_on_the_edge_does_not_flap() {
        // Oscillating around VOICE_UNSUBSCRIBE: one unsubscribe, then it stays off until
        // the speaker is clearly back inside VOICE_RESUBSCRIBE.
        let mut subscribed = true;
        let mut toggles = 0;
        for step in 0..100 {
            let wobble = if step % 2 == 0 { 0.4 } else { -0.4 };
            let next = keep_subscribed(subscribed, VOICE_UNSUBSCRIBE + wobble);
            toggles += usize::from(next != subscribed);
            subscribed = next;
        }
        assert_eq!(toggles, 1);
        assert!(!subscribed);
        // Around VOICE_RESUBSCRIBE likewise: back on once, then stays.
        for step in 0..100 {
            let wobble = if step % 2 == 0 { 0.4 } else { -0.4 };
            let next = keep_subscribed(subscribed, VOICE_RESUBSCRIBE + wobble);
            toggles += usize::from(next != subscribed);
            subscribed = next;
        }
        assert_eq!(toggles, 2);
        assert!(subscribed);
        // Never silent while subscribed-but-far: the gain is 0 beyond VOICE_FAR anyway.
        const { assert!(VOICE_RESUBSCRIBE >= VOICE_FAR && VOICE_UNSUBSCRIBE > VOICE_RESUBSCRIBE) };
    }
}
