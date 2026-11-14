//! Voice chat: LiveKit tokens and voice moderation (docs/knowledge/12). Voice is a view of the world
//! and never touches `WorldState`. The API secret stays in this process: only signed tokens leave.
//!
//! Off unless `LIVEKIT_URL`, `LIVEKIT_API_KEY` and `LIVEKIT_API_SECRET` are all set (logged once).
//! ponytail: the client is handed the same `LIVEKIT_URL` the server API uses; a separate public URL
//! comes when LiveKit runs behind a proxy.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use game_protocol::{ChatKind, ServerMsg};
use game_shared::commands::Requirement;
use game_shared::rules::PlayerId;
use livekit_api::access_token::{AccessToken, VideoGrants};
use livekit_api::services::room::{RemoveParticipantOptions, RoomClient, UpdateParticipantOptions};

use crate::world::{Out, Room};

const URL_ENV: &str = "LIVEKIT_URL";
const KEY_ENV: &str = "LIVEKIT_API_KEY";
const SECRET_ENV: &str = "LIVEKIT_API_SECRET";
/// How long a voice token is valid (doc 12: 10 minutes).
pub const TOKEN_TTL: Duration = Duration::from_secs(600);
/// How often everyone in a world gets a fresh token; half the TTL, so none ever expires in hand.
pub const TOKEN_REFRESH: Duration = Duration::from_secs(300);
/// The only track source a player may publish (LiveKit's grant name).
const MIC_GRANT: &str = "microphone";
/// `livekit.TrackSource.MICROPHONE` in LiveKit's protocol, for the server API's permission.
const MIC_SOURCE: i32 = 2;

/// LiveKit's address and keys, read from the environment once per process.
pub struct VoiceConfig {
    url: String,
    key: String,
    secret: String,
    rooms: RoomClient,
}

impl VoiceConfig {
    /// `None` (voice off) unless all three variables are set and non-empty.
    fn from_env() -> Option<Self> {
        let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
        let (Some(url), Some(key), Some(secret)) = (var(URL_ENV), var(KEY_ENV), var(SECRET_ENV))
        else {
            println!("voice chat off: set {URL_ENV}, {KEY_ENV} and {SECRET_ENV} to turn it on");
            return None;
        };
        println!("voice chat on: {url}");
        let rooms = RoomClient::with_api_key(&url, &key, &secret);
        Some(Self {
            url,
            key,
            secret,
            rooms,
        })
    }
}

fn config() -> Option<&'static VoiceConfig> {
    static CONFIG: OnceLock<Option<VoiceConfig>> = OnceLock::new();
    CONFIG.get_or_init(VoiceConfig::from_env).as_ref()
}

/// The LiveKit room of a world.
pub fn room_name(world: &str) -> String {
    format!("world-{world}")
}

/// A signed token: join `room` as `id`, subscribe, publish the microphone only (unless muted), no data.
fn mint(
    key: &str,
    secret: &str,
    room: &str,
    id: PlayerId,
    can_publish: bool,
) -> Result<String, String> {
    let grants = VideoGrants {
        room_join: true,
        room: room.to_owned(),
        can_subscribe: Some(true),
        can_publish: Some(can_publish),
        can_publish_sources: if can_publish {
            vec![MIC_GRANT.to_owned()]
        } else {
            vec![]
        },
        can_publish_data: Some(false),
        ..VideoGrants::default()
    };
    AccessToken::with_api_key(key, secret)
        .with_identity(&id.0.to_string())
        .with_ttl(TOKEN_TTL)
        .with_grants(grants)
        .to_jwt()
        .map_err(|e| e.to_string())
}

/// Runs a LiveKit server API call beside the world task, so the world never waits on HTTP. Without a
/// Tokio runtime (unit tests) it is skipped.
fn spawn(call: impl Future<Output = ()> + Send + 'static) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(call);
    }
}

/// One world's voice room. Lives in `Room::voice`, inside the world task.
pub struct Voice {
    config: &'static VoiceConfig,
    room: String,
    /// Players the world owner has muted. Their tokens carry no publish grant.
    muted: HashSet<PlayerId>,
}

