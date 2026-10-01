use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;
use utoipa::ToSchema;
use uuid::Uuid;

const CLAIM_TTL_MS: u64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ClaimCode {
    /// Returned exactly once. Only its SHA-256 digest is persisted.
    pub code: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Account {
    pub id: Uuid,
    pub email: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ClaimedAccount {
    pub account: Account,
    /// Returned exactly once. Only its digest is persisted.
    pub api_token: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OnboardingError {
    #[error("onboarding database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("onboarding database lock poisoned")]
    Poisoned,
    #[error("claim code is invalid or expired")]
    InvalidClaim,
    #[error("email already has an account")]
    EmailConflict,
    #[error("email address is invalid")]
    InvalidEmail,
    #[error("timestamp overflow")]
    TimestampOverflow,
}

pub struct AccountRegistry {
    connection: Mutex<Connection>,
}

impl AccountRegistry {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OnboardingError> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self, OnboardingError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, OnboardingError> {
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS agent_claims (
               code_hash BLOB PRIMARY KEY,
               expires_at_ms INTEGER NOT NULL,
               claimed_at_ms INTEGER,
               account_id TEXT
             );
             CREATE TABLE IF NOT EXISTS accounts (
               id TEXT PRIMARY KEY,
               email TEXT NOT NULL UNIQUE,
               created_at_ms INTEGER NOT NULL,
               api_token_hash BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS account_profiles (
               account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
               profile_id TEXT NOT NULL UNIQUE,
               PRIMARY KEY (account_id, profile_id)
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn issue_claim(&self, now_ms: u64) -> Result<ClaimCode, OnboardingError> {
        let code = Uuid::new_v4().simple().to_string();
        let expires_at_ms = now_ms
            .checked_add(CLAIM_TTL_MS)
            .ok_or(OnboardingError::TimestampOverflow)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| OnboardingError::Poisoned)?;
        connection.execute(
            "INSERT INTO agent_claims (code_hash, expires_at_ms) VALUES (?1, ?2)",
            params![hash_code(&code), to_i64(expires_at_ms)?],
        )?;
        Ok(ClaimCode {
            code,
            expires_at_ms,
        })
    }

    pub fn claim(
        &self,
        code: &str,
        email: &str,
        now_ms: u64,
    ) -> Result<ClaimedAccount, OnboardingError> {
        let email = normalize_email(email)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| OnboardingError::Poisoned)?;
        let tx = connection.transaction()?;
        let claim = tx
            .query_row(
                "SELECT expires_at_ms, claimed_at_ms FROM agent_claims WHERE code_hash = ?1",
                params![hash_code(code)],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()?;
        let Some((expires_at_ms, claimed_at_ms)) = claim else {
            return Err(OnboardingError::InvalidClaim);
        };
        if claimed_at_ms.is_some() || expires_at_ms <= to_i64(now_ms)? {
            return Err(OnboardingError::InvalidClaim);
        }
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM accounts WHERE email = ?1",
                [&email],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(OnboardingError::EmailConflict);
        }
        let account = Account {
            id: Uuid::new_v4(),
            email,
            created_at_ms: now_ms,
        };
        let api_token = format!("tardy_{}", Uuid::new_v4().simple());
        tx.execute(
            "INSERT INTO accounts (id, email, created_at_ms, api_token_hash) VALUES (?1, ?2, ?3, ?4)",
            params![account.id.to_string(), account.email, to_i64(now_ms)?, hash_code(&api_token)],
        )?;
        tx.execute(
            "UPDATE agent_claims SET claimed_at_ms = ?1, account_id = ?2 WHERE code_hash = ?3",
            params![to_i64(now_ms)?, account.id.to_string(), hash_code(code)],
        )?;
        tx.commit()?;
        Ok(ClaimedAccount { account, api_token })
    }

    pub fn authenticate(&self, api_token: &str) -> Result<Uuid, OnboardingError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| OnboardingError::Poisoned)?;
        let id: Option<String> = connection
            .query_row(
                "SELECT id FROM accounts WHERE api_token_hash = ?1",
                params![hash_code(api_token)],
                |row| row.get(0),
            )
            .optional()?;
        id.and_then(|id| Uuid::parse_str(&id).ok())
            .ok_or(OnboardingError::InvalidClaim)
    }

    pub fn bind_profile(&self, account_id: Uuid, profile_id: Uuid) -> Result<(), OnboardingError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| OnboardingError::Poisoned)?;
        connection.execute(
            "INSERT INTO account_profiles (account_id, profile_id) VALUES (?1, ?2)",
            params![account_id.to_string(), profile_id.to_string()],
        )?;
        Ok(())
    }

    pub fn owns_profile(
        &self,
        account_id: Uuid,
        profile_id: Uuid,
    ) -> Result<bool, OnboardingError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| OnboardingError::Poisoned)?;
        let found: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM account_profiles WHERE account_id = ?1 AND profile_id = ?2",
                params![account_id.to_string(), profile_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }
}

fn hash_code(code: &str) -> Vec<u8> {
    Sha256::digest(code.as_bytes()).to_vec()
}

fn to_i64(value: u64) -> Result<i64, OnboardingError> {
    value
        .try_into()
        .map_err(|_| OnboardingError::TimestampOverflow)
}

fn normalize_email(value: &str) -> Result<String, OnboardingError> {
    let value = value.trim().to_ascii_lowercase();
    let valid = value.len() <= 254
        && value.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
        });
    valid.then_some(value).ok_or(OnboardingError::InvalidEmail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_is_durable_single_use_and_normalizes_email() {
        let path = std::env::temp_dir().join(format!("tardy-onboarding-{}.sqlite", Uuid::new_v4()));
        let registry = AccountRegistry::open(&path).unwrap();
        let ticket = registry.issue_claim(1_000).unwrap();
        drop(registry);
        let registry = AccountRegistry::open(&path).unwrap();
        let account = registry
            .claim(&ticket.code, " Agent@Example.COM ", 2_000)
            .unwrap();
        assert_eq!(account.account.email, "agent@example.com");
        assert_eq!(
            registry.authenticate(&account.api_token).unwrap(),
            account.account.id
        );
        assert!(matches!(
            registry.claim(&ticket.code, "other@example.com", 3_000),
            Err(OnboardingError::InvalidClaim)
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn expired_codes_are_rejected() {
        let registry = AccountRegistry::in_memory().unwrap();
        let ticket = registry.issue_claim(10).unwrap();
        assert!(matches!(
            registry.claim(&ticket.code, "agent@example.com", ticket.expires_at_ms),
            Err(OnboardingError::InvalidClaim)
        ));
    }
}
