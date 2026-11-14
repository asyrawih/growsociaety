//! The HTTPS login (docs/knowledge/10, "Authentication"): `POST /api/register` and `POST /api/login`
//! with JSON `{ "name", "password" }` answer `{ "token", "name" }` or `{ "error" }`. Plain HTTP on
//! `GAME_HTTP_ADDR`; nginx terminates TLS in front of it and is the only thing that can reach it.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::accounts::AuthError;
use crate::conn::Server;

/// Bigger bodies are refused before parsing; a name and a 128-byte password fit easily.
const MAX_BODY: usize = 1024;

#[derive(Deserialize)]
struct Credentials {
    name: String,
    password: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Granted {
    pub token: String,
    pub name: String,
}

#[derive(Serialize)]
struct Refused {
    error: &'static str,
}

type Reply = (StatusCode, Json<serde_json::Value>);

pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(server)
}

pub async fn serve(addr: SocketAddr, server: Arc<Server>) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("login API on http://{addr}/api");
    axum::serve(
        listener,
        router(server).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}

/// The caller's address. Behind nginx every request comes from loopback, so only then is the
/// proxy's `X-Real-IP` / `X-Forwarded-For` believed.
fn client_ip(peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    if !peer.ip().is_loopback() {
        return peer.ip();
    }
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
    header("x-real-ip")
        .or_else(|| header("x-forwarded-for").and_then(|v| v.split(',').next()))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(peer.ip())
}

fn reply(result: Result<crate::accounts::Account, AuthError>, server: &Server) -> Reply {
    match result {
        Ok(account) => {
            let granted = Granted {
                token: server.signer.issue(&account),
                name: account.name,
            };
            (StatusCode::OK, Json(serde_json::json!(granted)))
        }
        Err(error) => {
            let status = match error {
                AuthError::WrongLogin => StatusCode::UNAUTHORIZED,
                AuthError::NameTaken => StatusCode::CONFLICT,
                AuthError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
                AuthError::BadName | AuthError::BadPassword => StatusCode::BAD_REQUEST,
            };
            let refused = Refused {
                error: error.message(),
            };
            (status, Json(serde_json::json!(refused)))
        }
    }
}

fn too_many() -> Reply {
    let refused = Refused {
        error: "Too many attempts, try again later",
    };
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(serde_json::json!(refused)),
    )
}

async fn register(
    State(server): State<Arc<Server>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Reply {
    let ip = client_ip(peer, &headers);
    // Registrations count as attempts too, so one address cannot mass-create accounts.
    if server.failures.blocked(ip, Instant::now()) {
        return too_many();
    }
    let result = server.accounts.register(&body.name, &body.password).await;
    server.failures.record(ip, Instant::now());
    reply(result, &server)
}

async fn login(
    State(server): State<Arc<Server>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Reply {
    let ip = client_ip(peer, &headers);
    if server.failures.blocked(ip, Instant::now()) {
        return too_many();
    }
    let result = server.accounts.login(&body.name, &body.password).await;
    if result.is_err() {
        server.failures.record(ip, Instant::now());
    }
    reply(result, &server)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_local_proxy_may_name_the_client() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.7".parse().unwrap());
        let proxy: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let stranger: SocketAddr = "198.51.100.1:5000".parse().unwrap();
        assert_eq!(
            client_ip(proxy, &headers),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            client_ip(stranger, &headers),
            stranger.ip(),
            "a direct caller cannot spoof"
        );
    }
}
