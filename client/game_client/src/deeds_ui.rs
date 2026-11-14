//! World Deeds on the client (docs/knowledge/ux/world-key.md): the bag's deed list (shown for the
//! `Deeds` chip, `inventory_filter.rs`; `hud::refresh_slots` puts it where the grid rows go), its
//! Use / Tear up / Discard buttons with their confirms, the deed tooltip and the void refusal
//! toast. The claim panel's Make World Deed button is `claim_ui.rs`'s, the trade rows
//! `trade_ui.rs`'s. The ring itself is `Player::deeds`, mirrored by `DeedsChanged`.

use bevy::picking::Pickable;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use game_shared::deeds::{Deed, WorldName};
use game_shared::rules::{Action, Reject};

use crate::audio::Sfx;
use crate::hud::{
    DISABLED_TEXT_COLOR, HUD_MARGIN, Notice, SLOT_GAP, SLOT_SELECTED_IMAGE, TextTip, icon_node,
    ink, label, sliced, tab_node,
};
use crate::net::Net;
use crate::state::{ActionMsg, ActionResult, Game, Phase};

/// Art (assets/README.md): 32×32 item art, and the 16×16 one for the ring and the trade row.
#[allow(dead_code)] // the 32 px art: no 32 px place shows a deed yet
pub(crate) const DEED_IMAGE: &str = "ui/deed.png";
pub(crate) const DEED_SMALL_IMAGE: &str = "ui/deed_small.png";
/// The 16 px art at the kit's 2× scale.
pub(crate) const DEED_ICON_SIZE: f32 = 32.0;
/// A void deed: the same art, greyed in code.
pub(crate) const VOID_ICON_TINT: Color = Color::srgba(0.5, 0.5, 0.5, 0.6);
/// Touch: every row at least this tall.
const ROW_MIN_HEIGHT: f32 = 48.0;
const FOCUS_INSET: f32 = 8.0;

pub(crate) const DEED_TITLE_PREFIX: &str = "World Deed · ";
const DEED_USE_HINT: &str = "Use it to become the owner of ";

/// What a deed row's button does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DeedAct {
    Use,
    TearUp,
    Discard,
}

/// Who owns a deed's world, as far as this client knows (only the world it is in).
#[derive(Clone, PartialEq, Eq, Debug)]
enum Owner {
    Me,
    Named(String),
    Unknown,
}

impl DeedAct {
    fn label(self) -> &'static str {
        match self {
            DeedAct::Use => "Use",
            DeedAct::TearUp => "Tear up",
            DeedAct::Discard => "Discard",
        }
    }

    /// The confirm's yes button.
    fn yes_label(self) -> &'static str {
        match self {
            DeedAct::Use => "Use deed",
            DeedAct::TearUp => "Tear up",
            DeedAct::Discard => "Discard",
        }
    }

    /// The confirm line and its yes button (the spec's texts); `None` acts at once.
    fn confirm(self, world: &str, owner: &Owner) -> Option<(String, &'static str)> {
        match self {
            DeedAct::Use => Some((
                format!("Become the owner of {world}? The deed is used up."),
                self.yes_label(),
            )),
            DeedAct::TearUp => Some((tear_up_text(world, owner), self.yes_label())),
            DeedAct::Discard => None,
        }
    }

    fn action(self, world: WorldName) -> Action {
        match self {
            DeedAct::Use => Action::UseDeed { world },
            DeedAct::TearUp => Action::TearUpDeed { world },
            DeedAct::Discard => Action::DiscardDeed { world },
        }
    }
}

/// The owner keeps the spec's first text; a holder who isn't the owner is told who stays it.
/// ponytail: the owner is only known inside that world; elsewhere the owner sentence is left out.
fn tear_up_text(world: &str, owner: &Owner) -> String {
    match owner {
        Owner::Me => format!("Tear up the deed for {world}? You stay the owner."),
        Owner::Named(name) => {
            format!(
                "Tear up the deed for {world}? It can't be used afterwards. {name} stays the owner."
            )
        }
        Owner::Unknown => format!("Tear up the deed for {world}? It can't be used afterwards."),
    }
}

