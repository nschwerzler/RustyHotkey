//! Small synchronous storage facade over the embedded Turso database.
//!
//! The facade is intentionally application-neutral.  Callers choose a database
//! path outside the repository and own their schema; the built-in cache uses
//! the `rhk_cache` table only.

use std::{
    collections::BTreeMap,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value as JsonValue;
use tokio::runtime::Runtime;

const MAX_SQL_BYTES: usize = 1 << 20;
const MAX_PARAMS: usize = 256;
const MAX_BATCH: usize = 256;
const MAX_ROWS: usize = 100_000;

/// A parameterized statement used by [`Store::transaction`].
#[derive(Clone, Debug)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<JsonValue>,
}

impl Statement {
    pub fn new(sql: impl Into<String>, params: Vec<JsonValue>) -> Self {
        Self {
            sql: sql.into(),
            params,
        }
    }
}

/// A bounded, synchronous wrapper suitable for CLI and scripting callers.
pub struct Store {
    runtime: Runtime,
    connection: turso::Connection,
}

impl Store {
    /// Open or create a local database at the caller-provided path.
    pub fn open(path: impl AsRef<Path>) -> crate::Result<Self> {
        let path = path
            .as_ref()
            .to_str()
            .ok_or_else(|| "database path is not valid UTF-8".to_string())?
            .to_owned();
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        let connection = runtime.block_on(async {
            let database = turso::Builder::new_local(&path)
                .build()
                .await
                .map_err(|e| e.to_string())?;
            database.connect().map_err(|e| e.to_string())
        })?;
        Ok(Self {
            runtime,
            connection,
        })
    }

    pub fn execute(&self, sql: &str, params: &[JsonValue]) -> crate::Result<u64> {
        validate(sql, params.len())?;
        let values = params
            .iter()
            .map(to_turso)
            .collect::<crate::Result<Vec<_>>>()?;
        self.runtime
            .block_on(self.connection.execute(sql, values))
            .map_err(|e| e.to_string())
    }

