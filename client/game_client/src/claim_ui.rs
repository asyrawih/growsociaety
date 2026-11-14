//! What the player sees of claims (docs/knowledge/ux/claims.md; rules in docs/knowledge/07):
//! the preview while holding a claim, the outline after placing, owner-naming refusals, the name
//! tag over a claim block, and the claim panel a wrench click opens (`interact::inspect`). It
//! decides nothing: the panel sends `SetClaimAccess` / `SetClaimPublic` and the rules answer; the
//! checks here only pick what to draw and say.

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::picking::Pickable;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use game_protocol::MAX_NAME_LEN;
use game_shared::claims::{Claim, MAX_CLAIM_ACCESS, claim_area, claim_kind};
use game_shared::items::{ItemKind, WORKBENCH, WRENCH, item};
use game_shared::mods::worn_mods;
use game_shared::rules::{Action, Event, PlayerId, Reject, in_reach_with};
use game_shared::world::{TileExtra, World};
use game_shared::{TILE_SIZE, tile_to_world};

use crate::audio::Sfx;
use crate::hud::{
    CLOSE_IMAGE, CRAFT_PANEL_KEY, HUD_MARGIN, INK_COLOR, Notice, OpenPanel, PANEL_IMAGE,
    PANEL_INSET, PANEL_PADDING, SEP, SLOT_GAP, SLOT_SELECTED_IMAGE, Selected, TOPBAR_HEIGHT,
    TextFocus, icon_node, ink, key_name, label, sliced,
};
use crate::interact::{HoveredTile, InspectedClaim};
use crate::net::Net;
use crate::player::offline_name;
use crate::settings::Settings;
use crate::state::{ActionMsg, ActionResult, Game, GameEvent, Phase};
use crate::world_view::{Temporary, Z_HIGHLIGHT};

/// Preview and outline colours, at the outline's 30 % alpha.
const CAN_COLOR: Color = Color::srgba(0.3, 0.8, 1.0, 0.3);
const FAR_COLOR: Color = Color::srgba(0.6, 0.6, 0.6, 0.3);
const NO_COLOR: Color = Color::srgba(1.0, 0.2, 0.2, 0.3);
/// The screen-edge frame for world claims: full strength, it is thin.
const FRAME_WIDTH: f32 = 6.0;
const FRAME_ALPHA: f32 = 0.8;
/// The frame while the wrench only points into a world claim: fainter, it is just a hint.
const HOVER_FRAME_ALPHA: f32 = 0.3;
/// The label beside the pointer: px right of and above it, so a finger does not cover it.
const LABEL_OFFSET: Vec2 = Vec2::new(16.0, -32.0);
const LABEL_PADDING: f32 = 4.0;
const LABEL_BACK: Color = Color::srgba(0.0, 0.0, 0.0, 0.6);
/// After placing: the outline stays, then fades.
const PLACED_OUTLINE_SECS: f32 = 3.0;
const PLACED_FADE_SECS: f32 = 0.5;
/// After a refusal: the claim's area (or the frame) flashes red this long.
const REFUSED_FLASH_SECS: f32 = 1.0;
/// At most one claim toast this often.
const TOAST_GAP_SECS: f32 = 2.0;
/// The tip waits for the placement toast to go first.
const TIP_DELAY_SECS: f32 = 3.0;
const TIP: &str = "Tip: hold the wrench and click your claim to manage it";

const TOGGLE_ON: &str = "ui/settings/toggle_on.png";
const TOGGLE_OFF: &str = "ui/settings/toggle_off.png";
const TOGGLE_SIZE: Vec2 = Vec2::new(57.0 * 1.5, 29.0 * 1.5);
const INPUT_IMAGE: &str = "ui/lobby/input.png";
const INPUT_INSET: f32 = 8.0;
const BUTTON_IMAGE: &str = "ui/lobby/button_primary.png";
const BUTTON_SECONDARY_IMAGE: &str = "ui/lobby/button_secondary.png";
const BUTTON_INSET: f32 = 10.0;
const FOCUS_INSET: f32 = 8.0;
const PANEL_WIDTH: f32 = 380.0;
/// Rows, chips and buttons: a touch target.
const ROW_HEIGHT: f32 = 44.0;
const REMOVE_SIZE: f32 = 32.0;
const BUTTON_WIDTH: f32 = 88.0;
const FIELD_PLACEHOLDER: &str = "Player name";
const PLACEHOLDER_COLOR: Color = Color::srgba(0.29, 0.17, 0.08, 0.5);
const ERROR_COLOR: Color = Color::srgb(0.7, 0.1, 0.08);
const HOVER_TINT: Color = Color::srgb(0.85, 0.85, 0.85);
const REMOVE_HOVER_TINT: Color = Color::srgb(1.0, 0.4, 0.4);
const DISABLED_TINT: Color = Color::srgba(1.0, 1.0, 1.0, 0.5);
/// Panel keys (the name field takes the rest while it has focus). Enter is the chat line's; no
/// Space binding, so the player still walks and jumps with the panel open. Tab focuses the name
/// field or releases it; Esc releases it, or closes the panel when nothing has focus.
const FOCUS_KEY: KeyCode = KeyCode::Tab;
const CLOSE_KEY: KeyCode = KeyCode::Escape;
const OFFLINE_ADD: &str = "Adding players needs an online game";

/// What holding a claim over the hovered tile would do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Look {
    Place,
    TooFar,
    /// The tile is inside an area claim.
    Claimed,
    /// An area claim has no free tile left to take here.
    NoFreeTiles,
    /// Inside someone's world claim.
    NoPermission(PlayerId),
    WorldPlace,
    WorldTaken,
    WorldForeign,
}

impl Look {
    fn is_world(self) -> bool {
        matches!(
            self,
            Look::WorldPlace | Look::WorldTaken | Look::WorldForeign
        )
    }
}

/// The preview: its look and the tiles (keys) it draws.
#[derive(PartialEq, Debug)]
struct Preview {
    look: Look,
    tiles: Vec<u32>,
}

/// The claim block key governing `(x, y)`: the area claim owning it, else the world claim.
fn governing_claim(world: &World, x: u32, y: u32) -> Option<u32> {
    let key = World::tile_key(x, y)?;
    world.claim_of.get(&key).copied().or(world.world_claim)
}

/// Who owns the claim that protects `(x, y)`, from the mirrored world.
fn owner_at(world: &World, x: u32, y: u32) -> Option<PlayerId> {
    let claim = world.claim(governing_claim(world, x, y)?)?;
    Some(claim.owner)
}

/// A name to show: the known one, else `Player #<id>` until the lookup answers.
fn display_name(known: Option<&str>, id: PlayerId) -> String {
    known.map_or_else(|| format!("Player #{}", id.0), str::to_owned)
}

/// The tag over a placed Workbench (docs/knowledge/ux/workbench.md).
fn workbench_tag() -> String {
    format!("Workbench{SEP}press {} to craft", key_name(CRAFT_PANEL_KEY))
}

