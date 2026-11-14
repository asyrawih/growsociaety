//! Chat commands (docs/knowledge/13): the parser, the table of who may use what, and the resolved
//! admin actions that `rules::apply_admin` performs. Pure: no accounts, no network, no clock.
//! The server (or offline mode) checks `required` against the caller before acting.

use std::fmt;

use crate::items::{ITEMS, ItemId};
use crate::rules::PlayerId;
use crate::{WORLD_HEIGHT, WORLD_WIDTH};

/// A whole command line, in bytes. Reference (doc 13).
pub const MAX_LINE: usize = 256;
/// A reason, in bytes.
pub const MAX_REASON: usize = 200;
/// Counts are `1..=MAX_COUNT` (the stack cap).
pub const MAX_COUNT: u16 = crate::inventory::MAX_STACK;
/// World bans: default and the longest a world owner may give.
pub const WORLD_BAN_DEFAULT_MINUTES: u32 = 60;
pub const WORLD_BAN_MAX_MINUTES: u32 = 7 * 24 * 60;

/// Staff roles, ordered: `Player < Moderator < Admin < Owner`. Owner is the super admin: it can do
/// everything an Admin can, and nobody (Admins included) can act on it. The server grants it only
/// from its own configuration, never from a command or the database.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Role {
    Player,
    Moderator,
    Admin,
    Owner,
}

/// What the caller needs before a command may run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Requirement {
    Anyone,
    /// The world claim's owner or someone on its access list.
    WorldAccess,
    /// The world claim's owner.
    WorldOwner,
    /// At least this staff role.
    Role(Role),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Duration {
    Minutes(u32),
    Permanent,
}

/// `<player>`: an exact name (case-insensitive) or `#<id>`. The caller resolves it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Target {
    Name(String),
    Id(PlayerId),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Command {
    Help {
        topic: Option<String>,
    },
    Who,
    Msg {
        to: Target,
        text: String,
    },
    Reply {
        text: String,
    },
    Ignore {
        who: Target,
        on: bool,
    },
    /// Ask a player in this world to trade (docs/knowledge/08).
    Trade {
        who: Target,
    },
    /// Put a player on (or take them off) the access list of your nearest claim (docs/knowledge/07).
    Access {
        who: Target,
        on: bool,
    },
    Kick {
        who: Target,
    },
    Pull {
        who: Target,
    },
    WorldBan {
        who: Target,
        length: Duration,
    },
    /// `None`: everyone.
    WorldUnban {
        who: Option<Target>,
    },
    Give {
        who: Target,
        item: ItemId,
        count: u16,
        reason: String,
    },
    Take {
        who: Target,
        item: ItemId,
        count: u16,
        reason: String,
    },
    Spawn {
        item: ItemId,
        count: u16,
    },
    Gems {
        who: Target,
        delta: i64,
        reason: String,
    },
    SetBlock {
        x: u32,
        y: u32,
        item: ItemId,
    },
    ClearBlock {
        x: u32,
        y: u32,
    },
    /// Owner only: the biggest backpack and one full stack of everything that can be held.
    GiveAll {
        who: Target,
    },
    /// Owner only: everything but the fist and wrench leaves the inventory; nothing stays worn.
    ClearInv {
        who: Target,
        reason: String,
    },
}

/// A world-changing admin command with its players resolved: what `rules::apply_admin` takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Admin {
    Give {
        to: PlayerId,
        item: ItemId,
        count: u16,
    },
    Take {
        from: PlayerId,
        item: ItemId,
        count: u16,
    },
    /// At the caller's feet.
    Spawn {
        item: ItemId,
        count: u16,
    },
    Gems {
        who: PlayerId,
        delta: i64,
    },
    SetBlock {
        x: u32,
        y: u32,
        item: ItemId,
    },
    ClearBlock {
        x: u32,
        y: u32,
    },
    GiveAll {
        to: PlayerId,
    },
    ClearInv {
        from: PlayerId,
    },
}