/// The owner of `world` if the player is in it and it has a World Claim.
fn known_owner(game: &Game, net: Option<&Net>, world: WorldName) -> Owner {
    let owner = (game.0.world.name == world.as_str())
        .then_some(())
        .and_then(|()| game.0.world.world_claim)
        .and_then(|key| game.0.world.claim(key))
        .map(|claim| claim.owner);
    match owner {
        None => Owner::Unknown,
        Some(id) if id == game.1 => Owner::Me,
        Some(id) => Owner::Named(crate::claim_ui::name_of(net, game.1, id)),
    }
}

/// A deed's buttons: `Use` · `Tear up`, or only `Discard` for a void one.
pub(crate) fn deed_actions(deed: Deed) -> &'static [DeedAct] {
    if deed.void {
        &[DeedAct::Discard]
    } else {
        &[DeedAct::Use, DeedAct::TearUp]
    }
}

/// The deed tooltip (U3 box): `World Deed · MYFARM` / `Use it to become the owner of MYFARM`, or
/// the void text.
pub(crate) fn deed_tip(deed: Deed) -> String {
    let world = deed.world.as_str();
    let second = if deed.void {
        game_protocol::deed_void_text(world)
    } else {
        format!("{DEED_USE_HINT}{world}")
    };
    format!("{DEED_TITLE_PREFIX}{world}\n{second}")
}

/// The deed list's state: the button armed by a first click (with its confirm line), and the
/// keyboard / gamepad focus (an index into `list_buttons`). Rows are rebuilt when it changes
/// (`hud::refresh_slots`).
#[derive(Resource, Default, PartialEq)]
pub(crate) struct DeedView {
    armed: Option<(WorldName, DeedAct, String)>,
    focus: Option<usize>,
}

/// Keys `hud::take_bag_keys` passes on while the Deeds chip is on show: Tab / arrows / d-pad
/// step, Enter / A activates.
#[derive(Resource, Default)]
pub(crate) struct DeedKeys {
    pub step: i32,
    pub activate: bool,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum DeedButton {
    Act(WorldName, DeedAct),
    Yes,
    Cancel,
}

pub struct DeedsUiPlugin;

impl Plugin for DeedsUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DeedView>()
            .init_resource::<DeedKeys>()
            .add_systems(Update, use_deeds.in_set(Phase::Input))
            .add_systems(Update, toast_void.in_set(Phase::View));
    }
}

/// One deed row's buttons: its actions, or the armed confirm's yes / Cancel.
fn row_buttons(deed: Deed, armed: Option<&(WorldName, DeedAct, String)>) -> Vec<DeedButton> {
    if !deed.void && armed.is_some_and(|(world, ..)| *world == deed.world) {
        return vec![DeedButton::Yes, DeedButton::Cancel];
    }
    deed_actions(deed)
        .iter()
        .map(|&act| DeedButton::Act(deed.world, act))
        .collect()
}

/// Every button of the list, top to bottom: the focus order.
fn list_buttons(deeds: &[Deed], view: &DeedView) -> Vec<DeedButton> {
    deeds
        .iter()
        .flat_map(|&deed| row_buttons(deed, view.armed.as_ref()))
        .collect()
}