/// What holding `held` over `(x, y)` would do, for `me`. `None` for a non-claim item.
fn preview(
    world: &World,
    me: PlayerId,
    held: game_shared::items::ItemId,
    (x, y): (u32, u32),
    reachable: bool,
) -> Option<Preview> {
    let kind = claim_kind(held)?;
    let key = World::tile_key(x, y)?;
    let look = |look| {
        Some(Preview {
            look,
            tiles: Vec::new(),
        })
    };
    let Some(count) = kind.tiles else {
        if !reachable {
            return look(Look::TooFar);
        }
        if world.world_claim.is_some() {
            return look(Look::WorldTaken);
        }
        let foreign = |extra: &TileExtra| matches!(extra, TileExtra::Claim(c) if c.owner != me && !c.tiles.is_empty());
        return look(if world.extras.values().any(foreign) {
            Look::WorldForeign
        } else {
            Look::WorldPlace
        });
    };
    if let Some(&block) = world.claim_of.get(&key) {
        let tiles = world
            .claim(block)
            .map(|c| c.tiles.clone())
            .unwrap_or_default();
        let look = if reachable {
            Look::Claimed
        } else {
            Look::TooFar
        };
        return Some(Preview { look, tiles });
    }
    let tiles = claim_area(world, x, y, count);
    let world_owner = world.world_claim.and_then(|k| world.claim(k));
    let look = if !reachable {
        Look::TooFar
    } else if let Some(c) = world_owner.filter(|c| c.owner != me && !c.access.contains(&me)) {
        Look::NoPermission(c.owner)
    } else if tiles.is_empty() {
        Look::NoFreeTiles
    } else {
        Look::Place
    };
    Some(Preview { look, tiles })
}

fn preview_color(look: Look) -> Color {
    match look {
        Look::Place | Look::WorldPlace => CAN_COLOR,
        Look::TooFar => FAR_COLOR,
        _ => NO_COLOR,
    }
}

/// "small claim" -> "Small claim".
fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The label beside the pointer. `owner` names the `NoPermission` owner.
fn preview_label(preview: &Preview, held: game_shared::items::ItemId, owner: &str) -> String {
    match preview.look {
        Look::Place => format!(
            "{} · {} tiles",
            capitalised(item(held).map_or("claim", |d| d.name)),
            preview.tiles.len()
        ),
        Look::TooFar => "Too far".to_string(),
        Look::Claimed | Look::WorldTaken => "Already claimed".to_string(),
        Look::NoFreeTiles => "No free tiles left here".to_string(),
        Look::NoPermission(_) => format!("This world belongs to {owner}"),
        Look::WorldPlace => "World claim · the whole world".to_string(),
        Look::WorldForeign => "Someone else has a claim here".to_string(),
    }
}

/// The toast for a refused punch or placement at `(x, y)`, if the spec gives one. `name` names a
/// player; `held` is what was placed (`None` for a punch).
fn refusal_toast(
    world: &World,
    reject: Reject,
    (x, y): (u32, u32),
    placed: Option<game_shared::items::ItemId>,
    name: impl Fn(PlayerId) -> String,
) -> Option<String> {
    let key = World::tile_key(x, y)?;
    match reject {
        Reject::NoPermission => {
            let owner = name(owner_at(world, x, y)?);
            Some(if world.claim_of.contains_key(&key) {
                format!("Locked by {owner}")
            } else {
                format!("This world is locked by {owner}")
            })
        }
        Reject::NotOwner => Some(format!(
            "Only {} can do that",
            name(world.claim(key)?.owner)
        )),
        Reject::Occupied => {
            let count = claim_kind(placed?)?.tiles;
            let full = count.is_some_and(|n| {
                !world.claim_of.contains_key(&key) && claim_area(world, x, y, n).is_empty()
            });
            Some(if full {
                "No free tiles left here".to_string()
            } else {
                "That spot is already claimed".to_string()
            })
        }
        _ => None,
    }
}

/// Why the typed name cannot be added, from the mirrored claim (the server checks again).
fn add_error(claim: &Claim, id: PlayerId, name: &str) -> Option<String> {
    if id == claim.owner {
        Some("You are the owner".to_string())
    } else if claim.access.contains(&id) {
        Some(format!("{name} can already build here"))
    } else if claim.access.len() >= MAX_CLAIM_ACCESS {
        Some(format!("The list is full ({MAX_CLAIM_ACCESS})"))
    } else {
        None
    }
}

/// Names for the view: the server's online, the local player's own offline.
fn name_of(net: Option<&Net>, me: PlayerId, id: PlayerId) -> String {
    match net {
        Some(net) => display_name(net.name(id), id),
        None if id == me => offline_name(),
        None => display_name(None, id),
    }
}

#[derive(Component)]
struct PreviewTile;

#[derive(Component)]
struct CursorLabel;

#[derive(Component)]
struct EdgeFrame;

/// A placed claim's outline: shown until `fade_at`, then fading out over `PLACED_FADE_SECS`.
#[derive(Component)]
struct FadeOutline {
    fade_at: f32,
}

/// The inspected claim's outline (claim panel open).
#[derive(Component)]
struct PanelOutline;

/// Claim feedback timing.
#[derive(Resource, Default)]
struct ClaimFeedback {
    /// No claim toast before this.
    next_toast: f32,
    /// The frame is red until this (a refusal in a world claim).
    frame_flash_until: f32,
    /// The first-placement tip goes out at this time (once ever: `Settings::claim_tip_seen`).
    tip_at: Option<f32>,
}

/// What a click (or the focus + activate key) on a claim panel element does.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum ClaimButton {
    Close,
    Public,
    ConfirmYes,
    ConfirmNo,
    Remove(PlayerId),
    Chip(PlayerId),
    Field,
    Add,
}

/// The claim panel's view state.
#[derive(Resource, Default)]
struct PanelState {
    field: String,
    error: Option<String>,
    /// Index into the focus order; nothing when the panel opens.
    focus: Option<usize>,
    confirm_public: bool,
    /// The name sent to `Net::find_player`, waiting for its answer.
    asked: Option<String>,
}

/// Panel keys, read before the game sees them.
#[derive(Resource, Default)]
struct PanelKeys {
    typed: Vec<Key>,
    /// Gamepad d-pad steps through the focus order.
    step: i32,
    /// Tab: focus the name field, or release the focus.
    toggle_field: bool,
    activate: bool,
    close: bool,
}

#[derive(Component)]
struct ClaimPanel;
#[derive(Component)]
struct PanelBody;
#[derive(Component)]
struct FieldText;

pub struct ClaimUiPlugin;

impl Plugin for ClaimUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClaimFeedback>()
            .init_resource::<PanelState>()
            .init_resource::<PanelKeys>()
            .init_resource::<WorldPreview>()
            .add_systems(Startup, spawn_claim_ui)
            // After `PreUpdate` (the chat line and screenshot runs clear the keys there), before
            // the player's `FixedUpdate` would walk on them.
            .add_systems(
                RunFixedMainLoop,
                take_panel_keys.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .add_systems(
                Update,
                (click_panel, use_panel_keys).chain().in_set(Phase::Input),
            )
            .add_systems(
                Update,
                (
                    draw_preview,
                    claim_feedback,
                    fade_outlines,
                    edge_frame,
                    (answer_found, refresh_panel, outline_inspected, field_text).chain(),
                )
                    .in_set(Phase::View),
            );
    }
}

