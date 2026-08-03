use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub struct Db(pub Mutex<Connection>);

/// A second connection, opened read-only, so search queries never queue behind an
/// indexing write transaction held on [`Db`]. Safe under WAL mode (set in `init_db`),
/// which lets readers and a single writer proceed concurrently.
pub struct ReadDb(pub Mutex<Connection>);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextChunkInput {
    pub chunk_index: i64,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkEmbeddingInput {
    pub chunk_index: i64,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub document_count: i64,
    pub chunk_count: i64,
    pub embedded_chunk_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMtime {
    pub path: String,
    pub modified_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChunk {
    pub path: String,
    pub chunk_index: i64,
    pub text: String,
}

/// Schema is versioned via `PRAGMA user_version` so an existing local database (from
/// before embeddings were nullable) gets migrated in place instead of silently keeping
/// its old, incompatible constraints.
pub fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    // journal_mode = WAL persists as a property of the database file itself, so it's
    // enough to set it once here on the writer connection - the second, read-only
    // connection opened in lib.rs's `setup` picks it up automatically. `synchronous` is
    // per-connection but only affects writes, so it doesn't need setting there either.
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;

    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        migrate_to_v1(conn)?;
    }
    if version < 2 {
        migrate_to_v2(conn)?;
    }

    Ok(())
}

fn migrate_to_v1(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS documents (
             path TEXT PRIMARY KEY,
             modified_ms INTEGER NOT NULL
         );",
    )?;

    let chunks_exists: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'chunks'",
        [],
        |r| r.get::<_, i64>(0),
    )? > 0;

    if chunks_exists {
        // Pre-v1 schema had `embedding`/`dim` as NOT NULL. Rebuild the table so text can
        // be stored (and become keyword-searchable) before its embedding is computed,
        // preserving whatever was already indexed.
        conn.execute_batch(
            "ALTER TABLE chunks RENAME TO chunks_v0;
             CREATE TABLE chunks (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 path TEXT NOT NULL REFERENCES documents(path) ON DELETE CASCADE,
                 chunk_index INTEGER NOT NULL,
                 text TEXT NOT NULL,
                 embedding BLOB,
                 dim INTEGER
             );
             INSERT INTO chunks (id, path, chunk_index, text, embedding, dim)
                 SELECT id, path, chunk_index, text, embedding, dim FROM chunks_v0;
             DROP TABLE chunks_v0;",
        )?;
    } else {
        conn.execute_batch(
            "CREATE TABLE chunks (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 path TEXT NOT NULL REFERENCES documents(path) ON DELETE CASCADE,
                 chunk_index INTEGER NOT NULL,
                 text TEXT NOT NULL,
                 embedding BLOB,
                 dim INTEGER
             );",
        )?;
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_chunks_path ON chunks(path);
         PRAGMA user_version = 1;",
    )
}

/// Adds an FTS5 external-content index over `chunks.text`, replacing what used to be a
/// hand-rolled BM25 implementation that re-tokenized the entire corpus on every search
/// (see `hybrid_search.rs`). `content='chunks'` keeps chunk text from being duplicated on
/// disk; SQLite maintains the index incrementally from here on via the three triggers
/// below, which cover every write path in this file:
///
/// - `upsert_document_chunks_text` deletes then re-inserts chunks for a path - covered by
///   the delete and insert triggers.
/// - `update_chunk_embeddings` only ever writes `embedding`/`dim`, never `text` - the
///   update trigger is scoped to `UPDATE OF text` specifically so that call is a no-op
///   here rather than needlessly deleting/reinserting the FTS row on every embedding
///   attach during Phase 2 indexing.
/// - `remove_document` deletes from `chunks` directly (see below) - covered by the delete
///   trigger.
fn migrate_to_v2(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE chunks_fts USING fts5(
             text, content='chunks', content_rowid='id', tokenize='unicode61'
         );
         INSERT INTO chunks_fts(chunks_fts) VALUES('rebuild');

         CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
             INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
         END;
         CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
             INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
         END;
         CREATE TRIGGER chunks_au AFTER UPDATE OF text ON chunks BEGIN
             INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
             INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
         END;

         PRAGMA user_version = 2;",
    )
}

pub(crate) fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub path: String,
    pub chunk_index: i64,
    pub text: String,
    pub score: f32,
}

