//! The Voice tab's status line and player list (the toggles and the microphone dropdown are
//! `settings.rs`'s), the mic status line, and the indicators above heads (speech bubble: talking;
//! muted mic: muted by the world owner). Icons: `assets/ui/voice_*.png`.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use game_shared::physics::HITBOX_HEIGHT;
use game_shared::rules::PlayerId;

use super::{Link, VoiceState};
use crate::hud::{
    BottomRightHud, HUD_MARGIN, OpenPanel, PANEL_COLOR, SLOT_GAP, ink, label, tab_node,
};
use crate::net::Net;
use crate::settings::{Field, OpenDropdown, Settings, SettingsTab, TabBody};
use crate::state::Game;

/// Overhead icon: its centre this far above the hitbox top (clear of the nameplate), world px.
const ICON_ABOVE: f32 = 52.0;
const ICON_Z: f32 = 30.0;
/// HUD icons are drawn at their native 32 px; the button icons a little smaller.
const HUD_ICON: f32 = 32.0;
const BUTTON_ICON: f32 = 24.0;
const MIC_ON: &str = "ui/voice_mic_on.png";
const MIC_MUTED: &str = "ui/voice_mic_muted.png";
const MIC_TALKING: &str = "ui/voice_mic_talking.png";
const SPEAKER_ON: &str = "ui/voice_speaker_on.png";
const SPEAKER_MUTED: &str = "ui/voice_speaker_muted.png";
const SPEAKING: &str = "ui/voice_speaking.png";
const KEY_V: &str = "ui/voice_key_v.png";
/// How fast an indicator follows its player (server bodies arrive at 20 Hz).
const ICON_STIFFNESS: f32 = 20.0;

#[derive(Component, Clone, Copy)]
pub(super) enum VoiceButton {
    /// Mute for me only.
    LocalMute(PlayerId),
    /// World owner: mute for everyone (the server checks).
    ServerMute(PlayerId, bool),
}

#[derive(Component)]
pub(super) struct VoiceStatus;
#[derive(Component)]
pub(super) struct VoiceRows;
#[derive(Component)]
pub(super) struct MicStatus;
#[derive(Component)]
pub(super) struct VoiceIcon(PlayerId);

/// The status line and the player list, under the Voice tab's settings; the mic line, top-left.
pub(super) fn spawn_panel(
    mut commands: Commands,
    assets: Res<AssetServer>,
    bodies: Query<(Entity, &TabBody)>,
) {
    if let Some((page, _)) = bodies.iter().find(|(_, b)| b.0 == SettingsTab::Voice) {
        commands.entity(page).with_children(|page| {
            page.spawn((ink(""), VoiceStatus));
            page.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(SLOT_GAP),
                    ..default()
                },
                VoiceRows,
            ));
        });
    }
    // Bottom-right, level with the hotbar (docs/knowledge/ux/bottom-hud.md); `hud` keeps it above
    // the bag when the open bag would cover it.
    let status = Node {
        position_type: PositionType::Absolute,
        right: px(HUD_MARGIN),
        bottom: px(HUD_MARGIN),
        padding: UiRect::all(px(SLOT_GAP)),
        column_gap: px(SLOT_GAP),
        align_items: AlignItems::Center,
        display: Display::None,
        ..default()
    };
    commands
        .spawn((
            status,
            BackgroundColor(PANEL_COLOR),
            MicStatus,
            BottomRightHud,
        ))
        .with_children(|s| {
            s.spawn(icon(&assets, MIC_ON, HUD_ICON));
            s.spawn(label(""));
        });
}

fn name_of(net: Option<&Net>, id: PlayerId) -> String {
    net.and_then(|net| net.name(id))
        .map_or_else(|| format!("Player {}", id.0), str::to_owned)
}

fn icon(assets: &AssetServer, file: &'static str, size: f32) -> impl Bundle {
    let node = Node {
        width: px(size),
        height: px(size),
        ..default()
    };
    (node, ImageNode::new(assets.load(file)))
}