impl Voice {
    /// `None` when voice is off for this server.
    pub fn for_world(world: &str) -> Option<Self> {
        Some(Self {
            config: config()?,
            room: room_name(world),
            muted: HashSet::new(),
        })
    }

    fn token(&self, id: PlayerId) -> Option<ServerMsg> {
        let config = self.config;
        let can_publish = !self.muted.contains(&id);
        match mint(&config.key, &config.secret, &self.room, id, can_publish) {
            Ok(token) => Some(ServerMsg::VoiceToken {
                url: config.url.clone(),
                token,
                expires_in: TOKEN_TTL.as_secs() as u32,
            }),
            Err(e) => {
                eprintln!("voice token for player {} failed: {e}", id.0);
                None
            }
        }
    }
}

impl Room {
    /// Right after `Joined`: the player's token, and who is muted already.
    pub(crate) fn voice_joined(&mut self, id: PlayerId) {
        let Some(voice) = &self.voice else {
            return;
        };
        let muted: Vec<PlayerId> = voice.muted.iter().copied().collect();
        if let Some(token) = voice.token(id) {
            self.send(id, Out::Reliable(token));
        }
        for player in muted {
            let msg = ServerMsg::VoiceMuted {
                player,
                muted: true,
            };
            self.send(id, Out::Reliable(msg));
        }
    }

    /// A fresh token for everyone in the world (`TOKEN_REFRESH`, from `run_world`).
    pub(crate) fn refresh_voice_tokens(&mut self) {
        let ids: Vec<PlayerId> = self.names.keys().copied().collect();
        for id in ids {
            if let Some(token) = self.voice.as_ref().and_then(|voice| voice.token(id)) {
                self.send(id, Out::Reliable(token));
            }
        }
    }