    /// Return rows as objects keyed by column name. Duplicate column names use
    /// the last value, matching the behavior of most JSON database adapters.
    pub fn query(
        &self,
        sql: &str,
        params: &[JsonValue],
    ) -> crate::Result<Vec<BTreeMap<String, JsonValue>>> {
        validate(sql, params.len())?;
        let values = params
            .iter()
            .map(to_turso)
            .collect::<crate::Result<Vec<_>>>()?;
        self.runtime.block_on(async {
            let mut rows = self
                .connection
                .query(sql, values)
                .await
                .map_err(|e| e.to_string())?;
            let names = rows.column_names();
            let mut result = Vec::new();
            while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
                if result.len() >= MAX_ROWS {
                    return Err("query exceeds 100000 rows".into());
                }
                let mut item = BTreeMap::new();
                for (index, name) in names.iter().enumerate() {
                    let value = row.get_value(index).map_err(|e| e.to_string())?;
                    item.insert(name.clone(), from_turso(value));
                }
                result.push(item);
            }
            Ok(result)
        })
    }

    /// Execute all statements atomically. A failed statement rolls the batch back.
    pub fn transaction(&self, statements: &[Statement]) -> crate::Result<Vec<u64>> {
        if statements.len() > MAX_BATCH {
            return Err("transaction exceeds 256 statements".into());
        }
        let mut batch = Vec::with_capacity(statements.len());
        for statement in statements {
            validate(&statement.sql, statement.params.len())?;
            let params = statement
                .params
                .iter()
                .map(to_turso)
                .collect::<crate::Result<Vec<_>>>()?;
            batch.push(
                turso::BatchStatement::new(&statement.sql, params).map_err(|e| e.to_string())?,
            );
        }
        self.runtime
            .block_on(
                self.connection
                    .transactional_batch(batch, turso::transaction::TransactionBehavior::Deferred),
            )
            .map(|results| {
                results
                    .into_iter()
                    .map(|result| result.rows_affected())
                    .collect()
            })
            .map_err(|e| e.to_string())
    }

    /// Apply a numbered, one-statement migration once.
    pub fn migrate(&self, version: i64, sql: &str) -> crate::Result<bool> {
        if version < 0 {
            return Err("migration version must be non-negative".into());
        }
        validate(sql, 0)?;
        self.execute("CREATE TABLE IF NOT EXISTS rhk_schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)", &[])?;
        if !self
            .query(
                "SELECT 1 FROM rhk_schema_migrations WHERE version = ?1",
                &[JsonValue::from(version)],
            )?
            .is_empty()
        {
            return Ok(false);
        }
        self.transaction(&[
            Statement::new(sql, vec![]),
            Statement::new(
                "INSERT INTO rhk_schema_migrations(version, applied_at) VALUES (?1, ?2)",
                vec![JsonValue::from(version), JsonValue::from(now_ms()?)],
            ),
        ])?;
        Ok(true)
    }

    pub fn cache_put(
        &self,
        namespace: &str,
        key: &str,
        value: JsonValue,
        ttl_ms: u64,
    ) -> crate::Result<()> {
        let now = now_ms()?;
        let ttl = i64::try_from(ttl_ms).map_err(|_| "cache TTL is too large".to_string())?;
        let expires = if ttl_ms == 0 {
            i64::MAX
        } else {
            now.checked_add(ttl)
                .ok_or_else(|| "cache TTL overflows timestamp".to_string())?
        };
        self.execute("CREATE TABLE IF NOT EXISTS rhk_cache (namespace TEXT NOT NULL, cache_key TEXT NOT NULL, value TEXT NOT NULL, cached_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, PRIMARY KEY(namespace, cache_key))", &[])?;
        self.execute("INSERT INTO rhk_cache(namespace, cache_key, value, cached_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(namespace, cache_key) DO UPDATE SET value=excluded.value, cached_at=excluded.cached_at, expires_at=excluded.expires_at", &[JsonValue::from(namespace), JsonValue::from(key), JsonValue::from(serde_json::to_string(&value).map_err(|e| e.to_string())?), JsonValue::from(now), JsonValue::from(expires)])?;
        Ok(())
    }

    pub fn cache_get(&self, namespace: &str, key: &str) -> crate::Result<Option<JsonValue>> {
        self.execute("CREATE TABLE IF NOT EXISTS rhk_cache (namespace TEXT NOT NULL, cache_key TEXT NOT NULL, value TEXT NOT NULL, cached_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, PRIMARY KEY(namespace, cache_key))", &[])?;
        let rows = self.query(
            "SELECT value, expires_at FROM rhk_cache WHERE namespace = ?1 AND cache_key = ?2",
            &[JsonValue::from(namespace), JsonValue::from(key)],
        )?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let expires = row
            .get("expires_at")
            .and_then(JsonValue::as_i64)
            .ok_or_else(|| "invalid cache expiry".to_string())?;
        if expires <= now_ms()? {
            self.cache_invalidate(namespace, key)?;
            return Ok(None);
        }
        let raw = row
            .get("value")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "invalid cache value".to_string())?;
        serde_json::from_str(raw)
            .map(Some)
            .map_err(|e| e.to_string())
    }

    pub fn cache_invalidate(&self, namespace: &str, key: &str) -> crate::Result<()> {
        self.ensure_cache_table()?;
        self.execute(
            "DELETE FROM rhk_cache WHERE namespace = ?1 AND cache_key = ?2",
            &[JsonValue::from(namespace), JsonValue::from(key)],
        )
        .map(|_| ())
    }

    pub fn cache_clear(&self, namespace: &str) -> crate::Result<()> {
        self.ensure_cache_table()?;
        self.execute(
            "DELETE FROM rhk_cache WHERE namespace = ?1",
            &[JsonValue::from(namespace)],
        )
        .map(|_| ())
    }

    pub fn cache_prune(&self) -> crate::Result<u64> {
        self.ensure_cache_table()?;
        self.execute(
            "DELETE FROM rhk_cache WHERE expires_at <= ?1",
            &[JsonValue::from(now_ms()?)],
        )
    }

    fn ensure_cache_table(&self) -> crate::Result<()> {
        self.execute("CREATE TABLE IF NOT EXISTS rhk_cache (namespace TEXT NOT NULL, cache_key TEXT NOT NULL, value TEXT NOT NULL, cached_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, PRIMARY KEY(namespace, cache_key))", &[]).map(|_| ())
    }
}

fn validate(sql: &str, params: usize) -> crate::Result<()> {
    if sql.is_empty() || sql.len() > MAX_SQL_BYTES {
        return Err("SQL must be between 1 byte and 1 MiB".into());
    }
    if params > MAX_PARAMS {
        return Err("statement exceeds 256 parameters".into());
    }
    Ok(())
}