/// Stores chunk text immediately, with no embedding yet - the chunk becomes keyword
/// (BM25) searchable right away, well before the slower embedding step finishes.
#[tauri::command]
pub fn upsert_document_chunks_text(
    db: tauri::State<Db>,
    path: String,
    modified_ms: i64,
    chunks: Vec<TextChunkInput>,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    tx.execute(
        "INSERT INTO documents (path, modified_ms) VALUES (?1, ?2)
         ON CONFLICT(path) DO UPDATE SET modified_ms = excluded.modified_ms",
        params![path, modified_ms],
    )
    .map_err(|e| e.to_string())?;

    tx.execute("DELETE FROM chunks WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;

    {
        let mut stmt = tx
            .prepare("INSERT INTO chunks (path, chunk_index, text) VALUES (?1, ?2, ?3)")
            .map_err(|e| e.to_string())?;
        for chunk in &chunks {
            stmt.execute(params![path, chunk.chunk_index, chunk.text])
                .map_err(|e| e.to_string())?;
        }
    }

    tx.commit().map_err(|e| e.to_string())
}

/// Attaches embeddings to chunks already stored by [`upsert_document_chunks_text`],
/// matched by (path, chunk_index).
#[tauri::command]
pub fn update_chunk_embeddings(
    db: tauri::State<Db>,
    path: String,
    embeddings: Vec<ChunkEmbeddingInput>,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    {
        let mut stmt = tx
            .prepare(
                "UPDATE chunks SET embedding = ?1, dim = ?2 WHERE path = ?3 AND chunk_index = ?4",
            )
            .map_err(|e| e.to_string())?;
        for chunk in &embeddings {
            let blob = embedding_to_blob(&chunk.embedding);
            stmt.execute(params![
                blob,
                chunk.embedding.len() as i64,
                path,
                chunk.chunk_index
            ])
            .map_err(|e| e.to_string())?;
        }
    }

    tx.commit().map_err(|e| e.to_string())
}

/// Deletes `chunks` explicitly (rather than relying solely on `ON DELETE CASCADE`) so the
/// FTS delete trigger fires unconditionally, without depending on the interaction between
/// foreign-key cascades and triggers.
#[tauri::command]
pub fn remove_document(db: tauri::State<Db>, path: String) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM chunks WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM documents WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_index_stats_cmd(db: tauri::State<Db>) -> Result<IndexStats, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let document_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let chunk_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let embedded_chunk_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE embedding IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(IndexStats {
        document_count,
        chunk_count,
        embedded_chunk_count,
    })
}

#[tauri::command]
pub fn get_indexed_mtimes_cmd(db: tauri::State<Db>) -> Result<Vec<DocumentMtime>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT path, modified_ms FROM documents")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(DocumentMtime {
                path: r.get(0)?,
                modified_ms: r.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Chunks still missing an embedding, regardless of when their text was stored - unlike