fn spawn_claim_ui(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn((
        label(""),
        CursorLabel,
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::all(px(LABEL_PADDING)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(LABEL_BACK),
        GlobalZIndex(i32::MAX - 2),
        Pickable::IGNORE,
        Hovered::default(),
    ));
    commands.spawn((
        EdgeFrame,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            border: UiRect::all(px(FRAME_WIDTH)),
            display: Display::None,
            ..default()
        },
        BorderColor::all(CAN_COLOR),
        Pickable::IGNORE,
        Hovered::default(),
    ));
    // Top-right under the top bar, where the old claim panel was.
    let panel = Node {
        position_type: PositionType::Absolute,
        right: px(HUD_MARGIN),
        top: px(TOPBAR_HEIGHT + 2.0 * HUD_MARGIN),
        width: px(PANEL_WIDTH),
        padding: UiRect::all(px(PANEL_PADDING)),
        flex_direction: FlexDirection::Column,
        display: Display::None,
        ..default()
    };
    commands
        .spawn((
            panel,
            ClaimPanel,
            Hovered::default(),
            sliced(&assets, PANEL_IMAGE, PANEL_INSET),
        ))
        .with_child((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(HUD_MARGIN),
                ..default()
            },
            PanelBody,
            Hovered::default(),
        ));
}

fn tile_sprite(key: u32, color: Color) -> Option<(Sprite, Transform)> {
    let (x, y) = World::tile_xy(key)?;
    let (cx, cy) = tile_to_world(x, y);
    Some((
        Sprite::from_color(color, Vec2::splat(TILE_SIZE)),
        Transform::from_xyz(cx, cy, Z_HIGHLIGHT),
    ))
}

/// While a claim item is selected: the tiles it would take and a label beside the pointer.
/// Otherwise, over a claim block: `<owner>'s claim` above it. Redrawn only when the hovered tile,
/// the item, the reach or the claims change.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn draw_preview(
    mut commands: Commands,
    game: Res<Game>,
    selected: Res<Selected>,
    hovered: Res<HoveredTile>,
    mut net: Option<ResMut<Net>>,
    mut events: MessageReader<GameEvent>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    tiles: Query<Entity, With<PreviewTile>>,
    cursor_label: Single<(&mut Text, &mut Node), With<CursorLabel>>,
    mut world_look: ResMut<WorldPreview>,
    mut shown: Local<Option<(Option<(u32, u32)>, Option<game_shared::items::ItemId>, bool)>>,
) {
    let claims_changed = events.read().any(|GameEvent(e)| {
        matches!(
            e,
            Event::ClaimPlaced { .. } | Event::ClaimRemoved { .. } | Event::ClaimUpdated { .. }
        )
    });
    let (mut text, mut node) = cursor_label.into_inner();
    let held = selected
        .item(&game)
        .filter(|&id| item(id).is_some_and(|d| d.kind == ItemKind::Claim));
    let reachable = match (hovered.0, game.me()) {
        (Some((x, y)), Some(me)) => in_reach_with(&me.body, x, y, worn_mods(&me.equipped)),
        _ => false,
    };
    let key = (hovered.0, held, reachable);
    let redraw = claims_changed || *shown != Some(key);
    let me = game.1;
    let world = &game.0.world;

    // Where the label goes this frame, and what it says (only rewritten on a redraw).
    let mut at = None;
    if let (Some(held), Some(tile)) = (held, hovered.0) {
        at = window.cursor_position().map(|c| c + LABEL_OFFSET);
        if redraw {
            for entity in &tiles {
                commands.entity(entity).despawn();
            }
            let preview = preview(world, me, held, tile, reachable);
            let look = preview.as_ref().map(|p| p.look).filter(|l| l.is_world());
            world_look.set_if_neq(WorldPreview(look));
            if let Some(preview) = preview {
                let colour = preview_color(preview.look);
                for &k in &preview.tiles {
                    if let Some(sprite) = tile_sprite(k, colour) {
                        commands.spawn((PreviewTile, sprite.0, sprite.1));
                    }
                }
                let owner = match preview.look {
                    Look::NoPermission(id) => {
                        if let Some(net) = net.as_deref_mut() {
                            net.lookup_names([id]);
                        }
                        name_of(net.as_deref(), me, id)
                    }
                    _ => String::new(),
                };
                text.0 = preview_label(&preview, held, &owner);
            }
        }
    } else {
        if redraw {
            for entity in &tiles {
                commands.entity(entity).despawn();
            }
            world_look.set_if_neq(WorldPreview(None));
        }
        // The name tag over a claim block, or over a Workbench how to use it.
        let block = hovered
            .0
            .and_then(|(x, y)| Some(((x, y), world.claim(World::tile_key(x, y)?)?.owner)));
        let tagged = match block {
            Some((tile, owner)) => {
                if let Some(net) = net.as_deref_mut() {
                    net.lookup_names([owner]);
                }
                Some((
                    tile,
                    format!("{}'s claim", name_of(net.as_deref(), me, owner)),
                ))
            }
            None => hovered
                .0
                .filter(|&(x, y)| world.get(x, y).is_some_and(|t| t.fg == WORKBENCH))
                .map(|tile| (tile, workbench_tag())),
        };
        if let Some(((x, y), tag)) = tagged {
            if text.0 != tag {
                text.0 = tag;
            }
            let (camera, transform) = *camera;
            let (cx, cy) = tile_to_world(x, y);
            let top = Vec3::new(cx, cy + TILE_SIZE, Z_HIGHLIGHT);
            at = camera.world_to_viewport(transform, top).ok();
        }
    }
    *shown = Some(key);
    match at {
        Some(at) => {
            if node.display != Display::Flex {
                node.display = Display::Flex;
            }
            if node.left != px(at.x) || node.top != px(at.y) {
                node.left = px(at.x);
                node.top = px(at.y);
            }
        }
        None => {
            if node.display != Display::None {
                node.display = Display::None;
            }
        }
    }
}

/// The world-claim preview on show, if any (drawn by `edge_frame`).
#[derive(Resource, Default, PartialEq)]
struct WorldPreview(Option<Look>);

