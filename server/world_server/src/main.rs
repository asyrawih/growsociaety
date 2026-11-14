//! Authoritative game server (docs/knowledge/10). `cargo run -p world_server [port]`.
//! Its self-signed certificate lives in `world_server_cert.der` (or `$GAME_SERVER_CERT`), the key
//! next to it in `<cert>.key`. Both are created on the first start and reused after that, so a
//! restart does not break the clients that pin the certificate.

mod accounts;
mod commands;
mod conn;
mod http;
mod leases;
mod reset;
mod store;
mod token;
mod voice;
mod world;

use std::net::{Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Instant;

use game_protocol::{DEFAULT_PORT, SERVER_NAME};
use quinn::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

/// The server's certificate and its private key.
type Identity = (CertificateDer<'static>, PrivatePkcs8KeyDer<'static>);

use crate::conn::Server;
use crate::world::Clock;

/// Where the dev certificate is written, unless `$GAME_SERVER_CERT` says otherwise.
const CERT_FILE: &str = "world_server_cert.der";
/// Demo accounts `demo1`..`demo10`, password = name, created at startup if missing.
const DEMO_ACCOUNTS: u32 = 10;
/// The login API, unless `$GAME_HTTP_ADDR` says otherwise. Production: behind nginx (TLS).
const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:8090";

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A new self-signed certificate for `SERVER_NAME`.
fn new_identity() -> Result<Identity, BoxError> {
    let certified = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_owned()])?;
    let cert = CertificateDer::from(certified.cert.der().to_vec());
    let key = PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
    Ok((cert, key))
}

/// The identity stored at `cert_path` (+ `.key`), or a new one written there if either is missing.
fn load_or_create_identity(cert_path: &str) -> Result<Identity, BoxError> {
    let key_path = format!("{cert_path}.key");
    if let (Ok(cert), Ok(key)) = (std::fs::read(cert_path), std::fs::read(&key_path)) {
        return Ok((CertificateDer::from(cert), PrivatePkcs8KeyDer::from(key)));
    }
    let (cert, key) = new_identity()?;
    write_private(&key_path, key.secret_pkcs8_der())?;
    std::fs::write(cert_path, &cert)?;
    Ok((cert, key))
}

/// Writes a file only its owner can read (the private key).
fn write_private(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(bytes)
}

/// A listening endpoint presenting `identity`.
fn listen(addr: SocketAddr, (cert, key): Identity) -> Result<quinn::Endpoint, BoxError> {
    let config = quinn::ServerConfig::with_single_cert(vec![cert], key.into())?;
    Ok(quinn::Endpoint::server(config, addr)?)
}