/// The deed list, one row per deed: icon, world name, then its buttons (or the armed confirm).
pub(crate) fn spawn_deed_rows(
    grid: &mut ChildSpawnerCommands,
    assets: &AssetServer,
    deeds: &[Deed],
    view: &DeedView,
) {
    let mut index = 0;
    for &deed in deeds {
        let row = Node {
            width: percent(100),
            min_height: px(ROW_MIN_HEIGHT),
            column_gap: px(HUD_MARGIN),
            row_gap: px(SLOT_GAP),
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            ..default()
        };
        grid.spawn((row, Hovered::default())).with_children(|row| {
            let name = Node {
                column_gap: px(HUD_MARGIN),
                align_items: AlignItems::Center,
                flex_grow: 1.0,
                ..default()
            };
            row.spawn((name, TextTip(deed_tip(deed)), Hovered::default()))
                .with_children(|name| {
                    let tint = if deed.void {
                        VOID_ICON_TINT
                    } else {
                        Color::WHITE
                    };
                    name.spawn((
                        ImageNode::new(assets.load(DEED_SMALL_IMAGE)).with_color(tint),
                        icon_node(DEED_ICON_SIZE),
                        Hovered::default(),
                    ));
                    if deed.void {
                        name.spawn((
                            label(deed.world.as_str()),
                            TextColor(DISABLED_TEXT_COLOR),
                            Hovered::default(),
                        ));
                    } else {
                        name.spawn((ink(deed.world.as_str()), Hovered::default()));
                    }
                });
            let buttons = row_buttons(deed, view.armed.as_ref());
            let confirm = view
                .armed
                .as_ref()
                .filter(|_| buttons[0] == DeedButton::Yes);
            if let Some((_, _, text)) = confirm {
                row.spawn((ink(text.clone()), Hovered::default()));
            }
            for button in buttons {
                let text = match button {
                    DeedButton::Act(_, act) => act.label(),
                    DeedButton::Yes => confirm.map_or("", |(_, act, _)| act.yes_label()),
                    DeedButton::Cancel => "Cancel",
                };
                let focused = view.focus == Some(index);
                index += 1;
                row.spawn((tab_node(assets, true), button, Hovered::default()))
                    .with_children(|b| {
                        b.spawn(label(text));
                        if focused {
                            b.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: px(-SLOT_GAP),
                                    right: px(-SLOT_GAP),
                                    top: px(-SLOT_GAP),
                                    bottom: px(-SLOT_GAP),
                                    ..default()
                                },
                                sliced(assets, SLOT_SELECTED_IMAGE, FOCUS_INSET),
                                Pickable::IGNORE,
                                Hovered::default(),
                            ));
                        }
                    });
            }
        });
    }
}

/// A deed button, by click or by focus + Enter / A.
fn press(
    button: DeedButton,
    game: &Game,
    net: Option<&Net>,
    view: &mut DeedView,
    actions: &mut MessageWriter<ActionMsg>,
) {
    match button {
        DeedButton::Act(world, act) => {
            let owner = known_owner(game, net, world);
            match act.confirm(world.as_str(), &owner) {
                Some((text, _)) => view.armed = Some((world, act, text)),
                None => {
                    actions.write(ActionMsg(act.action(world)));
                }
            }
        }
        DeedButton::Yes => {
            if let Some((world, act, _)) = view.armed.take() {
                actions.write(ActionMsg(act.action(world)));
            }
        }
        DeedButton::Cancel => view.armed = None,
    }
}