/// The screen-edge frame: a world-claim preview, a refusal in a world claim (red), an open panel
/// on a world claim, or the wrench pointing into a world claim (outside any area claim).
#[allow(clippy::too_many_arguments)] // a Bevy system
fn edge_frame(
    time: Res<Time>,
    game: Res<Game>,
    selected: Res<Selected>,
    hovered: Res<HoveredTile>,
    preview: Res<WorldPreview>,
    feedback: Res<ClaimFeedback>,
    inspected: Res<InspectedClaim>,
    frame: Single<(&mut Node, &mut BorderColor), With<EdgeFrame>>,
) {
    let (mut node, mut border) = frame.into_inner();
    let world = &game.0.world;
    let pointed = (selected.item(&game) == Some(WRENCH))
        .then(|| {
            hovered
                .0
                .and_then(|(x, y)| wrench_target(world, x, y).or(world.world_claim))
        })
        .flatten();
    let world_claim_admits = |key: Option<u32>| {
        key.and_then(|key| world.claim(key))
            .filter(|c| c.tiles.is_empty())
            .map(|c| {
                if c.admits(game.1) {
                    CAN_COLOR
                } else {
                    NO_COLOR
                }
            })
    };
    let opened = inspected.0.and_then(|(x, y)| World::tile_key(x, y));
    let colour = if time.elapsed_secs() < feedback.frame_flash_until {
        Some((NO_COLOR, FRAME_ALPHA))
    } else if let Some(look) = preview.0 {
        Some((preview_color(look), FRAME_ALPHA))
    } else if opened.is_some() {
        world_claim_admits(opened).map(|c| (c, FRAME_ALPHA))
    } else {
        world_claim_admits(pointed).map(|c| (c, HOVER_FRAME_ALPHA))
    };
    let display = if colour.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != display {
        node.display = display;
    }
    if let Some((colour, alpha)) = colour {
        let colour = BorderColor::all(colour.with_alpha(alpha));
        if *border != colour {
            *border = colour;
        }
    }
}

/// Placing and refusals: toasts, the placed outline, the red area flash and the first-claim tip.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn claim_feedback(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    mut net: Option<ResMut<Net>>,
    mut results: MessageReader<ActionResult>,
    mut feedback: ResMut<ClaimFeedback>,
    mut settings: ResMut<Settings>,
    mut notices: MessageWriter<Notice>,
) {
    let now = time.elapsed_secs();
    let me = game.1;
    let world = &game.0.world;
    for ActionResult { action, result } in results.read() {
        let (tile, placed) = match *action {
            Action::Punch { x, y } => ((x, y), None),
            Action::Place { x, y, item } => ((x, y), Some(item)),
            _ => continue,
        };
        match result {
            Ok(events) => {
                let Some(claim) = events.iter().find_map(|e| match e {
                    Event::ClaimPlaced { claim, .. } if claim.owner == me => Some(claim),
                    _ => None,
                }) else {
                    continue;
                };
                let text = if claim.tiles.is_empty() {
                    "World claimed: every tile is yours".to_string()
                } else {
                    format!("Claim placed: {} tiles are yours", claim.tiles.len())
                };
                notices.write(Notice(text));
                feedback.next_toast = now + TOAST_GAP_SECS;
                for &k in &claim.tiles {
                    if let Some((sprite, at)) = tile_sprite(k, CAN_COLOR) {
                        let fade_at = now + PLACED_OUTLINE_SECS;
                        let until = fade_at + PLACED_FADE_SECS;
                        commands.spawn((sprite, at, FadeOutline { fade_at }, Temporary { until }));
                    }
                }
                if !settings.claim_tip_seen && feedback.tip_at.is_none() {
                    feedback.tip_at = Some(now + TIP_DELAY_SECS);
                }
            }
            Err(reject) => {
                if *reject == Reject::NoPermission {
                    // Show where the player may not build: the area, or the frame for a world claim.
                    let area = World::tile_key(tile.0, tile.1)
                        .and_then(|key| world.claim_of.get(&key))
                        .and_then(|&block| world.claim(block));
                    match area {
                        Some(claim) => {
                            for &k in &claim.tiles {
                                if let Some((sprite, at)) = tile_sprite(k, NO_COLOR) {
                                    let until = now + REFUSED_FLASH_SECS;
                                    commands.spawn((sprite, at, Temporary { until }));
                                }
                            }
                        }
                        None => feedback.frame_flash_until = now + REFUSED_FLASH_SECS,
                    }
                }
                if now < feedback.next_toast {
                    continue;
                }
                if let Some(net) = net.as_deref_mut()
                    && let Some(owner) = owner_at(world, tile.0, tile.1)
                {
                    net.lookup_names([owner]);
                }
                let name = |id| name_of(net.as_deref(), me, id);
                if let Some(text) = refusal_toast(world, *reject, tile, placed, name) {
                    notices.write(Notice(text));
                    feedback.next_toast = now + TOAST_GAP_SECS;
                }
            }
        }
    }
    if feedback.tip_at.is_some_and(|at| now >= at) {
        feedback.tip_at = None;
        settings.claim_tip_seen = true;
        notices.write(Notice(TIP.to_string()));
    }
}

fn fade_outlines(time: Res<Time>, mut outlines: Query<(&FadeOutline, &mut Sprite)>) {
    let now = time.elapsed_secs();
    for (outline, mut sprite) in &mut outlines {
        let left = 1.0 - ((now - outline.fade_at) / PLACED_FADE_SECS).clamp(0.0, 1.0);
        let alpha = CAN_COLOR.alpha() * left;
        if sprite.color.alpha() != alpha {
            sprite.color.set_alpha(alpha);
        }
    }
}

/// The claim block key a wrench over `(x, y)` points at: the area claim owning the tile, or the
/// claim block itself.
fn wrench_target(world: &World, x: u32, y: u32) -> Option<u32> {
    let key = World::tile_key(x, y)?;
    world
        .claim_of
        .get(&key)
        .copied()
        .or_else(|| world.claim(key).map(|_| key))
}

/// One claim's tiles: the inspected one while the panel is open (pinned), else the one the held
/// wrench points at. Cyan if the player may build there, else red.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn outline_inspected(
    mut commands: Commands,
    game: Res<Game>,
    selected: Res<Selected>,
    hovered: Res<HoveredTile>,
    inspected: Res<InspectedClaim>,
    mut events: MessageReader<GameEvent>,
    outline: Query<Entity, With<PanelOutline>>,
    mut shown: Local<Option<u32>>,
) {
    let changed = events.read().any(|GameEvent(e)| {
        matches!(
            e,
            Event::ClaimUpdated { .. } | Event::ClaimRemoved { .. } | Event::ClaimPlaced { .. }
        )
    });
    let world = &game.0.world;
    let pinned = inspected.0.and_then(|(x, y)| World::tile_key(x, y));
    let pointed = (selected.item(&game) == Some(WRENCH))
        .then(|| hovered.0.and_then(|(x, y)| wrench_target(world, x, y)))
        .flatten();
    let target = pinned.or(pointed);
    if !changed && *shown == target {
        return;
    }
    *shown = target;
    for entity in &outline {
        commands.entity(entity).despawn();
    }
    let Some(claim) = target.and_then(|key| world.claim(key)) else {
        return;
    };
    let colour = if claim.admits(game.1) {
        CAN_COLOR
    } else {
        NO_COLOR
    };
    for &k in &claim.tiles {
        if let Some((sprite, at)) = tile_sprite(k, colour) {
            commands.spawn((PanelOutline, sprite, at));
        }
    }
}

