//! The trade window (docs/knowledge/08, "Trade flow"). It decides nothing: every click sends a
//! `Trade*` action and the window is rebuilt from `Game`'s copy of the trade when a trade or
//! inventory event arrives. The rules (counts, locks, the review delay) live in `game_shared::trade`.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use game_shared::items::{ItemId, PERMANENT, UNTRADEABLE, item};
use game_shared::rules::{Action, Event, PlayerId};
use game_shared::trade::{TRADE_REVIEW_DELAY, Trade, TradeStage};

use crate::audio::Sfx;
use crate::hud::{
    FitWindow, HUD_MARGIN, KIT_SCALE, SCROLLBAR_SPACE, SLOT_GAP, ink, label, panel_max_height,
    scroll_grid, sliced, tab_node,
};
use crate::net::Net;
use crate::screenshot;
use crate::state::{ActionMsg, Game, GameEvent, Phase};
use crate::world_view::item_icon;

const PANEL_IMAGE: &str = "ui/panel.png";
const PANEL_INSET: f32 = 18.0;
const SLOT_IMAGE: &str = "ui/slot.png";
const SLOT_INSET: f32 = 8.0;
const SLOT_SIZE: f32 = 37.0 * KIT_SCALE;
const ICON_SIZE: f32 = 48.0;
const PANEL_TOP: f32 = 72.0;
const PANEL_PADDING: f32 = 32.0;
const PICKER_COLUMNS: f32 = 6.0;
const PANEL_WIDTH: f32 = PICKER_COLUMNS * SLOT_SIZE
    + (PICKER_COLUMNS - 1.0) * SLOT_GAP
    + SCROLLBAR_SPACE
    + 2.0 * PANEL_PADDING;

/// Ask `to` for a trade. The coding session's `/trade <name>` command (or a click on a player)
/// writes this; nothing in the client targets players yet.
#[derive(Message, Clone, Copy)]
#[allow(dead_code)] // constructed by the pending entry point above
pub struct RequestTrade(pub PlayerId);

#[derive(Component)]
struct TradeRoot;

/// The scrolling grid of the player's tradeable items.
#[derive(Component)]
struct TradePicker;

#[derive(Component, Clone, Copy)]
enum TradeButton {
    Accept,
    Decline,
    /// Put one of this inventory item in the next free slot of the own offer.
    Pick(ItemId),
    /// Clear this slot of the own offer.
    Clear(u8),
    More(u8),
    Less(u8),
    Lock(bool),
    Confirm,
    Cancel,
}

pub struct TradeUiPlugin;

impl Plugin for TradeUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RequestTrade>()
            .add_systems(Startup, screenshot_trade)
            .add_systems(Update, click_trade.in_set(Phase::Input))
            .add_systems(Update, refresh_trade.in_set(Phase::View));
    }
}

/// The open trade or request of `me`, and which side of it `me` is.
fn my_trade(game: &Game) -> Option<(&Trade, usize)> {
    let me = game.1;
    game.0
        .trades
        .iter()
        .find_map(|trade| Some((trade, trade.side(me)?)))
}