/// A tab-art button with an icon before its text.
fn button(
    row: &mut ChildSpawnerCommands,
    assets: &AssetServer,
    file: &'static str,
    text: &str,
    action: VoiceButton,
) {
    let (mut node, art) = tab_node(assets, true);
    node.column_gap = px(SLOT_GAP);
    row.spawn((node, art, action, Hovered::default()))
        .with_children(|b| {
            b.spawn(icon(assets, file, BUTTON_ICON));
            b.spawn(label(text));
        });
}

fn status_text(state: &VoiceState, settings: &Settings) -> String {
    match &state.link {
        _ if !state.online => "Voice needs an online game".into(),
        Link::Off if settings.voice => "Waiting for the server".into(),
        Link::Off => "Voice is off".into(),
        Link::Connecting => "Connecting...".into(),
        Link::Connected => "Connected".into(),
        Link::Failed(reason) => format!("Unavailable: {reason} (turn off and on to retry)"),
    }
}

/// The mic line, and while the Voice tab is on show its status line and player list, rebuilt
/// when what they show changes.
#[allow(clippy::too_many_arguments)] // a Bevy system: the status lines, the rows, and their inputs
pub(super) fn refresh_panel(
    mut commands: Commands,
    assets: Res<AssetServer>,
    open: Res<OpenPanel>,
    tab: Res<SettingsTab>,
    settings: Res<Settings>,
    state: Res<VoiceState>,
    game: Res<Game>,
    net: Option<Res<Net>>,
    rows: Single<Entity, With<VoiceRows>>,
    mut voice_status: Single<&mut Text, With<VoiceStatus>>,
    status: Single<(&mut Node, &Children), With<MicStatus>>,
    mut texts: Query<&mut Text, Without<VoiceStatus>>,
    mut images: Query<&mut ImageNode>,
    mut shown: Local<Vec<PlayerId>>,
) {
    let (mut node, children) = status.into_inner();
    let line = match (&state.link, settings.self_muted) {
        (Link::Connected, _) if state.server_muted.contains(&game.1) => {
            Some((MIC_MUTED, "Muted by the world owner"))
        }
        (Link::Connected, true) => Some((MIC_MUTED, "Muted")),
        (Link::Connected, false) if state.mic_open => Some((MIC_TALKING, "On air")),
        (Link::Connected, false) if settings.open_mic => Some((MIC_ON, "Open mic")),
        (Link::Connected, false) => Some((KEY_V, "Hold V to talk")),
        _ => None,
    };
    let display = if line.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != display {
        node.display = display;
    }
    if let (Some((file, line)), [image, text, ..]) = (line, &children[..]) {
        if let Ok(mut text) = texts.get_mut(*text)
            && text.0 != line
        {
            text.0 = line.into();
        }
        let handle = assets.load(file);
        if let Ok(mut image) = images.get_mut(*image)
            && image.image != handle
        {
            image.image = handle;
        }
    }

    if *open != OpenPanel::Settings || *tab != SettingsTab::Voice {
        return;
    }
    let mut others: Vec<PlayerId> = game
        .0
        .players
        .keys()
        .copied()
        .filter(|&id| id != game.1)
        .collect();
    others.sort_by_key(|id| id.0);
    let changed =
        open.is_changed() || tab.is_changed() || settings.is_changed() || state.is_changed();
    if !changed && *shown == others {
        return;
    }
    *shown = others.clone();
    let status = status_text(&state, &settings);
    if voice_status.0 != status {
        voice_status.0 = status;
    }
    let world = &game.0.world;
    let owner = world
        .world_claim
        .and_then(|key| world.claim(key))
        .map(|c| c.owner);
    let i_own_the_world = owner == Some(game.1);
    let net = net.as_deref();

    commands
        .entity(*rows)
        .despawn_children()
        .with_children(|rows| {
            rows.spawn(ink(if others.is_empty() {
                "Nobody else is here"
            } else {
                "Players in this world"
            }));
            for &id in &others {
                let row = Node {
                    column_gap: px(SLOT_GAP),
                    align_items: AlignItems::Center,
                    ..default()
                };
                rows.spawn(row).with_children(|r| {
                    if state.speaking.contains(&id) {
                        r.spawn(icon(&assets, SPEAKING, BUTTON_ICON));
                    }
                    r.spawn((
                        ink(name_of(net, id)),
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                    ));
                    let (file, text) = if state.local_muted.contains(&id) {
                        (SPEAKER_MUTED, "Unmute for me")
                    } else {
                        (SPEAKER_ON, "Mute for me")
                    };
                    button(r, &assets, file, text, VoiceButton::LocalMute(id));
                    if i_own_the_world {
                        let muted = state.server_muted.contains(&id);
                        let text = if muted {
                            "Unmute for all"
                        } else {
                            "Mute for all"
                        };
                        let action = VoiceButton::ServerMute(id, !muted);
                        button(r, &assets, MIC_MUTED, text, action);
                    }
                });
            }
        });
}