/// A comma-separated list from the environment; empty tokens are ignored.
fn tokens(var: &str) -> Vec<String> {
    std::env::var(var)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Accepts connections until the endpoint closes.
async fn serve(
    endpoint: quinn::Endpoint,
    accounts: accounts::Accounts,
    signer: token::Signer,
    http: Option<SocketAddr>,
) {
    let server = Arc::new(Server {
        registry: Default::default(),
        clock: Clock(Instant::now()),
        // 0 is never handed out, so a default `PlayerId` can never be a real player.
        next_id: AtomicU32::new(1),
        slots: Default::default(),
        max_players: conn::MAX_PLAYERS,
        admin_tokens: tokens("GAME_ADMIN_TOKENS"),
        mod_tokens: tokens("GAME_MOD_TOKENS"),
        super_admins: tokens("GAME_SUPER_ADMINS"),
        accounts: Arc::new(accounts),
        identity: leases::Identity::from_env(),
        signer,
        online: Default::default(),
        failures: Default::default(),
    });
    {
        let server = server.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(leases::RENEW_EVERY);
            loop {
                every.tick().await;
                server
                    .accounts
                    .heartbeat(&server.identity, server.slots.online())
                    .await;
            }
        });
    }
    if let Some(addr) = http {
        let server = server.clone();
        tokio::spawn(async move {
            if let Err(error) = http::serve(addr, server).await {
                eprintln!("login API on {addr}: {error}");
            }
        });
    }
    while let Some(incoming) = endpoint.accept().await {
        let server = server.clone();
        tokio::spawn(async move {
            if let Ok(conn) = incoming.await {
                conn::handle(server, conn).await;
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // livekit-api's HTTPS (reqwest rustls-no-provider) panics without a process-wide provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let port = match std::env::args().nth(1) {
        Some(arg) => arg.parse()?,
        None => DEFAULT_PORT,
    };
    // Dual-stack: `localhost` may resolve to ::1 or 127.0.0.1.
    let cert_path = std::env::var("GAME_SERVER_CERT").unwrap_or_else(|_| CERT_FILE.into());
    let identity = load_or_create_identity(&cert_path)?;
    let endpoint = listen(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)), identity)?;
    println!("world_server listening on port {port}, certificate in {cert_path}");
    let accounts = accounts::Accounts::open(std::env::var("DATABASE_URL").ok()).await?;
    let store = if accounts.allows_dev() {
        "in memory (no DATABASE_URL: development logins allowed)"
    } else {
        "in PostgreSQL"
    };
    println!("accounts {store}");
    accounts
        .seed_demo(DEMO_ACCOUNTS)
        .await
        .map_err(|e| e.message())?;
    let http: SocketAddr = std::env::var("GAME_HTTP_ADDR")
        .unwrap_or_else(|_| DEFAULT_HTTP_ADDR.into())
        .parse()?;
    serve(endpoint, accounts, token::Signer::from_env(), Some(http)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! The whole path over real QUIC on localhost: two clients, movement and an action.
    use super::*;
    use game_protocol::{
        ChatKind, ClientMsg, MAX_SERVER_MSG, PROTOCOL_VERSION, ServerMsg, decode, read_msg,
        write_msg,
    };
    use game_shared::physics::Input;
    use game_shared::rules::{Action, Event, PlayerId};
    use game_shared::world_to_tile;
    use quinn::rustls::RootCertStore;
    use std::time::Duration;

    struct Client {
        conn: quinn::Connection,
        send: quinn::SendStream,
        recv: quinn::RecvStream,
        id: PlayerId,
        spawn_x: f32,
        _endpoint: quinn::Endpoint,
    }

    async fn join(
        addr: SocketAddr,
        cert: &CertificateDer<'static>,
        protocol: u16,
    ) -> Result<Client, String> {
        let mut roots = RootCertStore::empty();
        roots.add(cert.clone()).map_err(|e| e.to_string())?;
        let config = quinn::ClientConfig::with_root_certificates(Arc::new(roots))
            .map_err(|e| e.to_string())?;
        let mut endpoint =
            quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).map_err(|e| e.to_string())?;
        endpoint.set_default_client_config(config);
        let conn = endpoint
            .connect(addr, SERVER_NAME)
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
        let hello = ClientMsg::Hello {
            protocol,
            auth: game_protocol::Auth::Dev {
                token: "test".into(),
                name: "tester".into(),
            },
            country: Some("id".into()),
        };
        write_msg(&mut send, &hello)
            .await
            .map_err(|e| e.to_string())?;
        let id = match read_msg(&mut recv, MAX_SERVER_MSG).await {
            Ok(ServerMsg::Welcome { player, .. }) => player,
            other => return Err(format!("{other:?}")),
        };
        write_msg(
            &mut send,
            &ClientMsg::Join {
                world: "net".into(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        let spawn_x = match read_msg(&mut recv, MAX_SERVER_MSG).await {
            Ok(ServerMsg::Joined { snapshot }) => snapshot.you.body.pos.0,
            other => return Err(format!("{other:?}")),
        };
        Ok(Client {
            conn,
            send,
            recv,
            id,
            spawn_x,
            _endpoint: endpoint,
        })
    }

    /// Reads stream messages until one matches, within a timeout.
    async fn expect_msg(
        client: &mut Client,
        mut pick: impl FnMut(&ServerMsg) -> bool,
    ) -> ServerMsg {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let msg: ServerMsg = read_msg(&mut client.recv, MAX_SERVER_MSG)
                    .await
                    .expect("stream open");
                if pick(&msg) {
                    return msg;
                }
            }
        })
        .await
        .expect("message in time")
    }

    #[test]
    fn the_certificate_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("cert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cert.der").to_string_lossy().into_owned();
        let first = load_or_create_identity(&path).expect("created");
        let again = load_or_create_identity(&path).expect("loaded");
        assert_eq!(first.0, again.0);
        assert_eq!(first.1.secret_pkcs8_der(), again.1.secret_pkcs8_der());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(format!("{path}.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the key is private");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn two_clients_see_each_other_move_and_act() {
        let identity = new_identity().expect("identity");
        let cert = identity.0.clone();
        let endpoint = listen("127.0.0.1:0".parse().unwrap(), identity).expect("listen");
        let addr = endpoint.local_addr().expect("bound");
        tokio::spawn(serve(
            endpoint,
            accounts::Accounts::open(None).await.expect("memory"),
            token::Signer::new(b"test"),
            None,
        ));

        let wrong = join(addr, &cert, PROTOCOL_VERSION + 1).await;
        assert!(
            wrong.is_err_and(|e| e.contains("protocol")),
            "version mismatch is refused"
        );

        let mut a = join(addr, &cert, PROTOCOL_VERSION).await.expect("a joins");
        let mut b = join(addr, &cert, PROTOCOL_VERSION).await.expect("b joins");
        assert_ne!(a.id, b.id);
        let b_id = b.id;
        let joined = expect_msg(
            &mut a,
            |m| matches!(m, ServerMsg::PlayerJoined { id, .. } if *id == b_id),
        )
        .await;
        // The display name from Hello, not the world name.
        assert!(
            matches!(&joined, ServerMsg::PlayerJoined { name, country, .. }
                if name == "tester" && country.as_deref() == Some("id")),
            "{joined:?}"
        );

        // A walks right; B's datagrams show A moving and acknowledge nothing of B's own.
        let right = Input {
            move_x: 1,
            ..Input::default()
        };
        for seq in 1..=20 {
            write_msg(&mut a.send, &ClientMsg::Input { seq, input: right })
                .await
                .expect("send");
        }
        let a_id = a.id;
        // Both spawn on the door, so A's start is B's own spawn position.
        let a_start = b.spawn_x;
        let moved = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let bytes = b.conn.read_datagram().await.expect("datagrams");
                let Ok(ServerMsg::Bodies { others, .. }) = decode::<ServerMsg>(&bytes) else {
                    continue;
                };
                let Some(&(_, body)) = others.iter().find(|o| o.0 == a_id) else {
                    continue;
                };
                if body.pos.0 > a_start + 10.0 {
                    return body;
                }
            }
        })
        .await
        .expect("A seen moving");

        // Far away: the server refuses, whatever the client thinks.
        let far = Action::Punch { x: 0, y: 0 };
        write_msg(
            &mut a.send,
            &ClientMsg::Act {
                seq: 1,
                action: far,
            },
        )
        .await
        .expect("send");
        let acted = expect_msg(&mut a, |m| matches!(m, ServerMsg::Acted { seq: 1, .. })).await;
        assert!(
            matches!(acted, ServerMsg::Acted { result: Err(_), .. }),
            "{acted:?}"
        );

        // The ground under A: A gets `Acted`, B gets the same change as `Events`.
        // A replayed seq 1 is ignored, so the next answer A sees is seq 2.
        let (x, y) = world_to_tile(moved.pos.0, moved.pos.1).expect("in the world");
        let below = Action::Punch { x, y: y - 1 };
        write_msg(
            &mut a.send,
            &ClientMsg::Act {
                seq: 1,
                action: below,
            },
        )
        .await
        .expect("send");
        write_msg(
            &mut a.send,
            &ClientMsg::Act {
                seq: 2,
                action: below,
            },
        )
        .await
        .expect("send");
        let acted = expect_msg(&mut a, |m| matches!(m, ServerMsg::Acted { .. })).await;
        let ServerMsg::Acted {
            seq: 2,
            result: Ok(events),
        } = acted
        else {
            panic!("{acted:?}");
        };
        let seen = expect_msg(&mut b, |m| matches!(m, ServerMsg::Events { .. })).await;
        assert_eq!(
            seen,
            ServerMsg::Events {
                events: events.clone()
            }
        );
        assert!(
            matches!(events.as_slice(), [Event::TileDamaged { .. }]),
            "{events:?}"
        );

        // Chat: cleaned by the server, then echoed to everyone in the world.
        write_msg(
            &mut a.send,
            &ClientMsg::Chat {
                text: " hi\u{7} ".into(),
            },
        )
        .await
        .expect("send");
        let said = ServerMsg::Chat {
            from: a_id,
            text: "hi".into(),
            kind: ChatKind::World,
        };
        assert_eq!(
            expect_msg(&mut b, |m| matches!(m, ServerMsg::Chat { .. })).await,
            said
        );
        assert_eq!(
            expect_msg(&mut a, |m| matches!(m, ServerMsg::Chat { .. })).await,
            said
        );

        a.conn.close(0u32.into(), b"done");
        expect_msg(
            &mut b,
            |m| matches!(m, ServerMsg::PlayerLeft { id } if *id == a_id),
        )
        .await;
    }

    /// Sends a world-menu request and waits for its answer.
    async fn ask(client: &mut Client, msg: ClientMsg) -> ServerMsg {
        write_msg(&mut client.send, &msg).await.expect("send");
        expect_msg(client, |m| {
            matches!(m, ServerMsg::WorldStatus { .. } | ServerMsg::Worlds { .. })
        })
        .await
    }

    #[tokio::test]
    async fn the_world_menu_sees_status_and_the_busiest_worlds() {
        let identity = new_identity().expect("identity");
        let cert = identity.0.clone();
        let endpoint = listen("127.0.0.1:0".parse().unwrap(), identity).expect("listen");
        let addr = endpoint.local_addr().expect("bound");
        tokio::spawn(serve(
            endpoint,
            accounts::Accounts::open(None).await.expect("memory"),
            token::Signer::new(b"test"),
            None,
        ));
        let mut a = join(addr, &cert, PROTOCOL_VERSION)
            .await
            .expect("a joins NET");
        use game_protocol::WorldStatus;
        let status = |name: &str, status| ServerMsg::WorldStatus {
            name: name.into(),
            status,
        };
        let world = |name: &str| ClientMsg::WorldStatus { name: name.into() };
        assert_eq!(
            ask(&mut a, world("net")).await,
            status("NET", WorldStatus::Open { players: 1 })
        );
        assert_eq!(
            ask(&mut a, world("admin")).await,
            status("ADMIN", WorldStatus::NotAllowed)
        );
        assert_eq!(
            ask(&mut a, world("brandnew")).await,
            status("BRANDNEW", WorldStatus::New)
        );
        assert_eq!(
            ask(&mut a, ClientMsg::ListWorlds).await,
            ServerMsg::Worlds {
                active: vec![("NET".into(), 1)]
            }
        );
        // A refused name on Join is answered, not kicked: the connection stays and asks on.
        write_msg(&mut a.send, &ClientMsg::Leave)
            .await
            .expect("leave");
        let join_denied = ClientMsg::Join {
            world: "admin".into(),
        };
        assert_eq!(
            ask(&mut a, join_denied).await,
            status("ADMIN", WorldStatus::NotAllowed)
        );
        // Lookups refill at 2/s after a burst of 5; this is the sixth.
        tokio::time::sleep(Duration::from_millis(600)).await;
        // Still connected. NET is empty and was never built in, so it is not stored: New again.
        assert_eq!(
            ask(&mut a, world("net")).await,
            status("NET", WorldStatus::New)
        );
    }

    #[tokio::test]
    async fn a_logged_in_player_gets_a_fresh_token_on_demand() {
        let identity = new_identity().expect("identity");
        let cert = identity.0.clone();
        let endpoint = listen("127.0.0.1:0".parse().unwrap(), identity).expect("listen");
        let addr = endpoint.local_addr().expect("bound");
        tokio::spawn(serve(
            endpoint,
            accounts::Accounts::open(None).await.expect("memory"),
            token::Signer::new(b"test"),
            None,
        ));
        let mut roots = RootCertStore::empty();
        roots.add(cert).expect("cert");
        let config = quinn::ClientConfig::with_root_certificates(Arc::new(roots)).expect("config");
        let mut client = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).expect("client");
        client.set_default_client_config(config);
        let conn = client
            .connect(addr, SERVER_NAME)
            .expect("connect")
            .await
            .expect("connected");
        let (mut send, mut recv) = conn.open_bi().await.expect("stream");
        let hello = ClientMsg::Hello {
            protocol: PROTOCOL_VERSION,
            auth: game_protocol::Auth::Register {
                name: "Fresh".into(),
                password: "password1".into(),
            },
            country: None,
        };
        write_msg(&mut send, &hello).await.expect("hello");
        let Ok(ServerMsg::Welcome { player, .. }) = read_msg(&mut recv, MAX_SERVER_MSG).await
        else {
            panic!("welcome");
        };
        write_msg(&mut send, &ClientMsg::RefreshToken)
            .await
            .expect("ask");
        let token = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(ServerMsg::Token { token }) = read_msg(&mut recv, MAX_SERVER_MSG).await {
                    return token;
                }
            }
        })
        .await
        .expect("token in time");
        let account = token::Signer::new(b"test")
            .verify(&token)
            .expect("a valid token");
        assert_eq!((account.id, account.name.as_str()), (player, "Fresh"));
    }
}