fn click_trade(
    mouse: Res<ButtonInput<MouseButton>>,
    game: Res<Game>,
    buttons: Query<(&TradeButton, &Hovered)>,
    mut requests: MessageReader<RequestTrade>,
    mut actions: MessageWriter<ActionMsg>,
    mut sfx: MessageWriter<Sfx>,
) {
    for &RequestTrade(to) in requests.read() {
        actions.write(ActionMsg(Action::TradeRequest { to }));
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(&button) = buttons.iter().find(|(_, h)| h.get()).map(|(b, _)| b) else {
        return;
    };
    sfx.write(Sfx::UiClick);
    let offer = my_trade(&game).map(|(trade, side)| trade.offers[side]);
    let set = |slot: u8, item, count| Action::TradeSetOffer { slot, item, count };
    let in_slot = |slot: u8| offer.and_then(|offer| offer.get(usize::from(slot)).copied()?);
    let action = match button {
        TradeButton::Accept => Some(Action::TradeRespond { accept: true }),
        TradeButton::Decline => Some(Action::TradeRespond { accept: false }),
        TradeButton::Pick(id) => offer
            .and_then(|offer| offer.iter().position(Option::is_none))
            .and_then(|slot| u8::try_from(slot).ok())
            .map(|slot| set(slot, id, 1)),
        TradeButton::Clear(slot) => in_slot(slot).map(|(id, _)| set(slot, id, 0)),
        TradeButton::More(slot) => in_slot(slot).map(|(id, n)| set(slot, id, n.saturating_add(1))),
        TradeButton::Less(slot) => in_slot(slot).map(|(id, n)| set(slot, id, n.saturating_sub(1))),
        TradeButton::Lock(locked) => Some(Action::TradeLock { locked }),
        TradeButton::Confirm => Some(Action::TradeConfirm),
        TradeButton::Cancel => Some(Action::TradeCancel),
    };
    if let Some(action) = action {
        actions.write(ActionMsg(action));
    }
}

/// Rebuilds the window when a trade or inventory event arrives, or when the review delay ends.
#[allow(clippy::too_many_arguments)] // a Bevy system: state, events, clock and the old window
fn refresh_trade(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    net: Option<Res<Net>>,
    time: Res<Time>,
    window: Single<&Window>,
    mut events: MessageReader<GameEvent>,
    roots: Query<Entity, With<TradeRoot>>,
    picker: Query<&ScrollPosition, With<TradePicker>>,
    mut review: Local<Option<(u32, f32)>>,
    mut drawn: Local<Option<bool>>,
) {
    let me = game.1;
    let changed = events.read().any(|GameEvent(event)| match event {
        Event::TradeRequested { .. }
        | Event::TradeUpdated { .. }
        | Event::TradeCompleted { .. }
        | Event::TradeCancelled { .. } => true,
        Event::InventoryChanged { player, .. } => *player == me,
        _ => false,
    });
    let trade = my_trade(&game);
    // The review clock is the local one: the trade's `since` is the server's time.
    let now = time.elapsed_secs();
    *review = match trade {
        Some((trade, _)) if matches!(trade.stage, TradeStage::Review { .. }) => Some(
            review
                .filter(|(id, _)| *id == trade.id)
                .unwrap_or((trade.id, now)),
        ),
        _ => None,
    };
    let ready = review.is_some_and(|(_, since)| now >= since + TRADE_REVIEW_DELAY as f32);
    if !changed && *drawn == Some(ready) {
        return;
    }
    *drawn = Some(ready);
    // The rebuilt picker keeps the place the player scrolled to.
    let scroll = picker
        .iter()
        .next()
        .map_or(Vec2::ZERO, |position| position.0);
    for root in &roots {
        commands.entity(root).despawn();
    }
    let Some((trade, side)) = trade else {
        return;
    };
    let other = trade.other(me).unwrap_or(me);
    let name = net
        .as_deref()
        .and_then(|net| net.name(other))
        .map_or_else(|| format!("Player {}", other.0), str::to_owned);
    let root = Node {
        position_type: PositionType::Absolute,
        left: percent(50),
        top: px(PANEL_TOP),
        margin: UiRect::left(px(-PANEL_WIDTH / 2.0)),
        width: px(PANEL_WIDTH),
        // Set here and kept up to date on resize by `hud::fit_panels`.
        max_height: px(panel_max_height(window.size().y, PANEL_TOP)),
        padding: UiRect::all(px(PANEL_PADDING)),
        flex_direction: FlexDirection::Column,
        row_gap: px(HUD_MARGIN),
        ..default()
    };
    let panel = (
        TradeRoot,
        FitWindow { top: PANEL_TOP },
        root,
        sliced(&assets, PANEL_IMAGE, PANEL_INSET),
        Hovered::default(),
    );
    commands.spawn(panel).with_children(|panel| {
        if trade.stage == TradeStage::Requested {
            if side == 1 {
                panel.spawn(ink(format!("{name} wants to trade")));
                buttons(
                    panel,
                    &assets,
                    &[
                        ("Accept", Some(TradeButton::Accept)),
                        ("Decline", Some(TradeButton::Decline)),
                    ],
                );
            } else {
                panel.spawn(ink(format!("Waiting for {name}...")));
                buttons(panel, &assets, &[("Cancel", Some(TradeButton::Cancel))]);
            }
            return;
        }
        let editing = trade.stage == TradeStage::Editing;
        let mine = &trade.offers[side];
        let theirs = &trade.offers[1 - side];
        let state = |locked: bool, confirmed: bool| match (confirmed, locked) {
            (true, _) => "confirmed",
            (_, true) => "locked",
            _ => "editing",
        };
        panel.spawn(ink(format!("Trade with {name}")));
        panel.spawn(ink(format!(
            "You give ({})",
            state(trade.locked[side], trade.confirmed[side])
        )));
        panel.spawn(row()).with_children(|slots| {
            for (slot, stack) in (0u8..).zip(mine) {
                let column = Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: px(SLOT_GAP),
                    ..default()
                };
                slots.spawn(column).with_children(|column| {
                    let clear = stack.filter(|_| editing).map(|_| TradeButton::Clear(slot));
                    slot_cell(column, &assets, *stack, clear);
                    if editing && stack.is_some() {
                        column.spawn(row()).with_children(|pm| {
                            small_button(pm, &assets, "-", TradeButton::Less(slot));
                            small_button(pm, &assets, "+", TradeButton::More(slot));
                        });
                    }
                });
            }
        });
        panel.spawn(ink(format!(
            "You get ({})",
            state(trade.locked[1 - side], trade.confirmed[1 - side])
        )));
        panel.spawn(row()).with_children(|slots| {
            for stack in theirs {
                slot_cell(slots, &assets, *stack, None);
            }
        });
        if editing {
            panel.spawn(ink("Your items"));
            let offered = |id: ItemId| mine.iter().flatten().any(|&(o, _)| o == id);
            let pickable = game.me().map_or(Vec::new(), |me| {
                let tradeable = |id: ItemId| {
                    item(id).is_some_and(|def| !def.has(UNTRADEABLE | PERMANENT)) && !offered(id)
                };
                me.inventory
                    .slots()
                    .iter()
                    .copied()
                    .filter(|&(id, _)| tradeable(id))
                    .collect()
            });
            let grid = scroll_grid(panel, (TradePicker, ScrollPosition(scroll)));
            panel.commands().entity(grid).with_children(|grid| {
                for (id, count) in pickable {
                    slot_cell(
                        grid,
                        &assets,
                        Some((id, count)),
                        Some(TradeButton::Pick(id)),
                    );
                }
            });
        } else {
            panel.spawn(ink("Check both offers, then confirm"));
        }
        let locked = trade.locked[side];
        let lock = editing.then_some(TradeButton::Lock(!locked));
        let lock_text = if locked { "Unlock" } else { "Lock" };
        let confirm = (ready && !trade.confirmed[side]).then_some(TradeButton::Confirm);
        buttons(
            panel,
            &assets,
            &[
                (lock_text, lock),
                ("Confirm", confirm),
                ("Cancel", Some(TradeButton::Cancel)),
            ],
        );
    });
}