    /// Called from `Room::remove`, the one exit (leave, dead connection, kick, world ban, door):
    /// out of the voice room, and every token issued so far is revoked.
    pub(crate) fn voice_left(&mut self, id: PlayerId) {
        let Some(voice) = &mut self.voice else {
            return;
        };
        voice.muted.remove(&id);
        let (rooms, room) = (&voice.config.rooms, voice.room.clone());
        let revoke_token_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |t| t.as_millis() as i64);
        let options = RemoveParticipantOptions { revoke_token_ts };
        spawn(async move {
            // Most players never turned voice on: "not found" is the usual answer and is fine.
            let _ = rooms
                .remove_participant_with_options(&room, &id.0.to_string(), options)
                .await;
        });
    }

    /// The world claim owner or staff mute or unmute `player` for everyone, with the same checks
    /// as `/wban`: the access list alone is not enough, and nobody acts on an equal or higher role.
    pub(crate) fn voice_mute(&mut self, by: PlayerId, player: PlayerId, muted: bool) {
        let allowed = self.allowed(by, Requirement::WorldOwner) && self.may_act_on(by, player);
        if !allowed || !self.names.contains_key(&player) || self.voice.is_none() {
            let text = "You cannot mute that player's voice".to_owned();
            let kind = ChatKind::System;
            return self.send(
                by,
                Out::Reliable(ServerMsg::Chat {
                    from: by,
                    text,
                    kind,
                }),
            );
        }
        let Some(voice) = &mut self.voice else {
            return;
        };
        let changed = if muted {
            voice.muted.insert(player)
        } else {
            voice.muted.remove(&player)
        };
        if !changed {
            return;
        }
        let (rooms, room) = (&voice.config.rooms, voice.room.clone());
        let mut options = UpdateParticipantOptions {
            permission: Some(Default::default()),
            ..Default::default()
        };
        if let Some(permission) = &mut options.permission {
            permission.can_subscribe = true;
            permission.can_publish = !muted;
            permission.can_publish_sources = if muted { vec![] } else { vec![MIC_SOURCE] };
            permission.can_publish_data = false;
        }
        spawn(async move {
            // Revoking publish rights unpublishes the track; a player not in voice yet is covered
            // by their next token, which carries no publish grant.
            if let Err(e) = rooms
                .update_participant(&room, &player.0.to_string(), options)
                .await
            {
                eprintln!("voice mute of player {} in {room}: {e}", player.0);
            }
        });
        let ids: Vec<PlayerId> = self.names.keys().copied().collect();
        for id in ids {
            self.send(id, Out::Reliable(ServerMsg::VoiceMuted { player, muted }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{OUTBOX, ToWorld};
    use game_shared::claims::Claim;
    use game_shared::commands::Role;
    use game_shared::rules::WorldState;
    use game_shared::worldgen::generate;
    use livekit_api::access_token::{Claims, TokenVerifier};
    use tokio::sync::mpsc;

    const KEY: &str = "testkey";
    const SECRET: &str = "testsecret-at-least-32-characters-long";

    fn test_config() -> &'static VoiceConfig {
        static CONFIG: OnceLock<VoiceConfig> = OnceLock::new();
        CONFIG.get_or_init(|| VoiceConfig {
            url: "ws://voice.test:7880".into(),
            key: KEY.into(),
            secret: SECRET.into(),
            rooms: RoomClient::with_api_key("ws://voice.test:7880", KEY, SECRET),
        })
    }

    fn room_with_voice(world: &str) -> Room {
        let mut room = Room::new(WorldState::new(generate(world), 1));
        room.voice = Some(Voice {
            config: test_config(),
            room: room_name(world),
            muted: HashSet::new(),
        });
        room
    }

    fn join(room: &mut Room, id: u32) -> mpsc::Receiver<Out> {
        join_as(room, id, Role::Player)
    }

    fn join_as(room: &mut Room, id: u32, role: Role) -> mpsc::Receiver<Out> {
        let (out, rx) = mpsc::channel(OUTBOX);
        let join = ToWorld::Join {
            id: PlayerId(id),
            name: format!("p{id}"),
            country: None,
            role,
            save: None,
            out,
        };
        room.handle(join, 0.0);
        rx
    }

    fn drain(rx: &mut mpsc::Receiver<Out>) -> Vec<ServerMsg> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .map(|(Out::Reliable(msg) | Out::Datagram(msg))| msg)
            .collect()
    }

    fn tokens(msgs: &[ServerMsg]) -> Vec<String> {
        msgs.iter()
            .filter_map(|m| match m {
                ServerMsg::VoiceToken { token, .. } => Some(token.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_token_after_joined_has_the_right_room_identity_ttl_and_grants() {
        let mut room = room_with_voice("TALK");
        let mut rx = join(&mut room, 7);
        let msgs = drain(&mut rx);
        assert!(matches!(msgs.first(), Some(ServerMsg::Joined { .. })));
        let Some(ServerMsg::VoiceToken {
            url,
            token,
            expires_in,
        }) = msgs.get(1)
        else {
            panic!("{msgs:?}");
        };
        assert_eq!(url, "ws://voice.test:7880");
        assert_eq!(*expires_in, 600);
        // Signed with the secret: a verifier holding it accepts the token.
        let claims = TokenVerifier::with_api_key(KEY, SECRET)
            .verify(token)
            .expect("valid signature");
        assert!(
            TokenVerifier::with_api_key(KEY, "another-secret-of-at-least-32-chars")
                .verify(token)
                .is_err()
        );
        assert_eq!(claims.sub, "7");
        assert_eq!(claims.iss, KEY);
        assert_eq!(claims.exp - claims.nbf, 600);
        let grants = claims.video;
        assert!(grants.room_join);
        assert_eq!(grants.room, "world-TALK");
        assert_eq!(grants.can_subscribe, Some(true));
        assert_eq!(grants.can_publish, Some(true));
        assert_eq!(grants.can_publish_sources, vec!["microphone".to_owned()]);
        assert_eq!(grants.can_publish_data, Some(false));
        assert!(!grants.room_admin && !grants.room_create && !grants.room_list);
        // The secret never travels.
        assert!(!token.contains(SECRET));
    }

    #[test]
    fn tokens_go_only_to_players_in_the_world() {
        let mut room = room_with_voice("ONLY");
        let mut a = join(&mut room, 1);
        let mut b = join(&mut room, 2);
        drain(&mut a);
        drain(&mut b);
        room.handle(ToWorld::Leave { id: PlayerId(2) }, 1.0);
        drain(&mut a);

        room.refresh_voice_tokens();
        let to_a = tokens(&drain(&mut a));
        assert_eq!(to_a.len(), 1);
        let claims = Claims::from_unverified(&to_a[0]).expect("decodes");
        assert_eq!(claims.sub, "1");
        assert!(tokens(&drain(&mut b)).is_empty(), "B left: no token");

        // Without voice configured nobody gets one.
        let mut quiet = Room::new(WorldState::new(generate("QUIET"), 1));
        let mut c = join(&mut quiet, 3);
        quiet.refresh_voice_tokens();
        assert!(tokens(&drain(&mut c)).is_empty());
    }

    #[test]
    fn only_the_world_owner_mutes_and_a_muted_player_cannot_publish() {
        let mut room = room_with_voice("MUTE");
        let mut owner = join(&mut room, 1);
        let mut guest = join(&mut room, 2);
        room.state.world.insert_claim(
            0,
            Claim {
                owner: PlayerId(1),
                access: vec![PlayerId(2)],
                public: false,
                tiles: vec![],
            },
        );
        drain(&mut owner);
        drain(&mut guest);

        // On the access list is not enough.
        room.handle(
            ToWorld::VoiceMute {
                id: PlayerId(2),
                player: PlayerId(1),
                muted: true,
            },
            1.0,
        );
        assert!(drain(&mut owner).is_empty());
        let refused = drain(&mut guest);
        assert!(
            matches!(
                refused.as_slice(),
                [ServerMsg::Chat {
                    kind: ChatKind::System,
                    ..
                }]
            ),
            "{refused:?}"
        );

        room.handle(
            ToWorld::VoiceMute {
                id: PlayerId(1),
                player: PlayerId(2),
                muted: true,
            },
            1.0,
        );
        let muted = ServerMsg::VoiceMuted {
            player: PlayerId(2),
            muted: true,
        };
        assert_eq!(drain(&mut owner), vec![muted.clone()]);
        assert_eq!(drain(&mut guest), vec![muted.clone()]);

        // The muted player's fresh token has no publish grant; a newcomer learns who is muted.
        room.refresh_voice_tokens();
        let token = tokens(&drain(&mut guest)).pop().expect("token");
        let grants = Claims::from_unverified(&token).expect("decodes").video;
        assert_eq!(grants.can_publish, Some(false));
        assert!(grants.can_publish_sources.is_empty());
        let mut newcomer = join(&mut room, 3);
        assert!(drain(&mut newcomer).contains(&muted));
    }

    fn mute(room: &mut Room, by: u32, player: u32) {
        let msg = ToWorld::VoiceMute {
            id: PlayerId(by),
            player: PlayerId(player),
            muted: true,
        };
        room.handle(msg, 1.0);
    }

    fn was_muted(rx: &mut mpsc::Receiver<Out>, player: u32) -> bool {
        drain(rx).contains(&ServerMsg::VoiceMuted {
            player: PlayerId(player),
            muted: true,
        })
    }

    #[test]
    fn staff_mute_too_but_nobody_mutes_an_equal_or_higher_role() {
        let mut room = room_with_voice("STAFF");
        let mut owner = join(&mut room, 1);
        let mut helper = join(&mut room, 2);
        let mut player = join(&mut room, 3);
        let mut moderator = join_as(&mut room, 4, Role::Moderator);
        let mut admin = join_as(&mut room, 5, Role::Admin);
        room.state.world.insert_claim(
            0,
            Claim {
                owner: PlayerId(1),
                access: vec![PlayerId(2)],
                public: false,
                tiles: vec![],
            },
        );
        for rx in [
            &mut owner,
            &mut helper,
            &mut player,
            &mut moderator,
            &mut admin,
        ] {
            drain(rx);
        }

        // An access-list player is still refused.
        mute(&mut room, 2, 3);
        assert!(!was_muted(&mut player, 3));
        // World authority never reaches staff.
        mute(&mut room, 1, 4);
        assert!(!was_muted(&mut moderator, 4));
        // A moderator cannot mute an admin.
        mute(&mut room, 4, 5);
        assert!(!was_muted(&mut admin, 5));
        // A moderator who does not own the world can mute a player.
        mute(&mut room, 4, 3);
        assert!(was_muted(&mut player, 3));
    }
}
