//! Server-level Database for Co-Force (F-17).
//!
//! Manages:
//! - API Tokens (api_tokens) for authentication before routing
//! - Workspaces registry (workspaces)
//! - Audit log (audit_log)
//!
//! This database is stored in server.db (separate from workspace-specific database).

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ServerDatabase {
    conn: tokio_rusqlite::Connection,
}

impl ServerDatabase {
    /// Opens the server database in-memory for testing or at the specified file path.
    pub async fn open(path: &str) -> Result<Self> {
        let conn = if path == ":memory:" {
            tokio_rusqlite::Connection::open_in_memory().await?
        } else {
            tokio_rusqlite::Connection::open(path).await?
        };

        let db = Self { conn };
        db.migrate().await?;
        Ok(db)
    }

    pub fn conn(&self) -> &tokio_rusqlite::Connection {
        &self.conn
    }

    async fn migrate(&self) -> Result<()> {
        self.conn
            .call(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS api_tokens (
                        token_id TEXT PRIMARY KEY,
                        token_hash TEXT NOT NULL UNIQUE,
                        label TEXT,
                        kind TEXT NOT NULL,
                        workspace_scope TEXT NOT NULL,
                        expires_at TEXT,
                        revoked_at TEXT,
                        created_at TEXT NOT NULL,
                        last_used_at TEXT
                    );

                    CREATE TABLE IF NOT EXISTS workspaces (
                        workspace_id TEXT PRIMARY KEY,
                        name TEXT NOT NULL,
                        path TEXT NOT NULL,
                        created_at TEXT NOT NULL
                    );

                    CREATE TABLE IF NOT EXISTS audit_log (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        token_id TEXT,
                        ip_address TEXT,
                        action TEXT NOT NULL,
                        status TEXT NOT NULL,
                        occurred_at TEXT NOT NULL
                    );",
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to run server_db migrations: {e}"))?;

        Ok(())
    }

    /// Hash a raw token using SHA-256.
    pub fn hash_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Generate a new secure API token.
    /// Format: cfk_<kind>_<32 bytes random string>
    pub fn generate_token_raw(kind: &str) -> String {
        let random_part: String = Uuid::new_v4().simple().to_string();
        format!("cfk_{kind}_{random_part}")
    }
}

// ---------------------------------------------------------------------------
// Use cases & helper functions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub token_id: String,
    pub label: Option<String>,
    pub kind: String, // "admin" | "agent" | "enrollment"
    pub workspace_scope: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl ServerDatabase {
    /// Issues a new token and returns the raw token (only shown once).
    pub async fn issue_token(
        &self,
        label: Option<String>,
        kind: &str,
        workspace_scope: &str,
        ttl_hours: Option<i64>,
    ) -> Result<(String, ApiToken)> {
        let raw = Self::generate_token_raw(kind);
        let hash = Self::hash_token(&raw);
        let token_id = Uuid::new_v4().to_string();

        let expires_at = ttl_hours.map(|h| Utc::now() + chrono::Duration::hours(h));
        let created_at = Utc::now().to_rfc3339();
        let expires_str = expires_at.map(|dt| dt.to_rfc3339());

        let tid = token_id.clone();
        let lbl = label.clone();
        let knd = kind.to_string();
        let scope = workspace_scope.to_string();

        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO api_tokens \
                     (token_id, token_hash, label, kind, workspace_scope, expires_at, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![tid, hash, lbl, knd, scope, expires_str, created_at],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert API token: {e}"))?;

        Ok((
            raw,
            ApiToken {
                token_id,
                label,
                kind: kind.to_string(),
                workspace_scope: workspace_scope.to_string(),
                expires_at,
                revoked_at: None,
            },
        ))
    }

    /// Revokes an active token.
    pub async fn revoke_token(&self, token_id: &str) -> Result<()> {
        let tid = token_id.to_string();
        let now = Utc::now().to_rfc3339();

        self.conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE api_tokens SET revoked_at = ?1 WHERE token_id = ?2",
                    rusqlite::params![now, tid],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to revoke token: {e}"))?;

        Ok(())
    }

    /// Validates a raw token string. Returns token info if valid.
    pub async fn validate_token(&self, raw_token: &str) -> Result<ApiToken> {
        let hash = Self::hash_token(raw_token);

        let token_info = self
            .conn
            .call(move |conn| {
                let row_res = conn.query_row(
                    "SELECT token_id, label, kind, workspace_scope, expires_at, revoked_at \
                     FROM api_tokens WHERE token_hash = ?1",
                    [hash],
                    |row| {
                        let expires_str: Option<String> = row.get(4)?;
                        let revoked_str: Option<String> = row.get(5)?;

                        let expires_at = expires_str
                            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                            .map(|dt| dt.with_timezone(&Utc));

                        let revoked_at = revoked_str
                            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                            .map(|dt| dt.with_timezone(&Utc));

                        Ok(ApiToken {
                            token_id: row.get(0)?,
                            label: row.get(1)?,
                            kind: row.get(2)?,
                            workspace_scope: row.get(3)?,
                            expires_at,
                            revoked_at,
                        })
                    },
                )?;
                Ok(row_res)
            })
            .await
            .map_err(|_| anyhow::anyhow!("Token is invalid or does not exist"))?;

        // Checks: expired? revoked?
        if let Some(revoked) = token_info.revoked_at {
            anyhow::bail!("Token has been revoked at {revoked}");
        }

        if let Some(expires) = token_info.expires_at {
            if expires < Utc::now() {
                anyhow::bail!("Token has expired at {expires}");
            }
        }

        // Update last used timestamp asynchronously
        let tid = token_info.token_id.clone();
        let now = Utc::now().to_rfc3339();
        let conn_clone = self.conn.clone();
        tokio::spawn(async move {
            let _ = conn_clone
                .call(move |conn| {
                    let updated = conn.execute(
                        "UPDATE api_tokens SET last_used_at = ?1 WHERE token_id = ?2",
                        rusqlite::params![now, tid],
                    )?;
                    Ok(updated)
                })
                .await;
        });

        Ok(token_info)
    }

    /// Register a workspace in the workspaces registry.
    pub async fn register_workspace(&self, id: &str, name: &str, path: &str) -> Result<()> {
        let id_str = id.to_string();
        let name_str = name.to_string();
        let path_str = path.to_string();
        let now = Utc::now().to_rfc3339();

        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO workspaces (workspace_id, name, path, created_at) \
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![id_str, name_str, path_str, now],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to register workspace: {e}"))?;

        Ok(())
    }

    /// Log an access audit entry.
    pub async fn log_audit(
        &self,
        token_id: Option<String>,
        ip_address: Option<String>,
        action: &str,
        status: &str,
    ) -> Result<()> {
        let tid = token_id;
        let ip = ip_address;
        let act = action.to_string();
        let stat = status.to_string();
        let now = Utc::now().to_rfc3339();

        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO audit_log (token_id, ip_address, action, status, occurred_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![tid, ip, act, stat, now],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to write audit log: {e}"))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Unit Tests (TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_issue_validate_and_revoke_token() {
        let db = ServerDatabase::open(":memory:").await.unwrap();

        // 1. Issue an enrollment token (24h expiry)
        let (raw, token) = db
            .issue_token(
                Some("Trung-MacBook".to_string()),
                "enrollment",
                "*",
                Some(24),
            )
            .await
            .unwrap();

        assert!(raw.starts_with("cfk_enrollment_"));
        assert_eq!(token.kind, "enrollment");
        assert_eq!(token.workspace_scope, "*");

        // 2. Validate token
        let validated = db.validate_token(&raw).await.unwrap();
        assert_eq!(validated.token_id, token.token_id);

        // 3. Revoke token
        db.revoke_token(&token.token_id).await.unwrap();

        // 4. Validate again -> should fail
        let result = db.validate_token(&raw).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("revoked"));
    }

    #[tokio::test]
    async fn test_workspace_registry_and_audit_log() {
        let db = ServerDatabase::open(":memory:").await.unwrap();

        db.register_workspace("ws-1", "co-force", "/path/to/project")
            .await
            .unwrap();

        db.log_audit(
            Some("token-123".to_string()),
            Some("127.0.0.1".to_string()),
            "tools/list",
            "success",
        )
        .await
        .unwrap();
    }
}