/// The focusable elements of the owner's panel, in Tab order: toggle, rows, chips, field, Add.
fn focus_order(claim: &Claim, chips: &[PlayerId]) -> Vec<ClaimButton> {
    let mut order = vec![ClaimButton::Public];
    order.extend(claim.access.iter().map(|&id| ClaimButton::Remove(id)));
    order.extend(chips.iter().map(|&id| ClaimButton::Chip(id)));
    order.extend([ClaimButton::Field, ClaimButton::Add]);
    order
}

/// Players in the world who are neither the owner nor listed.
fn chips_for(game: &Game, claim: &Claim) -> Vec<PlayerId> {
    let mut ids: Vec<PlayerId> = game
        .0
        .players
        .keys()
        .copied()
        .filter(|id| *id != claim.owner && !claim.access.contains(id))
        .collect();
    ids.sort_by_key(|id| id.0);
    ids
}

/// The inspected claim, if the panel is open on one.
fn open_claim<'a>(
    game: &'a Game,
    open: &OpenPanel,
    inspected: &InspectedClaim,
) -> Option<(u32, u32, &'a Claim)> {
    if *open != OpenPanel::Claim {
        return None;
    }
    let (x, y) = inspected.0?;
    Some((x, y, game.0.world.claim(World::tile_key(x, y)?)?))
}

/// Typed text and panel keys while the owner's panel is open. With the name field focused the
/// keyboard is the field's, so nothing reaches the game. Keys the chat line or a screenshot run
/// cleared are not taken.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn take_panel_keys(
    game: Res<Game>,
    open: Res<OpenPanel>,
    inspected: Res<InspectedClaim>,
    state: Res<PanelState>,
    mut typed: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut out: ResMut<PanelKeys>,
) {
    let typed: Vec<KeyboardInput> = typed.read().cloned().collect();
    let Some((_, _, claim)) = open_claim(&game, &open, &inspected) else {
        return;
    };
    if claim.owner != game.1 {
        return; // the read-only view: Esc (hud) closes it
    }
    let pad = |button| pads.iter().any(|pad| pad.just_pressed(button));
    out.toggle_field |= keys.just_pressed(FOCUS_KEY);
    if pad(GamepadButton::DPadDown) {
        out.step += 1;
    }
    if pad(GamepadButton::DPadUp) {
        out.step -= 1;
    }
    out.close |= keys.just_pressed(CLOSE_KEY) || pad(GamepadButton::East);
    out.activate |= pad(GamepadButton::South);
    let field_focused = state.focus.is_some_and(|i| {
        let chips = chips_for(&game, claim);
        focus_order(claim, &chips).get(i) == Some(&ClaimButton::Field)
    });
    if field_focused {
        for key in typed.iter().filter(|k| k.state == ButtonState::Pressed) {
            if keys.pressed(key.key_code) && key.logical_key != Key::Tab {
                out.typed.push(key.logical_key.clone());
            }
        }
        keys.reset_all();
    } else {
        for code in [FOCUS_KEY, CLOSE_KEY] {
            if keys.pressed(code) {
                keys.reset(code);
            }
        }
    }
}