fn now_ms() -> crate::Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())
        .and_then(|d| {
            i64::try_from(d.as_millis())
                .map_err(|_| "system clock exceeds SQLite timestamp range".into())
        })
}

fn to_turso(value: &JsonValue) -> crate::Result<turso::Value> {
    Ok(match value {
        JsonValue::Null => turso::Value::Null,
        JsonValue::Bool(value) => turso::Value::Integer(i64::from(*value)),
        JsonValue::Number(value) => value
            .as_i64()
            .map(turso::Value::Integer)
            .or_else(|| value.as_f64().map(turso::Value::Real))
            .ok_or_else(|| "unsupported JSON number".to_string())?,
        JsonValue::String(value) => turso::Value::Text(value.clone()),
        JsonValue::Array(_) | JsonValue::Object(_) => {
            return Err(
                "SQL parameters support only null, bool, number and string JSON values".into(),
            );
        }
    })
}

fn from_turso(value: turso::Value) -> JsonValue {
    match value {
        turso::Value::Null => JsonValue::Null,
        turso::Value::Integer(value) => JsonValue::from(value),
        turso::Value::Real(value) => JsonValue::from(value),
        turso::Value::Text(value) => JsonValue::from(value),
        turso::Value::Blob(value) => {
            JsonValue::Array(value.into_iter().map(JsonValue::from).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn store() -> (Store, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rustyhotkey-store-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_file(&path);
        (Store::open(&path).unwrap(), path)
    }

    #[test]
    fn parameterized_rows_and_reopen() {
        let (db, path) = store();
        db.execute("CREATE TABLE items (name TEXT, n INTEGER, note TEXT)", &[])
            .unwrap();
        db.execute(
            "INSERT INTO items VALUES (?1, ?2, ?3)",
            &[
                JsonValue::from("O'Reilly"),
                JsonValue::from(7),
                JsonValue::Null,
            ],
        )
        .unwrap();
        let row = db
            .query("SELECT name, n, note FROM items", &[])
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(row["name"], JsonValue::from("O'Reilly"));
        assert_eq!(row["n"], JsonValue::from(7));
        assert!(row["note"].is_null());
        drop(db);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(
            reopened
                .query("SELECT count(*) AS n FROM items", &[])
                .unwrap()[0]["n"],
            JsonValue::from(1)
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn transaction_rolls_back_and_cache_expires() {
        let (db, path) = store();
        db.execute("CREATE TABLE t (n INTEGER UNIQUE)", &[])
            .unwrap();
        let result = db.transaction(&[
            Statement::new("INSERT INTO t VALUES (?1)", vec![JsonValue::from(1)]),
            Statement::new("INSERT INTO t VALUES (?1)", vec![JsonValue::from(1)]),
        ]);
        assert!(result.is_err());
        assert_eq!(
            db.query("SELECT count(*) AS n FROM t", &[]).unwrap()[0]["n"],
            JsonValue::from(0)
        );
        db.cache_put("a", "k", JsonValue::from("v"), 5000).unwrap();
        assert_eq!(db.cache_get("a", "k").unwrap(), Some(JsonValue::from("v")));
        db.execute(
            "UPDATE rhk_cache SET expires_at = 1 WHERE namespace = 'a' AND cache_key = 'k'",
            &[],
        )
        .unwrap();
        assert_eq!(db.cache_get("a", "k").unwrap(), None);
        db.cache_put("a", "x", JsonValue::from(1), 1000).unwrap();
        db.cache_put("b", "x", JsonValue::from(2), 1000).unwrap();
        db.cache_clear("a").unwrap();
        assert!(db.cache_get("a", "x").unwrap().is_none());
        assert!(db.cache_get("b", "x").unwrap().is_some());
        db.cache_put("a", "durable", JsonValue::from(true), 0)
            .unwrap();
        assert_eq!(
            db.cache_get("a", "durable").unwrap(),
            Some(JsonValue::from(true))
        );
        drop(db);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn migrations_are_idempotent() {
        let (db, path) = store();
        assert!(db.migrate(1, "CREATE TABLE migrated (v INTEGER)").unwrap());
        assert!(!db.migrate(1, "CREATE TABLE migrated (v INTEGER)").unwrap());
        drop(db);
        let _ = fs::remove_file(path);
    }
}
