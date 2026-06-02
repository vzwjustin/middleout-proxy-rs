use std::collections::{HashMap, BTreeMap};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use rusqlite::{params, Connection, Result};
use parking_lot::Mutex;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct CachedResponse {
    pub status_code: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub media_type: Option<String>,
    pub inserted_at: f64,
    pub last_hit_at: f64,
    pub hit_count: i64,
}

impl CachedResponse {
    pub fn new(status_code: u16, headers: HashMap<String, String>, body: Vec<u8>, media_type: Option<String>) -> Self {
        let now = current_time();
        CachedResponse {
            status_code,
            headers,
            body,
            media_type,
            inserted_at: now,
            last_hit_at: now,
            hit_count: 0,
        }
    }
}

fn current_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub struct L1Cache {
    conn: Mutex<Connection>,
    max_entries: usize,
    max_body_bytes: usize,
}

impl L1Cache {
    pub fn new<P: AsRef<Path>>(
        db_path: P,
        max_entries: usize,
        max_body_bytes: usize,
    ) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        
        // Optimize WAL mode and configure database schema
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS l1_cache (
                 cache_key TEXT PRIMARY KEY,
                 status_code INTEGER NOT NULL,
                 headers_json TEXT NOT NULL,
                 body BLOB NOT NULL,
                 media_type TEXT,
                 inserted_at REAL NOT NULL,
                 last_hit_at REAL NOT NULL,
                 hit_count INTEGER NOT NULL DEFAULT 0,
                 body_bytes INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS l1_cache_last_hit ON l1_cache(last_hit_at);"
        )?;

        Ok(L1Cache {
            conn: Mutex::new(conn),
            max_entries,
            max_body_bytes,
        })
    }

    pub fn get(&self, key: &str) -> Option<CachedResponse> {
        let conn = self.conn.lock();
        let mut stmt = match conn.prepare(
            "SELECT status_code, headers_json, body, media_type, inserted_at, last_hit_at, hit_count \
             FROM l1_cache WHERE cache_key = ?"
        ) {
            Ok(s) => s,
            Err(_) => return None,
        };

        let mut rows = match stmt.query(params![key]) {
            Ok(r) => r,
            Err(_) => return None,
        };

        let row = match rows.next() {
            Ok(Some(r)) => r,
            _ => return None,
        };

        let status_code: i32 = row.get(0).ok()?;
        let headers_json: String = row.get(1).ok()?;
        let body: Vec<u8> = row.get(2).ok()?;
        let media_type: Option<String> = row.get(3).ok()?;
        let inserted_at: f64 = row.get(4).ok()?;
        let hit_count: i64 = row.get(6).ok()?;

        let headers: HashMap<String, String> = match serde_json::from_str(&headers_json) {
            Ok(h) => h,
            Err(_) => return None,
        };

        let now = current_time();
        // Update stats, swallowing errors so reads don't fail on transient DB lock issues
        let _ = conn.execute(
            "UPDATE l1_cache SET last_hit_at = ?, hit_count = hit_count + 1 WHERE cache_key = ?",
            params![now, key],
        );

        Some(CachedResponse {
            status_code: status_code as u16,
            headers,
            body,
            media_type,
            inserted_at,
            last_hit_at: now,
            hit_count: hit_count + 1,
        })
    }

    pub fn put(&self, key: &str, response: &CachedResponse) {
        if response.status_code < 200 || response.status_code >= 300 {
            return;
        }
        if response.body.len() > self.max_body_bytes {
            return;
        }

        // Drop auth-leaking headers defensively
        let sorted_headers: BTreeMap<String, String> = response.headers.iter()
            .filter(|(k, _)| {
                let kl = k.to_lowercase();
                kl != "authorization"
                    && kl != "x-api-key"
                    && kl != "anthropic-api-key"
                    && kl != "proxy-authorization"
                    && kl != "set-cookie"
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let headers_json = match serde_json::to_string(&sorted_headers) {
            Ok(j) => j,
            Err(_) => return,
        };

        let now = current_time();
        let conn = self.conn.lock();
        let res = conn.execute(
            "INSERT OR REPLACE INTO l1_cache \
             (cache_key, status_code, headers_json, body, media_type, inserted_at, last_hit_at, hit_count, body_bytes) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                key,
                response.status_code as i32,
                headers_json,
                response.body,
                response.media_type,
                now,
                now,
                0,
                response.body.len() as i64,
            ],
        );

        if res.is_ok() {
            let _ = self.evict_if_over(&conn);
        }
    }

    pub fn stats(&self) -> Value {
        let conn = self.conn.lock();
        let row = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(body_bytes), 0), COALESCE(SUM(hit_count), 0) FROM l1_cache",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        );

        match row {
            Ok((entries, body_bytes, total_hits)) => {
                serde_json::json!({
                    "entries": entries,
                    "body_bytes": body_bytes,
                    "total_hits": total_hits,
                    "max_entries": self.max_entries,
                    "max_body_bytes": self.max_body_bytes,
                })
            }
            Err(_) => {
                serde_json::json!({
                    "entries": 0,
                    "body_bytes": 0,
                    "total_hits": 0,
                    "max_entries": self.max_entries,
                    "max_body_bytes": self.max_body_bytes,
                })
            }
        }
    }

    pub fn clear(&self) {
        let conn = self.conn.lock();
        let _ = conn.execute("DELETE FROM l1_cache", []);
    }

    pub fn purge(&self) -> i64 {
        let conn = self.conn.lock();
        let before: i64 = conn.query_row("SELECT COUNT(*) FROM l1_cache", [], |r| r.get(0)).unwrap_or(0);
        let _ = conn.execute("DELETE FROM l1_cache", []);
        before
    }

    fn evict_if_over(&self, conn: &Connection) -> Result<()> {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM l1_cache", [], |row| row.get(0))?;
        let excess = count - (self.max_entries as i64);
        if excess <= 0 {
            return Ok(());
        }

        conn.execute(
            "DELETE FROM l1_cache WHERE cache_key IN ( \
                 SELECT cache_key FROM l1_cache ORDER BY last_hit_at ASC LIMIT ? \
             )",
            params![excess],
        )?;
        Ok(())
    }
}