/// A claim panel button, by click or by focus + activate.
fn press(
    button: ClaimButton,
    (x, y, claim): (u32, u32, &Claim),
    state: &mut PanelState,
    net: Option<&mut Net>,
    open: &mut OpenPanel,
    actions: &mut MessageWriter<ActionMsg>,
) {
    let access = |player, allow| {
        ActionMsg(Action::SetClaimAccess {
            x,
            y,
            player,
            allow,
        })
    };
    match button {
        ClaimButton::Close => *open = OpenPanel::None,
        ClaimButton::Public if claim.public => {
            actions.write(ActionMsg(Action::SetClaimPublic {
                x,
                y,
                public: false,
            }));
        }
        ClaimButton::Public => state.confirm_public = true,
        ClaimButton::ConfirmYes => {
            state.confirm_public = false;
            actions.write(ActionMsg(Action::SetClaimPublic { x, y, public: true }));
        }
        ClaimButton::ConfirmNo => state.confirm_public = false,
        ClaimButton::Remove(id) => {
            actions.write(access(id, false));
        }
        ClaimButton::Chip(id) => {
            if claim.access.len() >= MAX_CLAIM_ACCESS {
                state.error = Some(format!("The list is full ({MAX_CLAIM_ACCESS})"));
            } else {
                actions.write(access(id, true));
            }
        }
        ClaimButton::Field => {}
        ClaimButton::Add => {
            let name = state.field.trim().to_string();
            if name.is_empty() {
                return;
            }
            if claim.access.len() >= MAX_CLAIM_ACCESS {
                state.error = Some(format!("The list is full ({MAX_CLAIM_ACCESS})"));
                return;
            }
            match net {
                Some(net) => {
                    net.find_player(&name);
                    state.asked = Some(name);
                }
                None => state.error = Some(OFFLINE_ADD.to_string()),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system
fn click_panel(
    mouse: Res<ButtonInput<MouseButton>>,
    game: Res<Game>,
    inspected: Res<InspectedClaim>,
    buttons: Query<(&ClaimButton, &Hovered)>,
    mut open: ResMut<OpenPanel>,
    mut state: ResMut<PanelState>,
    mut net: Option<ResMut<Net>>,
    mut actions: MessageWriter<ActionMsg>,
    mut sfx: MessageWriter<Sfx>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some((x, y, claim)) = open_claim(&game, &open, &inspected) else {
        return;
    };
    let clicked = buttons.iter().find(|(_, h)| h.get()).map(|(&b, _)| b);
    // Only a click on the name field gives focus; any other click, inside or outside the panel,
    // releases it.
    let chips = chips_for(&game, claim);
    let focus = clicked
        .filter(|&b| b == ClaimButton::Field)
        .and_then(|b| focus_order(claim, &chips).iter().position(|&o| o == b));
    if state.focus != focus {
        state.focus = focus;
    }
    let Some(button) = clicked else {
        return;
    };
    sfx.write(Sfx::UiClick);
    let claim = claim.clone();
    let mut panel = *open;
    press(
        button,
        (x, y, &claim),
        &mut state,
        net.as_deref_mut(),
        &mut panel,
        &mut actions,
    );
    open.set_if_neq(panel);
}

#[allow(clippy::too_many_arguments)] // a Bevy system
fn use_panel_keys(
    game: Res<Game>,
    inspected: Res<InspectedClaim>,
    mut keys: ResMut<PanelKeys>,
    mut open: ResMut<OpenPanel>,
    mut state: ResMut<PanelState>,
    mut net: Option<ResMut<Net>>,
    mut actions: MessageWriter<ActionMsg>,
) {
    let PanelKeys {
        typed,
        step,
        toggle_field,
        activate,
        close,
    } = std::mem::take(&mut *keys);
    let Some((x, y, claim)) = open_claim(&game, &open, &inspected) else {
        return;
    };
    let claim = claim.clone();
    if close {
        // Esc releases the focus first; with nothing focused it closes the panel.
        if state.focus.is_some() {
            state.focus = None;
        } else {
            *open = OpenPanel::None;
        }
        return;
    }
    let chips = chips_for(&game, &claim);
    let order = focus_order(&claim, &chips);
    let field = order.iter().position(|&b| b == ClaimButton::Field);
    if toggle_field {
        state.focus = if state.focus == field { None } else { field };
    }
    if step != 0 {
        let len = order.len() as i32;
        let at = state.focus.map_or(0, |i| i as i32);
        state.focus = Some((at + step).rem_euclid(len) as usize);
    }
    let focused = state.focus.and_then(|i| order.get(i).copied());
    for key in typed {
        match key {
            Key::Enter => {
                let mut panel = *open;
                press(
                    ClaimButton::Add,
                    (x, y, &claim),
                    &mut state,
                    net.as_deref_mut(),
                    &mut panel,
                    &mut actions,
                );
                open.set_if_neq(panel);
            }
            Key::Escape => state.focus = None,
            Key::Backspace => {
                state.field.pop();
            }
            Key::Character(text) => {
                for c in text
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                {
                    if state.field.len() < MAX_NAME_LEN {
                        state.field.push(c);
                        state.error = None;
                    }
                }
            }
            _ => {}
        }
    }
    if activate && let Some(button) = focused {
        let button = if button == ClaimButton::Field {
            ClaimButton::Add
        } else {
            button
        };
        let mut panel = *open;
        press(
            button,
            (x, y, &claim),
            &mut state,
            net.as_deref_mut(),
            &mut panel,
            &mut actions,
        );
        open.set_if_neq(panel);
    }
}

/// The server's answer to the typed name: add the player, or say why not.
fn answer_found(
    game: Res<Game>,
    open: Res<OpenPanel>,
    inspected: Res<InspectedClaim>,
    mut net: Option<ResMut<Net>>,
    mut state: ResMut<PanelState>,
    mut actions: MessageWriter<ActionMsg>,
) {
    let Some(net) = net.as_deref_mut() else {
        return;
    };
    let found = net.take_found();
    let Some((x, y, claim)) = open_claim(&game, &open, &inspected) else {
        return;
    };
    for (asked, found) in found {
        if state.asked.as_deref() != Some(asked.as_str()) {
            continue;
        }
        state.asked = None;
        match found {
            None => state.error = Some(format!("No player called {asked}")),
            Some((id, name)) => match add_error(claim, id, &name) {
                Some(error) => state.error = Some(error),
                None => {
                    actions.write(ActionMsg(Action::SetClaimAccess {
                        x,
                        y,
                        player: id,
                        allow: true,
                    }));
                    state.field.clear();
                    state.error = None;
                }
            },
        }
    }
}

/// Everything the panel shows except the field's text; the body is rebuilt when it changes.
#[derive(PartialEq, Clone, Default)]
struct PanelView {
    claim: Option<(u32, u32, Claim)>,
    mine: bool,
    names: Vec<String>,
    chips: Vec<(PlayerId, String)>,
    error: Option<String>,
    focus: Option<ClaimButton>,
    confirm: bool,
    can_add: bool,
}

#[allow(clippy::too_many_arguments)] // a Bevy system
fn refresh_panel(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    open: Res<OpenPanel>,
    inspected: Res<InspectedClaim>,
    mut net: Option<ResMut<Net>>,
    mut state: ResMut<PanelState>,
    mut text_focus: ResMut<TextFocus>,
    search: Res<crate::inventory_filter::InventoryView>,
    panel: Single<&mut Node, With<ClaimPanel>>,
    body: Single<Entity, With<PanelBody>>,
    mut shown: Local<PanelView>,
) {
    let mut panel = panel.into_inner();
    let me = game.1;
    let Some((x, y, claim)) = open_claim(&game, &open, &inspected) else {
        if panel.display != Display::None {
            panel.display = Display::None;
            *state = PanelState::default();
            *shown = PanelView::default();
        }
        // The one writer of `TextFocus`: the craft panel, the store and the almanac keep Enter too
        // (it crafts, buys or searches there), and so does the inventory's search field and the
        // avatar menu (Enter chooses the focused row). One panel is open at a time.
        text_focus.set_if_neq(TextFocus(
            matches!(
                *open,
                OpenPanel::Craft
                    | OpenPanel::Store
                    | OpenPanel::Almanac
                    | OpenPanel::WorldSelect
                    | OpenPanel::AvatarMenu
            ) || search.focused,
        ));
        return;
    };
    if panel.display != Display::Flex {
        panel.display = Display::Flex;
    }
    let mine = claim.owner == me;
    let chips = if mine {
        chips_for(&game, claim)
    } else {
        Vec::new()
    };
    if let Some(net) = net.as_deref_mut() {
        net.lookup_names(std::iter::once(claim.owner).chain(claim.access.iter().copied()));
    }
    let name = |id| name_of(net.as_deref(), me, id);
    let order = focus_order(claim, &chips);
    let field_focused = mine && state.focus.and_then(|i| order.get(i)) == Some(&ClaimButton::Field);
    text_focus.set_if_neq(TextFocus(field_focused));
    let view = PanelView {
        claim: Some((x, y, claim.clone())),
        mine,
        names: std::iter::once(claim.owner)
            .chain(claim.access.iter().copied())
            .map(name)
            .collect(),
        chips: chips.iter().map(|&id| (id, name(id))).collect(),
        error: state.error.clone(),
        focus: if mine {
            state.focus.and_then(|i| order.get(i).copied())
        } else {
            None
        },
        confirm: state.confirm_public,
        can_add: !state.field.trim().is_empty() && claim.access.len() < MAX_CLAIM_ACCESS,
    };
    if *shown == view {
        return;
    }
    let size = if claim.tiles.is_empty() {
        "the whole world".to_string()
    } else {
        format!("{} tiles", claim.tiles.len())
    };
    let can_build = claim.admits(me);
    let public = claim.public;
    let access = claim.access.clone();
    commands
        .entity(*body)
        .despawn_children()
        .with_children(|body| {
            let focus_frame = |parent: &mut ChildSpawnerCommands, button: ClaimButton| {
                if view.focus == Some(button) {
                    parent.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(-SLOT_GAP),
                            right: px(-SLOT_GAP),
                            top: px(-SLOT_GAP),
                            bottom: px(-SLOT_GAP),
                            ..default()
                        },
                        sliced(&assets, SLOT_SELECTED_IMAGE, FOCUS_INSET),
                        Pickable::IGNORE,
                        Hovered::default(),
                    ));
                }
            };
            let line = || Node {
                min_height: px(ROW_HEIGHT),
                column_gap: px(HUD_MARGIN),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            };
            body.spawn((line(), Hovered::default()))
                .with_children(|title| {
                    title.spawn(ink("Claim"));
                    title.spawn((
                        ImageNode::new(assets.load(CLOSE_IMAGE)),
                        icon_node(ROW_HEIGHT),
                        ClaimButton::Close,
                        Hovered::default(),
                    ));
                });
            let pair = |body: &mut ChildSpawnerCommands, key: &str, value: String| {
                body.spawn((line(), Hovered::default()))
                    .with_children(|row| {
                        row.spawn(ink(key.to_string()));
                        row.spawn(ink(value));
                    });
            };
            pair(body, "Owner", view.names[0].clone());
            pair(body, "Size", size);
            if !mine {
                let text = if can_build {
                    "You can build here"
                } else {
                    "You can't build here"
                };
                body.spawn(ink(text));
                return;
            }
            body.spawn((line(), Hovered::default()))
                .with_children(|row| {
                    row.spawn(ink("Public"));
                    let file = if public { TOGGLE_ON } else { TOGGLE_OFF };
                    row.spawn((
                        Node {
                            width: px(TOGGLE_SIZE.x),
                            height: px(TOGGLE_SIZE.y),
                            ..default()
                        },
                        ImageNode::new(assets.load(file)),
                        ClaimButton::Public,
                        Hovered::default(),
                    ))
                    .with_children(|toggle| focus_frame(toggle, ClaimButton::Public));
                });
            if view.confirm {
                body.spawn(ink("Everyone can build here. Turn on?"));
                body.spawn((line(), Hovered::default()))
                    .with_children(|row| {
                        for (text, button, file) in [
                            ("Yes", ClaimButton::ConfirmYes, BUTTON_IMAGE),
                            ("No", ClaimButton::ConfirmNo, BUTTON_SECONDARY_IMAGE),
                        ] {
                            row.spawn((
                                button_node(),
                                sliced(&assets, file, BUTTON_INSET),
                                button,
                                Hovered::default(),
                            ))
                            .with_child((label(text), TextColor(Color::WHITE)));
                        }
                    });
            }
            body.spawn((
                Node {
                    height: px(2.0),
                    ..default()
                },
                BackgroundColor(INK_COLOR),
            ));
            body.spawn(ink(format!(
                "Can build ({}/{MAX_CLAIM_ACCESS})",
                access.len()
            )));
            for (&id, name) in access.iter().zip(view.names.iter().skip(1)) {
                body.spawn((line(), Hovered::default()))
                    .with_children(|row| {
                        row.spawn(ink(name.clone()));
                        row.spawn((
                            ImageNode::new(assets.load(CLOSE_IMAGE)),
                            icon_node(REMOVE_SIZE),
                            ClaimButton::Remove(id),
                            Hovered::default(),
                        ))
                        .with_children(|x| focus_frame(x, ClaimButton::Remove(id)));
                    });
            }
            if !view.chips.is_empty() {
                let chips = Node {
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: px(SLOT_GAP),
                    row_gap: px(SLOT_GAP),
                    align_items: AlignItems::Center,
                    ..default()
                };
                body.spawn((chips, Hovered::default()))
                    .with_children(|row| {
                        row.spawn(ink("In this world:"));
                        for (id, name) in &view.chips {
                            row.spawn((
                                button_node(),
                                sliced(&assets, BUTTON_SECONDARY_IMAGE, BUTTON_INSET),
                                ClaimButton::Chip(*id),
                                Hovered::default(),
                            ))
                            .with_children(|chip| {
                                chip.spawn((label(name.clone()), TextColor(Color::WHITE)));
                                focus_frame(chip, ClaimButton::Chip(*id));
                            });
                        }
                    });
            }
            body.spawn((line(), Hovered::default()))
                .with_children(|row| {
                    row.spawn((
                        Node {
                            flex_grow: 1.0,
                            min_height: px(ROW_HEIGHT),
                            padding: UiRect::horizontal(px(HUD_MARGIN)),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        sliced(&assets, INPUT_IMAGE, INPUT_INSET),
                        ClaimButton::Field,
                        Hovered::default(),
                    ))
                    .with_children(|field| {
                        field.spawn((ink(""), FieldText));
                        focus_frame(field, ClaimButton::Field);
                    });
                    let tint = if view.can_add {
                        Color::WHITE
                    } else {
                        DISABLED_TINT
                    };
                    row.spawn((
                        Node {
                            width: px(BUTTON_WIDTH),
                            ..button_node()
                        },
                        sliced(&assets, BUTTON_IMAGE, BUTTON_INSET).with_color(tint),
                        ClaimButton::Add,
                        Hovered::default(),
                    ))
                    .with_children(|add| {
                        add.spawn((label("Add"), TextColor(Color::WHITE)));
                        focus_frame(add, ClaimButton::Add);
                    });
                });
            if let Some(error) = &view.error {
                body.spawn((label(error.clone()), TextColor(ERROR_COLOR)));
            }
        });
    *shown = view;
}

fn button_node() -> Node {
    Node {
        min_height: px(ROW_HEIGHT),
        padding: UiRect::horizontal(px(HUD_MARGIN)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    }
}

/// The name field's text (or its placeholder), and hover tints on the panel's buttons.
fn field_text(
    state: Res<PanelState>,
    mut field: Query<(&mut Text, &mut TextColor), With<FieldText>>,
    mut buttons: Query<(&ClaimButton, &Hovered, &mut ImageNode)>,
) {
    for (mut text, mut colour) in &mut field {
        let (new, c) = if state.field.is_empty() {
            (FIELD_PLACEHOLDER, PLACEHOLDER_COLOR)
        } else {
            (state.field.as_str(), INK_COLOR)
        };
        if text.0 != new {
            text.0 = new.to_string();
        }
        colour.set_if_neq(TextColor(c));
    }
    for (button, hovered, mut image) in &mut buttons {
        let tint = match (button, hovered.get()) {
            (ClaimButton::Remove(_), true) => REMOVE_HOVER_TINT,
            (ClaimButton::Public | ClaimButton::Close | ClaimButton::Chip(_), true) => HOVER_TINT,
            (ClaimButton::Add, _) => continue, // keeps its enabled / disabled tint
            _ => Color::WHITE,
        };
        if image.color != tint {
            image.color = tint;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::{DIRT, MEDIUM_CLAIM, SMALL_CLAIM, WORLD_CLAIM};
    use game_shared::worldgen::generate;

    const ME: PlayerId = PlayerId(1);
    const ANA: PlayerId = PlayerId(2);

    fn key(x: u32, y: u32) -> u32 {
        World::tile_key(x, y).unwrap()
    }

    fn claim(owner: PlayerId, tiles: Vec<u32>) -> Claim {
        Claim {
            owner,
            access: Vec::new(),
            public: false,
            tiles,
        }
    }

    fn with_area(owner: PlayerId, (x, y): (u32, u32), count: u32) -> World {
        let mut world = generate("T");
        let tiles = claim_area(&world, x, y, count);
        world.insert_claim(key(x, y), claim(owner, tiles));
        world
    }

    #[test]
    fn the_owner_comes_from_the_area_claim_else_the_world_claim() {
        let world = with_area(ANA, (10, 10), 10);
        assert_eq!(owner_at(&world, 10, 10), Some(ANA), "the claim block");
        assert_eq!(owner_at(&world, 11, 10), Some(ANA), "a tile of its area");
        assert_eq!(owner_at(&world, 40, 10), None, "free ground");
        let mut world = with_area(ANA, (10, 10), 10);
        world.insert_claim(key(40, 20), claim(ME, Vec::new()));
        assert_eq!(world.world_claim, Some(key(40, 20)));
        assert_eq!(owner_at(&world, 40, 10), Some(ME), "the world claim");
        assert_eq!(owner_at(&world, 11, 10), Some(ANA), "the area claim wins");
    }

    #[test]
    fn the_wrench_points_at_the_claim_of_any_of_its_tiles() {
        let world = with_area(ANA, (10, 10), 10);
        assert_eq!(
            wrench_target(&world, 10, 10),
            Some(key(10, 10)),
            "the block"
        );
        assert_eq!(
            wrench_target(&world, 11, 10),
            Some(key(10, 10)),
            "a tile of its area"
        );
        assert_eq!(wrench_target(&world, 40, 10), None, "free ground");
    }

    #[test]
    fn a_workbench_says_how_to_craft() {
        assert_eq!(workbench_tag(), "Workbench · press C to craft");
    }

    #[test]
    fn unknown_names_show_the_id_until_the_lookup_answers() {
        assert_eq!(display_name(None, PlayerId(42)), "Player #42");
        assert_eq!(display_name(Some("Ana"), PlayerId(42)), "Ana");
    }

    #[test]
    fn the_preview_shows_the_exact_area_and_why_not() {
        let free = generate("T");
        let p = preview(&free, ME, MEDIUM_CLAIM, (20, 20), true).unwrap();
        assert_eq!(p.look, Look::Place);
        assert_eq!(
            p.tiles,
            claim_area(&free, 20, 20, 48),
            "the server's function"
        );
        assert_eq!(preview_color(p.look), CAN_COLOR);
        assert_eq!(
            preview_label(&p, MEDIUM_CLAIM, ""),
            "Medium claim · 48 tiles"
        );

        let far = preview(&free, ME, SMALL_CLAIM, (20, 20), false).unwrap();
        assert_eq!((far.look, far.tiles.len()), (Look::TooFar, 10));
        assert_eq!(preview_color(far.look), FAR_COLOR);
        assert_eq!(preview_label(&far, SMALL_CLAIM, ""), "Too far");

        let taken = with_area(ANA, (10, 10), 10);
        let p = preview(&taken, ME, SMALL_CLAIM, (11, 10), true).unwrap();
        assert_eq!(p.look, Look::Claimed);
        assert_eq!(p.tiles.len(), 10, "that claim's area, in red");
        assert_eq!(preview_color(p.look), NO_COLOR);
        assert_eq!(preview_label(&p, SMALL_CLAIM, ""), "Already claimed");

        let mut world_of_ana = generate("T");
        world_of_ana.insert_claim(key(5, 5), claim(ANA, Vec::new()));
        let p = preview(&world_of_ana, ME, SMALL_CLAIM, (20, 20), true).unwrap();
        assert_eq!(p.look, Look::NoPermission(ANA));
        assert_eq!(
            preview_label(&p, SMALL_CLAIM, "Ana"),
            "This world belongs to Ana"
        );
        let mine = preview(&world_of_ana, ANA, SMALL_CLAIM, (20, 20), true).unwrap();
        assert_eq!(
            mine.look,
            Look::Place,
            "the world's owner may carve out areas"
        );

        assert_eq!(preview(&free, ME, DIRT, (20, 20), true), None);
    }

    #[test]
    fn a_world_claim_previews_as_a_frame() {
        let free = generate("T");
        let p = preview(&free, ME, WORLD_CLAIM, (20, 20), true).unwrap();
        assert_eq!(
            p,
            Preview {
                look: Look::WorldPlace,
                tiles: Vec::new()
            }
        );
        assert_eq!(
            preview_label(&p, WORLD_CLAIM, ""),
            "World claim · the whole world"
        );
        let foreign = with_area(ANA, (10, 10), 10);
        let p = preview(&foreign, ME, WORLD_CLAIM, (20, 20), true).unwrap();
        assert_eq!(p.look, Look::WorldForeign);
        assert_eq!(
            preview_label(&p, WORLD_CLAIM, ""),
            "Someone else has a claim here"
        );
        let own = with_area(ME, (10, 10), 10);
        assert_eq!(
            preview(&own, ME, WORLD_CLAIM, (20, 20), true).unwrap().look,
            Look::WorldPlace,
            "own area claims do not block it"
        );
        let mut claimed = generate("T");
        claimed.insert_claim(key(5, 5), claim(ANA, Vec::new()));
        let p = preview(&claimed, ME, WORLD_CLAIM, (20, 20), true).unwrap();
        assert_eq!(preview_label(&p, WORLD_CLAIM, ""), "Already claimed");
        assert!(p.look.is_world() && preview_color(p.look) == NO_COLOR);
    }

    #[test]
    fn refusals_name_the_owner() {
        let world = with_area(ANA, (10, 10), 10);
        let name = |id: PlayerId| {
            if id == ANA {
                "Ana".into()
            } else {
                display_name(None, id)
            }
        };
        let toast = |reject, tile, placed| refusal_toast(&world, reject, tile, placed, name);
        assert_eq!(
            toast(Reject::NoPermission, (11, 10), None).as_deref(),
            Some("Locked by Ana")
        );
        assert_eq!(
            toast(Reject::NotOwner, (10, 10), None).as_deref(),
            Some("Only Ana can do that")
        );
        assert_eq!(
            toast(Reject::Occupied, (11, 10), Some(SMALL_CLAIM)).as_deref(),
            Some("That spot is already claimed")
        );
        assert_eq!(
            toast(Reject::Occupied, (30, 10), Some(DIRT)),
            None,
            "a full tile"
        );
        assert_eq!(toast(Reject::OutOfRange, (30, 10), None), None);
        let mut world = generate("T");
        world.insert_claim(key(5, 5), claim(ANA, Vec::new()));
        assert_eq!(
            refusal_toast(&world, Reject::NoPermission, (20, 20), None, name).as_deref(),
            Some("This world is locked by Ana")
        );
    }

    #[test]
    fn adding_says_why_not_with_the_spec_texts() {
        let mut c = claim(ME, vec![1]);
        assert_eq!(
            add_error(&c, ME, "me").as_deref(),
            Some("You are the owner")
        );
        assert_eq!(add_error(&c, ANA, "Ana"), None);
        c.access.push(ANA);
        assert_eq!(
            add_error(&c, ANA, "Ana").as_deref(),
            Some("Ana can already build here")
        );
        c.access = (100..100 + MAX_CLAIM_ACCESS as u32).map(PlayerId).collect();
        assert_eq!(
            add_error(&c, ANA, "Ana").as_deref(),
            Some("The list is full (32)")
        );
    }

    #[test]
    fn tab_goes_toggle_rows_chips_field_add() {
        let mut c = claim(ME, vec![1]);
        c.access.push(ANA);
        let order = focus_order(&c, &[PlayerId(7)]);
        assert_eq!(
            order,
            vec![
                ClaimButton::Public,
                ClaimButton::Remove(ANA),
                ClaimButton::Chip(PlayerId(7)),
                ClaimButton::Field,
                ClaimButton::Add
            ]
        );
    }
}