/// the mtime-based comparison the frontend uses to decide which *files* to reprocess,
/// this is what lets an interrupted embedding pass (app closed, process restarted) pick
/// back up where it left off instead of being silently skipped forever. `ORDER BY id`
/// gives simple, stable paging: since already-embedded rows drop out of the `WHERE`
/// clause, repeated calls naturally advance without needing an explicit cursor. Read-only,
/// so this runs on `ReadDb` rather than the writer `Db`.
#[tauri::command]
pub fn get_chunks_pending_embedding_cmd(
    db: tauri::State<ReadDb>,
    limit: i64,
) -> Result<Vec<PendingChunk>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT path, chunk_index, text FROM chunks WHERE embedding IS NULL ORDER BY id LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(PendingChunk {
                path: r.get(0)?,
                chunk_index: r.get(1)?,
                text: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    /// Inverse of `embedding_to_blob`, kept test-only since no production code needs to
    /// go from a stored blob back to a `Vec<f32>` (search scores directly off the raw
    /// bytes - see `hybrid_search::dot_bytes`); tests still need it to assert on what got
    /// stored.
    fn decode_blob(blob: &[u8]) -> Vec<f32> {
        blob.chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    #[test]
    fn text_only_insert_leaves_embedding_null() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO documents (path, modified_ms) VALUES (?1, ?2)",
            params!["/tmp/a.txt", 100],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text) VALUES (?1, 0, 'hello')",
            params!["/tmp/a.txt"],
        )
        .unwrap();

        let embedding: Option<Vec<u8>> = conn
            .query_row(
                "SELECT embedding FROM chunks WHERE path = ?1",
                params!["/tmp/a.txt"],
                |r| r.get(0),
            )
            .unwrap();
        assert!(embedding.is_none());
    }

    #[test]
    fn update_chunk_embeddings_attaches_to_existing_row() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO documents (path, modified_ms) VALUES (?1, ?2)",
            params!["/tmp/a.txt", 100],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text) VALUES (?1, 0, 'hello')",
            params!["/tmp/a.txt"],
        )
        .unwrap();

        // Simulate what update_chunk_embeddings does internally, without the State wrapper.
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "UPDATE chunks SET embedding = ?1, dim = ?2 WHERE path = ?3 AND chunk_index = ?4",
            params![embedding_to_blob(&[1.0, 2.0]), 2, "/tmp/a.txt", 0],
        )
        .unwrap();
        tx.commit().unwrap();

        let (embedding, dim): (Vec<u8>, i64) = conn
            .query_row(
                "SELECT embedding, dim FROM chunks WHERE path = ?1",
                params!["/tmp/a.txt"],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(decode_blob(&embedding), vec![1.0, 2.0]);
        assert_eq!(dim, 2);
    }

    /// Inserts a document with a single chunk, embedding it if `embedding` is `Some`.
    fn insert_chunk_with_optional_embedding(
        conn: &Connection,
        path: &str,
        chunk_index: i64,
        text: &str,
        embedding: Option<&[f32]>,
    ) {
        conn.execute(
            "INSERT INTO documents (path, modified_ms) VALUES (?1, 0)
             ON CONFLICT(path) DO NOTHING",
            params![path],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text) VALUES (?1, ?2, ?3)",
            params![path, chunk_index, text],
        )
        .unwrap();
        if let Some(e) = embedding {
            conn.execute(
                "UPDATE chunks SET embedding = ?1, dim = ?2 WHERE path = ?3 AND chunk_index = ?4",
                params![embedding_to_blob(e), e.len() as i64, path, chunk_index],
            )
            .unwrap();
        }
    }

    /// Simulates what get_chunks_pending_embedding_cmd does internally, without the
    /// State wrapper - same convention as the other command tests in this module.
    fn pending_embedding_texts(conn: &Connection, limit: i64) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT text FROM chunks WHERE embedding IS NULL ORDER BY id LIMIT ?1",
            )
            .unwrap();
        stmt.query_map(params![limit], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn pending_embedding_query_returns_only_unembedded_chunks_in_order() {
        let conn = test_db();
        insert_chunk_with_optional_embedding(&conn, "/tmp/a.txt", 0, "already embedded", Some(&[1.0]));
        insert_chunk_with_optional_embedding(&conn, "/tmp/b.txt", 0, "pending one", None);
        insert_chunk_with_optional_embedding(&conn, "/tmp/c.txt", 0, "pending two", None);

        let pending = pending_embedding_texts(&conn, 10);
        assert_eq!(pending, vec!["pending one", "pending two"]);
    }

    #[test]
    fn pending_embedding_query_respects_limit() {
        let conn = test_db();
        for i in 0..5 {
            insert_chunk_with_optional_embedding(&conn, &format!("/tmp/{i}.txt"), 0, "pending", None);
        }

        let pending = pending_embedding_texts(&conn, 2);
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn pending_embedding_query_is_empty_once_everything_is_embedded() {
        let conn = test_db();
        insert_chunk_with_optional_embedding(&conn, "/tmp/a.txt", 0, "done", Some(&[1.0]));

        let pending = pending_embedding_texts(&conn, 10);
        assert!(pending.is_empty());
    }

    #[test]
    fn upsert_text_replaces_previous_chunks() {
        let conn = test_db();

        conn.execute(
            "INSERT INTO documents (path, modified_ms) VALUES (?1, ?2)",
            params!["/tmp/a.txt", 100],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text) VALUES (?1, 0, 'old')",
            params!["/tmp/a.txt"],
        )
        .unwrap();

        // Simulate what upsert_document_chunks_text does internally, without the State wrapper.
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO documents (path, modified_ms) VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET modified_ms = excluded.modified_ms",
            params!["/tmp/a.txt", 200],
        )
        .unwrap();
        tx.execute("DELETE FROM chunks WHERE path = ?1", params!["/tmp/a.txt"])
            .unwrap();
        tx.execute(
            "INSERT INTO chunks (path, chunk_index, text) VALUES (?1, 0, 'new')",
            params!["/tmp/a.txt"],
        )
        .unwrap();
        tx.commit().unwrap();

        let text: String = conn
            .query_row("SELECT text FROM chunks WHERE path = ?1", params!["/tmp/a.txt"], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(text, "new");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks WHERE path = ?1", params!["/tmp/a.txt"], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);

        // The FTS index should reflect only the replacement, not an orphaned row for the
        // deleted "old" chunk.
        let old_fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'old'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(old_fts_count, 0);
        let new_fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'new'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(new_fts_count, 1);
    }

    #[test]
    fn fts_index_has_no_orphan_after_document_removal() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO documents (path, modified_ms) VALUES (?1, ?2)",
            params!["/tmp/removeme.txt", 100],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text) VALUES (?1, 0, 'searchable content')",
            params!["/tmp/removeme.txt"],
        )
        .unwrap();

        let before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'searchable'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, 1);

        // Exercise the real transactional delete path, not a hand-copied approximation.
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "DELETE FROM chunks WHERE path = ?1",
            params!["/tmp/removeme.txt"],
        )
        .unwrap();
        tx.execute(
            "DELETE FROM documents WHERE path = ?1",
            params!["/tmp/removeme.txt"],
        )
        .unwrap();
        tx.commit().unwrap();

        let after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'searchable'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, 0);
    }

    #[test]
    fn migrate_to_v2_backfills_fts_from_existing_v1_chunks() {
        let conn = Connection::open_in_memory().unwrap();
        // Hand-build a v1 database (nullable embeddings, no FTS) predating this migration.
        conn.execute_batch(
            "CREATE TABLE documents (path TEXT PRIMARY KEY, modified_ms INTEGER NOT NULL);
             CREATE TABLE chunks (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 path TEXT NOT NULL REFERENCES documents(path) ON DELETE CASCADE,
                 chunk_index INTEGER NOT NULL,
                 text TEXT NOT NULL,
                 embedding BLOB,
                 dim INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_chunks_path ON chunks(path);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO documents (path, modified_ms) VALUES (?1, ?2)",
            params!["/tmp/pre-existing.txt", 100],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text) VALUES (?1, 0, 'preexisting searchable text')",
            params!["/tmp/pre-existing.txt"],
        )
        .unwrap();

        init_db(&conn).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'preexisting'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn removing_document_cascades_to_chunks() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO documents (path, modified_ms) VALUES (?1, ?2)",
            params!["/tmp/b.txt", 100],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text, embedding, dim) VALUES (?1, 0, 'x', ?2, 1)",
            params!["/tmp/b.txt", embedding_to_blob(&[1.0])],
        )
        .unwrap();

        conn.execute("DELETE FROM documents WHERE path = ?1", params!["/tmp/b.txt"])
            .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn migrates_pre_v1_schema_preserving_existing_embeddings() {
        let conn = Connection::open_in_memory().unwrap();
        // Recreate the pre-v1 schema by hand (embedding/dim NOT NULL, no user_version).
        conn.execute_batch(
            "CREATE TABLE documents (path TEXT PRIMARY KEY, modified_ms INTEGER NOT NULL);
             CREATE TABLE chunks (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 path TEXT NOT NULL REFERENCES documents(path) ON DELETE CASCADE,
                 chunk_index INTEGER NOT NULL,
                 text TEXT NOT NULL,
                 embedding BLOB NOT NULL,
                 dim INTEGER NOT NULL
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO documents (path, modified_ms) VALUES (?1, ?2)",
            params!["/tmp/old.txt", 100],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text, embedding, dim) VALUES (?1, 0, 'kept', ?2, 2)",
            params!["/tmp/old.txt", embedding_to_blob(&[1.0, 2.0])],
        )
        .unwrap();

        // Now run the real migration path.
        init_db(&conn).unwrap();

        let (text, embedding): (String, Vec<u8>) = conn
            .query_row(
                "SELECT text, embedding FROM chunks WHERE path = ?1",
                params!["/tmp/old.txt"],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(text, "kept");
        assert_eq!(decode_blob(&embedding), vec![1.0, 2.0]);

        // New nullable-embedding inserts should now work against the migrated table.
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text) VALUES (?1, 1, 'new chunk')",
            params!["/tmp/old.txt"],
        )
        .unwrap();
    }
}