impl Command {
    /// The resolved admin action, if this is one. `resolve` turns a `Target` into a player.
    pub fn admin(&self, resolve: impl Fn(&Target) -> Option<PlayerId>) -> Option<Admin> {
        Some(match self {
            Command::Give {
                who, item, count, ..
            } => Admin::Give {
                to: resolve(who)?,
                item: *item,
                count: *count,
            },
            Command::Take {
                who, item, count, ..
            } => Admin::Take {
                from: resolve(who)?,
                item: *item,
                count: *count,
            },
            &Command::Spawn { item, count } => Admin::Spawn { item, count },
            Command::Gems { who, delta, .. } => Admin::Gems {
                who: resolve(who)?,
                delta: *delta,
            },
            &Command::SetBlock { x, y, item } => Admin::SetBlock { x, y, item },
            &Command::ClearBlock { x, y } => Admin::ClearBlock { x, y },
            Command::GiveAll { who } => Admin::GiveAll { to: resolve(who)? },
            Command::ClearInv { who, .. } => Admin::ClearInv {
                from: resolve(who)?,
            },
            _ => return None,
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Command::Help { .. } => "help",
            Command::Who => "who",
            Command::Msg { .. } => "msg",
            Command::Reply { .. } => "r",
            Command::Ignore { on: true, .. } => "ignore",
            Command::Ignore { on: false, .. } => "unignore",
            Command::Trade { .. } => "trade",
            Command::Access { .. } => "access",
            Command::Kick { .. } => "kick",
            Command::Pull { .. } => "pull",
            Command::WorldBan { .. } => "wban",
            Command::WorldUnban { .. } => "wunban",
            Command::Give { .. } => "give",
            Command::Take { .. } => "take",
            Command::Spawn { .. } => "spawn",
            Command::Gems { .. } => "gems",
            Command::SetBlock { .. } => "setblock",
            Command::ClearBlock { .. } => "clearblock",
            Command::GiveAll { .. } => "giveall",
            Command::ClearInv { .. } => "clearinv",
        }
    }
}

/// One row of the command table: `/help`, the client's launcher and `required` all read it.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
    pub needs: Requirement,
}

const fn spec(
    name: &'static str,
    usage: &'static str,
    summary: &'static str,
    needs: Requirement,
) -> Spec {
    Spec {
        name,
        usage,
        summary,
        needs,
    }
}

use Requirement::{Anyone, WorldAccess, WorldOwner};
const ADMIN: Requirement = Requirement::Role(Role::Admin);
const OWNER: Requirement = Requirement::Role(Role::Owner);

/// Every command that runs today. Reference (doc 13, "Command list").
pub const COMMANDS: &[Spec] = &[
    spec(
        "help",
        "/help [command]",
        "Commands you may use, or one command's syntax",
        Anyone,
    ),
    spec("who", "/who", "Players in this world", Anyone),
    spec(
        "msg",
        "/msg <player> <text>",
        "Private message to a player in this world",
        Anyone,
    ),
    spec(
        "r",
        "/r <text>",
        "Reply to your last private message",
        Anyone,
    ),
    spec(
        "ignore",
        "/ignore <player>",
        "Hide a player's chat for you",
        Anyone,
    ),
    spec(
        "unignore",
        "/unignore <player>",
        "Show a player's chat again",
        Anyone,
    ),
    spec(
        "trade",
        "/trade <player>",
        "Ask a player in this world to trade",
        Anyone,
    ),
    spec(
        "access",
        "/access <player> on|off",
        "Let a player edit your nearest claim, or stop them",
        Anyone,
    ),
    spec(
        "kick",
        "/kick <player>",
        "Send a player to the main door",
        WorldAccess,
    ),
    spec(
        "pull",
        "/pull <player>",
        "Move a player in this world to you",
        WorldAccess,
    ),
    spec(
        "wban",
        "/wban <player> [30m|2h|7d]",
        "Ban from this world (default 1h, max 7d)",
        WorldOwner,
    ),
    spec(
        "wunban",
        "/wunban <player|all>",
        "Lift world bans",
        WorldOwner,
    ),
    spec(
        "give",
        "/give <player> <item> [count] <reason>",
        "Put items in a player's inventory",
        ADMIN,
    ),
    spec(
        "take",
        "/take <player> <item> [count] <reason>",
        "Remove items from a player",
        ADMIN,
    ),
    spec(
        "spawn",
        "/spawn <item> [count]",
        "Drop items at your feet",
        ADMIN,
    ),
    spec(
        "gems",
        "/gems <player> <+n|-n> <reason>",
        "Adjust a player's gems",
        ADMIN,
    ),
    spec(
        "setblock",
        "/setblock <x> <y> <item>",
        "Set one tile, ignoring claims",
        ADMIN,
    ),
    spec(
        "clearblock",
        "/clearblock <x> <y>",
        "Empty one tile, ignoring claims",
        ADMIN,
    ),
    spec(
        "giveall",
        "/giveall <player>",
        "Max backpack and a full stack of every item",
        OWNER,
    ),
    spec(
        "clearinv",
        "/clearinv <player> <reason>",
        "Remove every item but the fist and wrench",
        OWNER,
    ),
];