/// Clicks and keys on the deed list. An armed deed that left the ring (used, torn up, traded,
/// voided) disarms; the focus stays inside the list.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn use_deeds(
    mouse: Res<ButtonInput<MouseButton>>,
    game: Res<Game>,
    net: Option<Res<Net>>,
    buttons: Query<(&DeedButton, &Hovered)>,
    mut keys: ResMut<DeedKeys>,
    mut view: ResMut<DeedView>,
    mut actions: MessageWriter<ActionMsg>,
    mut sfx: MessageWriter<Sfx>,
) {
    let DeedKeys { step, activate } = std::mem::take(&mut *keys);
    let deeds = game.me().map_or(&[][..], |me| &me.deeds[..]);
    if let Some((world, ..)) = view.armed
        && !deeds.contains(&Deed { world, void: false })
    {
        view.armed = None;
    }
    let order = list_buttons(deeds, &view);
    let focus = match (view.focus, step) {
        (_, _) if order.is_empty() => None,
        (None, 0) => None,
        (None, s) if s > 0 => Some(0),
        (None, _) => Some(order.len() - 1),
        (Some(at), s) => Some((at as i64 + i64::from(s)).rem_euclid(order.len() as i64) as usize),
    };
    if view.focus != focus {
        view.focus = focus;
        sfx.write(Sfx::UiClick);
    }
    let net = net.as_deref();
    if activate && let Some(&button) = focus.and_then(|at| order.get(at)) {
        sfx.write(Sfx::UiClick);
        press(button, &game, net, &mut view, &mut actions);
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(&button) = buttons.iter().find(|(_, h)| h.get()).map(|(b, _)| b) else {
        return;
    };
    sfx.write(Sfx::UiClick);
    press(button, &game, net, &mut view, &mut actions);
}

/// `Reject::DeedVoid` needs the world's name, which the action carries: `hud` stays silent on it.
fn toast_void(mut results: MessageReader<ActionResult>, mut notices: MessageWriter<Notice>) {
    for result in results.read() {
        if let (Action::UseDeed { world }, Err(Reject::DeedVoid)) = (result.action, &result.result)
        {
            notices.write(Notice(game_protocol::deed_void_text(world.as_str())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deed(void: bool) -> Deed {
        Deed {
            world: WorldName::new("MYFARM").unwrap(),
            void,
        }
    }

    #[test]
    fn a_void_deed_can_only_be_discarded() {
        assert_eq!(deed_actions(deed(false)), [DeedAct::Use, DeedAct::TearUp]);
        assert_eq!(deed_actions(deed(true)), [DeedAct::Discard]);
        let labels: Vec<_> = deed_actions(deed(false))
            .iter()
            .map(|a| a.label())
            .collect();
        assert_eq!(labels, ["Use", "Tear up"]);
        assert_eq!(DeedAct::Discard.label(), "Discard");
    }

    #[test]
    fn use_and_tear_up_ask_first_with_the_spec_texts() {
        assert_eq!(
            DeedAct::Use.confirm("MYFARM", &Owner::Me),
            Some((
                "Become the owner of MYFARM? The deed is used up.".to_string(),
                "Use deed"
            ))
        );
        assert_eq!(
            DeedAct::TearUp.confirm("MYFARM", &Owner::Me),
            Some((
                "Tear up the deed for MYFARM? You stay the owner.".to_string(),
                "Tear up"
            ))
        );
        assert_eq!(DeedAct::Discard.confirm("MYFARM", &Owner::Me), None);
        let world = WorldName::new("MYFARM").unwrap();
        assert_eq!(DeedAct::Use.action(world), Action::UseDeed { world });
        assert_eq!(DeedAct::TearUp.action(world), Action::TearUpDeed { world });
        assert_eq!(
            DeedAct::Discard.action(world),
            Action::DiscardDeed { world }
        );
    }

    #[test]
    fn a_holder_who_is_not_the_owner_is_told_who_stays_it() {
        assert_eq!(
            tear_up_text("MYFARM", &Owner::Named("Ana".to_string())),
            "Tear up the deed for MYFARM? It can't be used afterwards. Ana stays the owner."
        );
        assert_eq!(
            tear_up_text("MYFARM", &Owner::Unknown),
            "Tear up the deed for MYFARM? It can't be used afterwards."
        );
    }

    #[test]
    fn the_focus_order_follows_the_rows_and_the_armed_confirm() {
        let (live, void) = (deed(false), deed(true));
        let world = live.world;
        let mut view = DeedView::default();
        assert_eq!(
            list_buttons(&[live, void], &view),
            [
                DeedButton::Act(world, DeedAct::Use),
                DeedButton::Act(world, DeedAct::TearUp),
                DeedButton::Act(world, DeedAct::Discard),
            ]
        );
        view.armed = Some((world, DeedAct::Use, String::new()));
        assert_eq!(
            list_buttons(&[live, void], &view),
            [
                DeedButton::Yes,
                DeedButton::Cancel,
                DeedButton::Act(world, DeedAct::Discard),
            ],
            "the void deed of the same world keeps its Discard"
        );
    }

    #[test]
    fn the_tooltip_names_the_world_or_says_it_is_void() {
        assert_eq!(
            deed_tip(deed(false)),
            "World Deed · MYFARM\nUse it to become the owner of MYFARM"
        );
        assert_eq!(
            deed_tip(deed(true)),
            "World Deed · MYFARM\nVoid: MYFARM's World Claim is gone"
        );
    }
}
