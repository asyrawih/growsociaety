//! Login tokens (docs/knowledge/10, "Authentication"): the HTTPS login hands one out, the client
//! presents it in QUIC `Hello`. Short-lived and signed with HMAC-SHA256 over `GAME_AUTH_SECRET`,
//! so the game port trusts it without a database round trip.
//!
//! Format: `<id>.<name>.<role>.<expires unix secs>.<hex signature>`. Names are `[A-Za-z0-9_]`, so
//! the dots are unambiguous.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use game_protocol::valid_name;
use game_shared::commands::Role;
use game_shared::rules::PlayerId;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::accounts::Account;

/// How long a token may wait between login and `Hello`. Tuning.
pub const TOKEN_TTL: Duration = Duration::from_secs(15 * 60);

/// The signing key. Production: `GAME_AUTH_SECRET`; a development server without it makes a
/// random one per start (tokens from an earlier run stop working, which is fine there).
pub struct Signer(Vec<u8>);

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Player => "player",
        Role::Moderator => "moderator",
        // Owner is never written into a token; the server grants it at Hello from its own list.
        Role::Admin | Role::Owner => "admin",
    }
}

fn role_from(text: &str) -> Option<Role> {
    Some(match text {
        "player" => Role::Player,
        "moderator" => Role::Moderator,
        "admin" => Role::Admin,
        _ => return None,
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

impl Signer {
    pub fn new(secret: &[u8]) -> Self {
        Signer(secret.to_vec())
    }

    /// `GAME_AUTH_SECRET`, or a random key with a warning.
    pub fn from_env() -> Self {
        match std::env::var("GAME_AUTH_SECRET") {
            Ok(secret) if !secret.is_empty() => Signer::new(secret.as_bytes()),
            _ => {
                let mut key = [0u8; 32];
                if getrandom::fill(&mut key).is_err() {
                    panic!("no system randomness for the token key");
                }
                eprintln!("GAME_AUTH_SECRET not set: login tokens use a random key until restart");
                Signer::new(&key)
            }
        }
    }

    fn mac(&self) -> Hmac<Sha256> {
        // HMAC takes keys of any length.
        <Hmac<Sha256> as KeyInit>::new_from_slice(&self.0).expect("any key length")
    }

    pub fn issue(&self, account: &Account) -> String {
        self.issue_until(account, now_secs() + TOKEN_TTL.as_secs())
    }

    fn issue_until(&self, account: &Account, expires: u64) -> String {
        let body = format!(
            "{}.{}.{}.{expires}",
            account.id.0,
            account.name,
            role_name(account.role)
        );
        let mut mac = self.mac();
        mac.update(body.as_bytes());
        format!("{body}.{}", hex(&mac.finalize().into_bytes()))
    }

    /// The account a token names, if the signature is right and it has not expired.
    pub fn verify(&self, token: &str) -> Option<Account> {
        self.verify_at(token, now_secs())
    }

    fn verify_at(&self, token: &str, now: u64) -> Option<Account> {
        let (body, signature) = token.rsplit_once('.')?;
        let mut mac = self.mac();
        mac.update(body.as_bytes());
        // Constant-time comparison.
        mac.verify_slice(&unhex(signature)?).ok()?;
        let mut parts = body.split('.');
        let id: u32 = parts.next()?.parse().ok()?;
        let name = parts.next()?.to_owned();
        let role = role_from(parts.next()?)?;
        let expires: u64 = parts.next()?.parse().ok()?;
        if parts.next().is_some() || expires < now || !valid_name(&name) {
            return None;
        }
        Some(Account {
            id: PlayerId(id),
            name,
            role,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budi() -> Account {
        Account {
            id: PlayerId(42),
            name: "Budi".into(),
            role: Role::Moderator,
        }
    }

    #[test]
    fn a_token_round_trips_until_it_expires() {
        let signer = Signer::new(b"secret");
        let token = signer.issue_until(&budi(), 1_000);
        assert_eq!(signer.verify_at(&token, 999), Some(budi()));
        assert_eq!(signer.verify_at(&token, 1_001), None, "expired");
        assert_eq!(signer.verify(&signer.issue(&budi())), Some(budi()));
    }

    #[test]
    fn a_forged_or_tampered_token_is_refused() {
        let signer = Signer::new(b"secret");
        let token = signer.issue_until(&budi(), 1_000);
        // Promoting yourself breaks the signature.
        let promoted = token.replace(".moderator.", ".admin.");
        assert_eq!(signer.verify_at(&promoted, 0), None);
        // Another key's token is worthless.
        let other = Signer::new(b"other").issue_until(&budi(), 1_000);
        assert_eq!(signer.verify_at(&other, 0), None);
        for junk in [
            "",
            ".",
            "a.b.c.d.e",
            "42.Budi.admin.99999999999.zz",
            &format!("{token}x"),
        ] {
            assert_eq!(signer.verify_at(junk, 0), None, "{junk}");
        }
    }
}