/// Commands of doc 13 that need moderation, accounts or persistence (phases C3+).
const PLANNED: &[&str] = &[
    "report",
    "staff",
    "vmute",
    "mute",
    "unmute",
    "warn",
    "kickserver",
    "ban",
    "unban",
    "goto",
    "tp",
    "bring",
    "spectate",
    "inspect",
    "history",
    "reports",
    "closeworld",
    "openworld",
    "role",
    "announce",
    "shutdown",
];

pub fn spec_of(name: &str) -> Option<&'static Spec> {
    COMMANDS.iter().find(|spec| spec.name == name)
}

/// What the caller needs for `command`. One table, so the launcher, `/help` and the check agree.
pub fn required(command: &Command) -> Requirement {
    spec_of(command.name()).map_or(ADMIN, |spec| spec.needs)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ParseError {
    /// The line does not start with `/`.
    NotACommand,
    TooLong,
    Unknown(String),
    /// A doc 13 command that is not built yet.
    NotYet(&'static str),
    /// Wrong arguments; holds the usage line.
    Usage(&'static str),
    UnknownItem,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::NotACommand => write!(f, "Commands start with /"),
            ParseError::TooLong => write!(f, "That line is too long"),
            ParseError::Unknown(name) => write!(f, "Unknown command /{name}. Try /help"),
            ParseError::NotYet(name) => write!(f, "/{name} is not available yet"),
            ParseError::Usage(usage) => write!(f, "Usage: {usage}"),
            ParseError::UnknownItem => write!(f, "No item by that name"),
        }
    }
}

/// `30m`, `2h`, `7d` or `perm`.
pub fn parse_duration(text: &str) -> Option<Duration> {
    if text.eq_ignore_ascii_case("perm") {
        return Some(Duration::Permanent);
    }
    let split = text.len().checked_sub(1)?;
    let (number, unit) = text.split_at_checked(split)?;
    let n: u32 = number.parse().ok().filter(|&n| n > 0)?;
    let per = match unit {
        "m" => 1,
        "h" => 60,
        "d" => 24 * 60,
        _ => return None,
    };
    n.checked_mul(per).map(Duration::Minutes)
}

fn target(word: &str) -> Target {
    match word.strip_prefix('#').and_then(|id| id.parse().ok()) {
        Some(id) => Target::Id(PlayerId(id)),
        None => Target::Name(word.to_owned()),
    }
}

/// The longest item name (case-insensitive, words joined by spaces) at the start of `words`, or a
/// numeric id. Returns the item and how many words it used.
pub fn find_item(words: &[&str]) -> Option<(ItemId, usize)> {
    let first = words.first()?;
    if let Ok(id) = first.parse::<u16>() {
        return crate::items::item(ItemId(id)).map(|def| (def.id, 1));
    }
    (1..=words.len()).rev().find_map(|n| {
        let name = words[..n].join(" ");
        ITEMS
            .iter()
            .find(|def| def.name.eq_ignore_ascii_case(&name))
            .map(|def| (def.id, n))
    })
}

fn count(word: Option<&&str>) -> Option<Option<u16>> {
    match word {
        None => Some(None),
        Some(word) => match word.parse::<u16>() {
            Ok(n) if (1..=MAX_COUNT).contains(&n) => Some(Some(n)),
            Ok(_) => None,
            Err(_) => Some(None),
        },
    }
}

fn coordinate(word: &str, below: u32) -> Option<u32> {
    word.parse().ok().filter(|&v| v < below)
}

fn reason(words: &[&str]) -> Option<String> {
    let text = words.join(" ");
    (!text.is_empty() && text.len() <= MAX_REASON).then_some(text)
}

/// Parses one chat line that starts with `/`. Never panics; bad input is a `ParseError`.
pub fn parse(line: &str) -> Result<Command, ParseError> {
    if line.len() > MAX_LINE {
        return Err(ParseError::TooLong);
    }
    let body = line
        .trim()
        .strip_prefix('/')
        .ok_or(ParseError::NotACommand)?;
    let words: Vec<&str> = body.split_whitespace().collect();
    let Some((&name, args)) = words.split_first() else {
        return Err(ParseError::Unknown(String::new()));
    };
    let name = name.to_ascii_lowercase();
    let Some(spec) = spec_of(&name) else {
        return Err(match PLANNED.iter().find(|planned| **planned == name) {
            Some(planned) => ParseError::NotYet(planned),
            None => ParseError::Unknown(name),
        });
    };
    let usage = ParseError::Usage(spec.usage);
    let rest = |from: usize| args.get(from..).map(|words| words.join(" "));
    let command = match (spec.name, args) {
        ("help", []) => Command::Help { topic: None },
        ("help", [topic]) => Command::Help {
            topic: Some(topic.trim_start_matches('/').to_ascii_lowercase()),
        },
        ("who", []) => Command::Who,
        ("msg", [to, _, ..]) => Command::Msg {
            to: target(to),
            text: rest(1).ok_or(usage)?,
        },
        ("r", [_, ..]) => Command::Reply {
            text: rest(0).ok_or(usage)?,
        },
        ("ignore" | "unignore", [who]) => Command::Ignore {
            who: target(who),
            on: spec.name == "ignore",
        },
        ("trade", [who]) => Command::Trade { who: target(who) },
        ("access", [who, switch]) => Command::Access {
            who: target(who),
            on: match switch.to_ascii_lowercase().as_str() {
                "on" => true,
                "off" => false,
                _ => return Err(usage),
            },
        },
        ("kick", [who]) => Command::Kick { who: target(who) },
        ("pull", [who]) => Command::Pull { who: target(who) },
        ("wban", [who, length @ ..]) => {
            let length = match length {
                [] => Duration::Minutes(WORLD_BAN_DEFAULT_MINUTES),
                [text] => match parse_duration(text) {
                    Some(Duration::Minutes(m)) if m <= WORLD_BAN_MAX_MINUTES => {
                        Duration::Minutes(m)
                    }
                    _ => return Err(usage),
                },
                _ => return Err(usage),
            };
            Command::WorldBan {
                who: target(who),
                length,
            }
        }
        ("wunban", [who]) => Command::WorldUnban {
            who: (!who.eq_ignore_ascii_case("all")).then(|| target(who)),
        },
        ("give" | "take", [who, rest @ ..]) => {
            let (item, used) = find_item(rest).ok_or(ParseError::UnknownItem)?;
            let after = &rest[used..];
            let given = count(after.first()).ok_or(usage.clone())?;
            let skip = usize::from(given.is_some());
            let reason = reason(&after[skip..]).ok_or(usage)?;
            let (who, count) = (target(who), given.unwrap_or(1));
            if spec.name == "give" {
                Command::Give {
                    who,
                    item,
                    count,
                    reason,
                }
            } else {
                Command::Take {
                    who,
                    item,
                    count,
                    reason,
                }
            }
        }
        ("spawn", [_, ..]) => {
            let (item, used) = find_item(args).ok_or(ParseError::UnknownItem)?;
            let count = match &args[used..] {
                [] => 1,
                [n] => count(Some(n)).flatten().ok_or(usage)?,
                _ => return Err(usage),
            };
            Command::Spawn { item, count }
        }
        ("gems", [who, delta, reason_words @ ..]) => {
            let signed = delta.starts_with('+') || delta.starts_with('-');
            let delta: i64 = delta.parse().ok().filter(|_| signed).ok_or(usage.clone())?;
            Command::Gems {
                who: target(who),
                delta,
                reason: reason(reason_words).ok_or(usage)?,
            }
        }
        ("setblock", [x, y, _, ..]) => {
            let (x, y) = coordinate(x, WORLD_WIDTH)
                .zip(coordinate(y, WORLD_HEIGHT))
                .ok_or(usage)?;
            let (item, used) = find_item(&args[2..]).ok_or(ParseError::UnknownItem)?;
            if used != args.len() - 2 {
                return Err(ParseError::UnknownItem);
            }
            Command::SetBlock { x, y, item }
        }
        ("clearblock", [x, y]) => {
            let (x, y) = coordinate(x, WORLD_WIDTH)
                .zip(coordinate(y, WORLD_HEIGHT))
                .ok_or(usage)?;
            Command::ClearBlock { x, y }
        }
        ("giveall", [who]) => Command::GiveAll { who: target(who) },
        ("clearinv", [who, words @ ..]) => Command::ClearInv {
            who: target(who),
            reason: reason(words).ok_or(usage)?,
        },
        _ => return Err(usage),
    };
    Ok(command)
}

/// `/help` text: the commands `allowed` lets the caller use, or one command's usage.
pub fn help(topic: Option<&str>, allowed: impl Fn(&Spec) -> bool) -> String {
    match topic {
        Some(topic) => match spec_of(topic) {
            Some(spec) => format!("{} - {}", spec.usage, spec.summary),
            None => format!("No command /{topic}"),
        },
        None => {
            let names: Vec<String> = COMMANDS
                .iter()
                .filter(|spec| allowed(spec))
                .map(|spec| format!("/{}", spec.name))
                .collect();
            format!("Commands: {}. /help <command> for details", names.join(" "))
        }
    }
}

/// How many of the item the admin command's subject held before it ran (0 if not Give/Take).
/// Read before `apply_admin`, then pass to `report`.
pub fn held_before(state: &crate::rules::WorldState, admin: &Admin) -> u16 {
    let (who, item) = match *admin {
        Admin::Give { to, item, .. } => (to, item),
        Admin::Take { from, item, .. } => (from, item),
        _ => return 0,
    };
    state
        .players
        .get(&who)
        .map_or(0, |player| player.inventory.count(item))
}

/// One line telling the caller what an admin command did: how many items actually fit, and so on.
pub fn report(
    admin: &Admin,
    held_before: u16,
    result: &Result<Vec<crate::rules::Event>, crate::rules::Reject>,
) -> String {
    use crate::rules::Event;
    let name = |id: ItemId| crate::items::item(id).map_or("?", |def| def.name);
    let events = match result {
        Ok(events) => events,
        Err(reject) => return format!("Refused: {reject:?}"),
    };
    let total = events.iter().find_map(|event| match event {
        Event::InventoryChanged { count, .. } => Some(*count),
        _ => None,
    });
    match *admin {
        Admin::Give { item, count, .. } => {
            let added = total.unwrap_or(held_before).saturating_sub(held_before);
            let lost = count - added.min(count);
            let note = if lost > 0 {
                format!(" ({lost} did not fit)")
            } else {
                String::new()
            };
            format!("Gave {added} {}{note}", name(item))
        }
        Admin::Take { item, .. } => {
            let taken = held_before.saturating_sub(total.unwrap_or(held_before));
            format!("Took {taken} {}", name(item))
        }
        Admin::Spawn { item, count } => format!("Spawned {count} {}", name(item)),
        Admin::Gems { .. } => match events.first() {
            Some(Event::GemsChanged { gems, .. }) => format!("Gems are now {gems}"),
            _ => "Gems unchanged".into(),
        },
        Admin::SetBlock { x, y, item } => format!("Set ({x}, {y}) to {}", name(item)),
        Admin::ClearBlock { x, y } if events.is_empty() => format!("({x}, {y}) was already empty"),
        Admin::ClearBlock { x, y } => format!("Cleared ({x}, {y})"),
        Admin::ClearInv { .. } => {
            let kinds = events
                .iter()
                .filter(|e| matches!(e, Event::InventoryChanged { .. }))
                .count();
            format!("Cleared {kinds} items")
        }
        Admin::GiveAll { .. } => {
            let kinds = events
                .iter()
                .filter(|e| matches!(e, Event::InventoryChanged { .. }))
                .count();
            format!("Gave full stacks of {kinds} items")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{DIRT, SMALL_CLAIM, seed_of};

    fn name(n: &str) -> Target {
        Target::Name(n.into())
    }

    #[test]
    fn every_command_parses_from_its_documented_syntax() {
        let cases = [
            ("/help", Command::Help { topic: None }),
            (
                "/help /give",
                Command::Help {
                    topic: Some("give".into()),
                },
            ),
            ("/who", Command::Who),
            (
                "/msg Budi hi there",
                Command::Msg {
                    to: name("Budi"),
                    text: "hi there".into(),
                },
            ),
            ("/r ok", Command::Reply { text: "ok".into() }),
            (
                "/ignore Budi",
                Command::Ignore {
                    who: name("Budi"),
                    on: true,
                },
            ),
            (
                "/unignore #7",
                Command::Ignore {
                    who: Target::Id(PlayerId(7)),
                    on: false,
                },
            ),
            ("/trade Budi", Command::Trade { who: name("Budi") }),
            (
                "/access Budi on",
                Command::Access {
                    who: name("Budi"),
                    on: true,
                },
            ),
            (
                "/access #7 OFF",
                Command::Access {
                    who: Target::Id(PlayerId(7)),
                    on: false,
                },
            ),
            ("/kick Budi", Command::Kick { who: name("Budi") }),
            ("/pull Budi", Command::Pull { who: name("Budi") }),
            (
                "/wban Budi",
                Command::WorldBan {
                    who: name("Budi"),
                    length: Duration::Minutes(60),
                },
            ),
            (
                "/wban Budi 2h",
                Command::WorldBan {
                    who: name("Budi"),
                    length: Duration::Minutes(120),
                },
            ),
            ("/wunban all", Command::WorldUnban { who: None }),
            (
                "/wunban Budi",
                Command::WorldUnban {
                    who: Some(name("Budi")),
                },
            ),
            (
                "/give Budi small claim 3 event prize",
                Command::Give {
                    who: name("Budi"),
                    item: SMALL_CLAIM,
                    count: 3,
                    reason: "event prize".into(),
                },
            ),
            (
                "/take Budi DIRT dupe",
                Command::Take {
                    who: name("Budi"),
                    item: DIRT,
                    count: 1,
                    reason: "dupe".into(),
                },
            ),
            (
                "/spawn dirt seed 20",
                Command::Spawn {
                    item: seed_of(DIRT).unwrap(),
                    count: 20,
                },
            ),
            (
                "/spawn 1",
                Command::Spawn {
                    item: DIRT,
                    count: 1,
                },
            ),
            (
                "/gems Budi +500 contest",
                Command::Gems {
                    who: name("Budi"),
                    delta: 500,
                    reason: "contest".into(),
                },
            ),
            (
                "/gems Budi -20 refund",
                Command::Gems {
                    who: name("Budi"),
                    delta: -20,
                    reason: "refund".into(),
                },
            ),
            (
                "/setblock 3 4 dirt",
                Command::SetBlock {
                    x: 3,
                    y: 4,
                    item: DIRT,
                },
            ),
            ("/clearblock 3 4", Command::ClearBlock { x: 3, y: 4 }),
        ];
        for (line, expected) in cases {
            assert_eq!(parse(line), Ok(expected), "{line}");
        }
        // Every row of the table is reachable through `parse`.
        for spec in COMMANDS {
            assert!(
                !matches!(
                    parse(&format!("/{}", spec.name)),
                    Err(ParseError::Unknown(_))
                ),
                "{}",
                spec.name
            );
        }
    }

    #[test]
    fn bad_syntax_is_an_error_with_usage_never_a_panic() {
        let usage = |name| Err(ParseError::Usage(spec_of(name).unwrap().usage));
        assert_eq!(parse("/kick"), usage("kick"));
        assert_eq!(parse("/give Budi dirt 3"), usage("give")); // reason missing
        assert_eq!(parse("/give Budi dirt 0 x"), usage("give")); // count outside 1..=200
        assert_eq!(parse("/give Budi dirt 201 x"), usage("give"));
        assert_eq!(parse("/spawn dirt 201"), usage("spawn"));
        assert_eq!(parse("/gems Budi 50 x"), usage("gems")); // needs a sign
        assert_eq!(parse("/setblock 100 0 dirt"), usage("setblock")); // outside the world
        assert_eq!(parse("/clearblock 0 50"), usage("clearblock"));
        assert_eq!(parse("/wban Budi 8d"), usage("wban")); // over the owner maximum
        assert_eq!(parse("/wban Budi perm"), usage("wban"));
        assert_eq!(parse("/spawn unobtainium"), Err(ParseError::UnknownItem));
        assert_eq!(
            parse("/setblock 1 1 dirt extra"),
            Err(ParseError::UnknownItem)
        );
        assert_eq!(parse("/dance"), Err(ParseError::Unknown("dance".into())));
        assert_eq!(parse("/mute Budi 1h spam"), Err(ParseError::NotYet("mute")));
        assert_eq!(parse("hello"), Err(ParseError::NotACommand));
        assert_eq!(
            parse(&format!("/msg Budi {}", "x".repeat(MAX_LINE))),
            Err(ParseError::TooLong)
        );
        assert!(parse(&format!("/give Budi dirt 1 {}", "x".repeat(MAX_REASON + 1))).is_err());
        for garbage in [
            "/",
            "/ ",
            "//",
            "/give",
            "/give #",
            "/setblock -1 2 dirt",
            "/gems Budi +99999999999999999999 x",
        ] {
            assert!(parse(garbage).is_err(), "{garbage}");
        }
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30m"), Some(Duration::Minutes(30)));
        assert_eq!(parse_duration("2h"), Some(Duration::Minutes(120)));
        assert_eq!(parse_duration("7d"), Some(Duration::Minutes(7 * 24 * 60)));
        assert_eq!(parse_duration("perm"), Some(Duration::Permanent));
        for bad in ["", "m", "0m", "5", "5w", "-1h", "1.5h", "99999999999d"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
    }

    #[test]
    fn requirement_table() {
        let cases = [
            ("/help", Requirement::Anyone),
            ("/who", Requirement::Anyone),
            ("/msg a b", Requirement::Anyone),
            ("/r b", Requirement::Anyone),
            ("/ignore a", Requirement::Anyone),
            ("/unignore a", Requirement::Anyone),
            ("/trade a", Requirement::Anyone),
            ("/access a on", Requirement::Anyone),
            ("/kick a", Requirement::WorldAccess),
            ("/pull a", Requirement::WorldAccess),
            ("/wban a", Requirement::WorldOwner),
            ("/wunban all", Requirement::WorldOwner),
            ("/give a dirt why", ADMIN),
            ("/take a dirt why", ADMIN),
            ("/spawn dirt", ADMIN),
            ("/gems a +1 why", ADMIN),
            ("/setblock 1 1 dirt", ADMIN),
            ("/clearblock 1 1", ADMIN),
            ("/giveall a", OWNER),
            ("/clearinv a why", OWNER),
        ];
        assert_eq!(cases.len(), COMMANDS.len());
        for (line, needs) in cases {
            assert_eq!(required(&parse(line).expect(line)), needs, "{line}");
        }
        assert!(Role::Player < Role::Moderator && Role::Moderator < Role::Admin);
    }

    #[test]
    fn report_says_how_many_fit() {
        use crate::rules::{Event, Reject};
        let give = Admin::Give {
            to: PlayerId(1),
            item: DIRT,
            count: 120,
        };
        let full = Ok(vec![Event::InventoryChanged {
            player: PlayerId(1),
            item: DIRT,
            count: 200,
        }]);
        assert_eq!(report(&give, 150, &full), "Gave 50 dirt (70 did not fit)");
        assert_eq!(
            report(&give, 0, &Err(Reject::InventoryFull)),
            "Refused: InventoryFull"
        );
        let take = Admin::Take {
            from: PlayerId(1),
            item: DIRT,
            count: 9,
        };
        let none = Ok(vec![Event::InventoryChanged {
            player: PlayerId(1),
            item: DIRT,
            count: 0,
        }]);
        assert_eq!(report(&take, 5, &none), "Took 5 dirt");
    }

    #[test]
    fn items_resolve_case_insensitively() {
        assert_eq!(find_item(&["Small", "CLAIM", "5"]), Some((SMALL_CLAIM, 2)));
        assert_eq!(
            find_item(&["dirt", "seed"]),
            Some((seed_of(DIRT).unwrap(), 2))
        );
        assert_eq!(find_item(&["nothing"]), None);
        assert_eq!(find_item(&["65535"]), None);
    }
}