/// Per-player mute buttons; asks the voice thread for the microphones when the Voice tab first
/// shows and whenever the Microphone dropdown opens.
#[allow(clippy::too_many_arguments)] // a Bevy system: the buttons, three view states, two outputs
pub(super) fn click_panel(
    mouse: Res<ButtonInput<MouseButton>>,
    buttons: Query<(&VoiceButton, &Hovered)>,
    open: Res<OpenPanel>,
    tab: Res<SettingsTab>,
    dropdown: Res<OpenDropdown>,
    mut state: ResMut<VoiceState>,
    mut net: Option<ResMut<Net>>,
    mut asked_devices: Local<bool>,
) {
    let on_show = *open == OpenPanel::Settings && *tab == SettingsTab::Voice;
    let mic_list = dropdown.is_changed() && dropdown.0 == Some(Field::Mic);
    if state.online && ((on_show && !*asked_devices) || mic_list) {
        *asked_devices = true;
        state.request_devices = true;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(&action) = buttons.iter().find(|(_, h)| h.get()).map(|(b, _)| b) else {
        return;
    };
    match action {
        VoiceButton::LocalMute(id) => {
            if !state.local_muted.remove(&id) {
                state.local_muted.insert(id);
            }
        }
        VoiceButton::ServerMute(id, muted) => {
            if let Some(net) = net.as_mut() {
                net.voice_mute(id, muted);
            }
        }
    }
}

/// A speech bubble over every talking player, a muted mic over every player the owner muted.
pub(super) fn show_indicators(
    mut commands: Commands,
    assets: Res<AssetServer>,
    time: Res<Time>,
    game: Res<Game>,
    state: Res<VoiceState>,
    mut icons: Query<(Entity, &VoiceIcon, &mut Transform, &mut Sprite)>,
) {
    let target = |id: PlayerId| {
        let body = game.0.players.get(&id)?.body;
        Some(Vec3::new(
            body.pos.0,
            body.pos.1 + HITBOX_HEIGHT / 2.0 + ICON_ABOVE,
            ICON_Z,
        ))
    };
    let file = |id: PlayerId| {
        if state.server_muted.contains(&id) {
            Some(MIC_MUTED)
        } else if state.speaking.contains(&id) {
            Some(SPEAKING)
        } else {
            None
        }
    };
    let follow = 1.0 - (-ICON_STIFFNESS * time.delta_secs()).exp();
    let mut drawn = Vec::new();
    for (entity, icon, mut transform, mut sprite) in &mut icons {
        match (target(icon.0), file(icon.0)) {
            (Some(goal), Some(file)) => {
                let step = (goal - transform.translation) * follow;
                transform.translation += step;
                let handle = assets.load(file);
                if sprite.image != handle {
                    sprite.image = handle;
                }
                drawn.push(icon.0);
            }
            _ => commands.entity(entity).despawn(),
        }
    }
    let wanted = state.speaking.iter().chain(&state.server_muted);
    for &id in wanted {
        if drawn.contains(&id) {
            continue;
        }
        if let (Some(at), Some(file)) = (target(id), file(id)) {
            drawn.push(id);
            commands.spawn((
                Sprite::from_image(assets.load(file)),
                Transform::from_translation(at),
                VoiceIcon(id),
            ));
        }
    }
}
