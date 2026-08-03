use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub struct Db(pub Mutex<Connection>);

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

/// Schema is versioned via `PRAGMA user_version` so an existing local database (from
/// before embeddings were nullable) gets migrated in place instead of silently keeping
/// its old, incompatible constraints.
pub fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;

    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        migrate_to_v1(conn)?;
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

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserializes a little-endian f32 BLOB back into a vector, as produced by [`embedding_to_blob`].
pub fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// Embeddings are unit-normalized before storage, so a plain dot product equals cosine similarity.
pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
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

#[tauri::command]
pub fn remove_document(db: tauri::State<Db>, path: String) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM documents WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn blob_roundtrip_preserves_values() {
        let embedding = vec![0.1_f32, -0.5, 3.25, 0.0];
        let blob = embedding_to_blob(&embedding);
        let restored = blob_to_embedding(&blob);
        assert_eq!(embedding, restored);
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
        assert_eq!(blob_to_embedding(&embedding), vec![1.0, 2.0]);
        assert_eq!(dim, 2);
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
    }

    #[test]
    fn dot_product_ranks_closer_vectors_higher() {
        let query = vec![1.0_f32, 0.0];
        let close = vec![0.9_f32, 0.1];
        let far = vec![0.0_f32, 1.0];
        assert!(dot(&query, &close) > dot(&query, &far));
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
        assert_eq!(blob_to_embedding(&embedding), vec![1.0, 2.0]);

        // New nullable-embedding inserts should now work against the migrated table.
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text) VALUES (?1, 1, 'new chunk')",
            params!["/tmp/old.txt"],
        )
        .unwrap();
    }
}