fn row() -> Node {
    Node {
        column_gap: px(SLOT_GAP),
        ..default()
    }
}

/// A row of wooden buttons; one without an action is drawn dimmed and does nothing.
fn buttons(
    parent: &mut ChildSpawnerCommands,
    assets: &AssetServer,
    list: &[(&str, Option<TradeButton>)],
) {
    parent.spawn(row()).with_children(|row| {
        for &(text, button) in list {
            let mut entity = row.spawn((tab_node(assets, button.is_some()), Hovered::default()));
            if let Some(button) = button {
                entity.insert(button);
            }
            entity.with_children(|b| {
                b.spawn(label(text));
            });
        }
    });
}

fn small_button(
    parent: &mut ChildSpawnerCommands,
    assets: &AssetServer,
    text: &str,
    button: TradeButton,
) {
    let (mut node, image) = tab_node(assets, true);
    node.min_width = px(SLOT_SIZE / 2.0 - SLOT_GAP / 2.0);
    node.padding = UiRect::ZERO;
    parent
        .spawn((node, image, button, Hovered::default()))
        .with_children(|b| {
            b.spawn(label(text));
        });
}

/// A kit slot with the stack's icon and count; clickable when it has a button.
fn slot_cell(
    parent: &mut ChildSpawnerCommands,
    assets: &AssetServer,
    stack: Option<(ItemId, u16)>,
    button: Option<TradeButton>,
) {
    let node = Node {
        width: px(SLOT_SIZE),
        height: px(SLOT_SIZE),
        flex_shrink: 0.0,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    };
    let image = sliced(assets, SLOT_IMAGE, SLOT_INSET);
    let mut cell = parent.spawn((node, image, Hovered::default()));
    if let Some(button) = button {
        cell.insert(button);
    }
    let Some((def, count)) = stack.and_then(|(id, n)| Some((item(id)?, n))) else {
        return;
    };
    cell.with_children(|cell| {
        let icon = Node {
            width: px(ICON_SIZE),
            height: px(ICON_SIZE),
            ..default()
        };
        cell.spawn((item_icon(assets, def.sprite).node(), icon));
        if count > 1 {
            let corner = Node {
                position_type: PositionType::Absolute,
                right: px(6.0),
                bottom: px(3.0),
                ..default()
            };
            cell.spawn((label(count.to_string()), corner));
        }
    });
}

/// `GAME_SCREENSHOT_UI=trade` (or `trade-prompt`) opens a trade with a stand-in partner offline,
/// so the window can be captured. Goes through `apply` like any trade.
fn screenshot_trade(mut game: ResMut<Game>, net: Option<Res<Net>>) {
    use game_shared::items::{DIRT, SAND, STONE};
    use game_shared::rules::apply;
    let Some(request) = screenshot::ui_request().filter(|r| r.starts_with("trade")) else {
        return;
    };
    if net.is_some() {
        return;
    }
    let (me, partner) = (game.1, PlayerId(game.1.0 + 1));
    let state = &mut game.0;
    state.join(partner);
    for (who, id, n) in [(me, DIRT, 40), (me, SAND, 12), (partner, STONE, 20)] {
        if let Some(player) = state.players.get_mut(&who) {
            player.inventory.add(id, n);
        }
    }
    let mut script = vec![(partner, Action::TradeRequest { to: me })];
    if request != "trade-prompt" {
        script.extend([
            (me, Action::TradeRespond { accept: true }),
            (
                me,
                Action::TradeSetOffer {
                    slot: 0,
                    item: DIRT,
                    count: 5,
                },
            ),
            (
                partner,
                Action::TradeSetOffer {
                    slot: 0,
                    item: STONE,
                    count: 20,
                },
            ),
            (partner, Action::TradeLock { locked: true }),
        ]);
    }
    for (who, action) in script {
        if let Err(reject) = apply(state, who, action, 0.0) {
            warn!("screenshot trade: {action:?} rejected: {reject:?}");
        }
    }
}
