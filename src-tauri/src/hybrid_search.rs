//! Keyword ranking runs against the `chunks_fts` FTS5 index (see `store::migrate_to_v2`)
//! instead of re-tokenizing the whole corpus per query; semantic ranking streams the
//! embedded chunk table in bounded batches, scored in parallel via rayon, merged into a
//! fixed-size top-k heap. Neither pass loads chunk text - only the final, already-fused
//! and truncated id list gets hydrated with text at the end.

use crate::store::{ReadDb, SearchResult};
use rayon::prelude::*;
use rusqlite::{params, Connection};
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};

/// Standard Reciprocal Rank Fusion damping constant (as used by Elasticsearch/TREC).
const RRF_K: f32 = 60.0;
/// Each ranking is capped to this many candidates before fusion - anything ranked below
/// this contributes less than 1 / (60 + 201) ≈ 0.004 to an RRF score, so the cap costs
/// negligible recall while bounding how much work every query does regardless of corpus
/// size.
const CANDIDATE_LIMIT: i64 = 200;
/// Semantic scoring streams through the embedded chunk table this many rows at a time,
/// rather than loading every embedding into memory at once (hundreds of megabytes on a
/// large index).
const SEMANTIC_BATCH_SIZE: i64 = 8192;

struct ChunkRow {
    path: String,
    chunk_index: i64,
    text: String,
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(str::to_string)
        .collect()
}

/// Builds an FTS5 MATCH expression from already-tokenized query terms. Each token is
/// quoted so FTS5's own operator syntax (AND/OR/NEAR/*/^/:) can never reach the parser -
/// `tokenize` already strips everything but alphanumerics, so in practice there is
/// nothing to escape, but quoting is cheap insurance against that invariant changing.
/// Tokens are joined with OR (not FTS5's default AND) to match the previous hand-rolled
/// BM25's any-term-overlap behavior. Only the last token gets a prefix operator, since in
/// a live, debounced search box it's usually still being typed - earlier tokens stay
/// exact so short words don't fan out into unrelated matches.
fn build_match_expr(query_tokens: &[String]) -> Option<String> {
    if query_tokens.is_empty() {
        return None;
    }
    let last = query_tokens.len() - 1;
    let quoted: Vec<String> = query_tokens
        .iter()
        .enumerate()
        .map(|(i, token)| {
            let escaped = token.replace('"', "\"\"");
            if i == last {
                format!("\"{escaped}\"*")
            } else {
                format!("\"{escaped}\"")
            }
        })
        .collect();
    Some(quoted.join(" OR "))
}

/// Ranks chunk ids by BM25 against `chunks_fts`, best first, via the inverted index
/// SQLite maintains incrementally as chunks are written - no re-tokenization of the
/// corpus per query. FTS5's `bm25()` returns more-negative-is-better, so the default
/// ascending sort is already best-first.
fn keyword_ranking(conn: &Connection, query_tokens: &[String], limit: i64) -> rusqlite::Result<Vec<i64>> {
    let Some(match_expr) = build_match_expr(query_tokens) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(
        "SELECT rowid FROM chunks_fts WHERE chunks_fts MATCH ?1 ORDER BY bm25(chunks_fts) LIMIT ?2",
    )?;
    let ids = stmt
        .query_map(params![match_expr, limit], |r| r.get(0))?
        .collect();
    ids
}

/// A chunk id paired with a semantic similarity score, ordered purely by score so it can
/// sit in a [`BinaryHeap`] for bounded top-k selection.
struct ScoredId(i64, f32);

impl PartialEq for ScoredId {
    fn eq(&self, other: &Self) -> bool {
        self.1 == other.1
    }
}
impl Eq for ScoredId {}
impl PartialOrd for ScoredId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ScoredId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.1.total_cmp(&other.1)
    }
}

/// Embeddings are unit-normalized before storage, so a plain dot product equals cosine
/// similarity. Computed directly off the little-endian f32 bytes rather than first
/// deserializing into a `Vec<f32>`, since this runs once per chunk per query and the
/// intermediate allocation would dominate at scale.
fn dot_bytes(query: &[f32], blob: &[u8]) -> f32 {
    blob.chunks_exact(4)
        .zip(query)
        .map(|(b, q)| f32::from_le_bytes([b[0], b[1], b[2], b[3]]) * *q)
        .sum()
}

/// Ranks chunk ids by semantic similarity to the query embedding, best first. Streams
/// through every embedded chunk in [`SEMANTIC_BATCH_SIZE`]-sized batches via keyset
/// pagination on `id`, scoring each batch in parallel across cores via rayon and merging
/// into a size-`limit` min-heap, so memory use and per-query work stay bounded regardless
/// of corpus size. Chunks with no embedding yet (still in Phase 1 of indexing) are
/// excluded by the query itself rather than penalized with a fabricated low score.
fn semantic_top_k(conn: &Connection, query_embedding: &[f32], limit: usize) -> rusqlite::Result<Vec<i64>> {
    let mut heap: BinaryHeap<Reverse<ScoredId>> = BinaryHeap::with_capacity(limit + 1);
    let mut last_id = 0i64;

    loop {
        let mut stmt = conn.prepare_cached(
            "SELECT id, embedding FROM chunks
             WHERE embedding IS NOT NULL AND id > ?1
             ORDER BY id LIMIT ?2",
        )?;
        let batch: Vec<(i64, Vec<u8>)> = stmt
            .query_map(params![last_id, SEMANTIC_BATCH_SIZE], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_, _>>()?;

        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        last_id = batch[batch_len - 1].0;

        let scored: Vec<ScoredId> = batch
            .par_iter()
            .map(|(id, blob)| ScoredId(*id, dot_bytes(query_embedding, blob)))
            .collect();

        for s in scored {
            heap.push(Reverse(s));
            if heap.len() > limit {
                heap.pop();
            }
        }

        if batch_len < SEMANTIC_BATCH_SIZE as usize {
            break;
        }
    }

    let mut top: Vec<ScoredId> = heap.into_iter().map(|Reverse(s)| s).collect();
    top.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(top.into_iter().map(|s| s.0).collect())
}

/// Combines multiple rankings of the same items into one score via Reciprocal Rank
/// Fusion, so BM25's unbounded scores and cosine similarity's [-1, 1] range never need
/// to be normalized onto a common scale - only each ranking's relative order matters.
fn reciprocal_rank_fusion(rankings: &[Vec<i64>]) -> HashMap<i64, f32> {
    let mut fused: HashMap<i64, f32> = HashMap::new();
    for ranking in rankings {
        for (rank, id) in ranking.iter().enumerate() {
            *fused.entry(*id).or_insert(0.0) += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    fused
}

/// Fetches path/chunk_index/text for exactly the given ids. Called only after fusion and
/// truncation to the final result count, so this touches a handful of rows rather than
/// the whole corpus - unlike the old implementation, which loaded every chunk's text
/// unconditionally on every query.
fn hydrate(conn: &Connection, ids: &[i64]) -> rusqlite::Result<HashMap<i64, ChunkRow>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let sql = format!("SELECT id, path, chunk_index, text FROM chunks WHERE id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let bind_params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    let rows = stmt
        .query_map(bind_params.as_slice(), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                ChunkRow {
                    path: r.get(1)?,
                    chunk_index: r.get(2)?,
                    text: r.get(3)?,
                },
            ))
        })?
        .collect();
    rows
}

#[tauri::command]
pub fn hybrid_search_cmd(
    db: tauri::State<ReadDb>,
    query: String,
    query_embedding: Vec<f32>,
    limit: i64,
) -> Result<Vec<SearchResult>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    let query_tokens = tokenize(&query);
    let keyword_ids =
        keyword_ranking(&conn, &query_tokens, CANDIDATE_LIMIT).map_err(|e| e.to_string())?;
    let semantic_ids = semantic_top_k(&conn, &query_embedding, CANDIDATE_LIMIT as usize)
        .map_err(|e| e.to_string())?;

    let fused = reciprocal_rank_fusion(&[semantic_ids, keyword_ids]);
    let mut fused_vec: Vec<(i64, f32)> = fused.into_iter().collect();
    fused_vec.sort_by(|a, b| b.1.total_cmp(&a.1));
    fused_vec.truncate(limit.max(0) as usize);

    let ids: Vec<i64> = fused_vec.iter().map(|(id, _)| *id).collect();
    let rows = hydrate(&conn, &ids).map_err(|e| e.to_string())?;

    Ok(fused_vec
        .into_iter()
        .filter_map(|(id, score)| {
            rows.get(&id).map(|c| SearchResult {
                path: c.path.clone(),
                chunk_index: c.chunk_index,
                text: c.text.clone(),
                score,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{embedding_to_blob, init_db};

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    /// Inserts a document + single chunk, returning the chunk's id. Goes through raw SQL
    /// (like the store.rs tests) rather than the `#[tauri::command]` functions, so the
    /// FTS triggers created by `migrate_to_v2` are exercised exactly as they'd fire in
    /// production.
    fn insert_chunk(conn: &Connection, path: &str, text: &str, embedding: Option<&[f32]>) -> i64 {
        conn.execute(
            "INSERT INTO documents (path, modified_ms) VALUES (?1, 0)
             ON CONFLICT(path) DO NOTHING",
            params![path],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (path, chunk_index, text) VALUES (?1, 0, ?2)",
            params![path, text],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        if let Some(e) = embedding {
            conn.execute(
                "UPDATE chunks SET embedding = ?1, dim = ?2 WHERE id = ?3",
                params![embedding_to_blob(e), e.len() as i64, id],
            )
            .unwrap();
        }
        id
    }

    #[test]
    fn keyword_ranking_only_prefixes_the_final_token() {
        let conn = test_db();
        let budgeting = insert_chunk(&conn, "/tmp/a.txt", "the budgeting spreadsheet for this year", None);
        let quarterly_only = insert_chunk(&conn, "/tmp/b.txt", "quarterly report", None);
        insert_chunk(&conn, "/tmp/c.txt", "completely unrelated gardening content", None);

        // "quarterly budg" -> "quarterly" OR "budg"* - both non-prefix "quarterly" and
        // prefix "budg"* should match, gardening should not.
        let tokens = tokenize("quarterly budg");
        let ranked = keyword_ranking(&conn, &tokens, 10).unwrap();
        let ranked: std::collections::HashSet<_> = ranked.into_iter().collect();
        assert!(ranked.contains(&budgeting));
        assert!(ranked.contains(&quarterly_only));
        assert_eq!(ranked.len(), 2);

        // "qua budg" -> "qua" OR "budg"* - if the first token were also prefixed, "qua"
        // would spuriously match "quarterly report" via prefix. It must not.
        let tokens = tokenize("qua budg");
        let ranked = keyword_ranking(&conn, &tokens, 10).unwrap();
        assert_eq!(ranked, vec![budgeting]);
    }

    #[test]
    fn keyword_ranking_handles_query_syntax_characters_without_erroring() {
        let conn = test_db();
        insert_chunk(&conn, "/tmp/a.txt", "some normal text content", None);

        let tokens = tokenize("foo AND \"bar OR (baz");
        let result = keyword_ranking(&conn, &tokens, 10);
        assert!(result.is_ok());
    }

    #[test]
    fn semantic_top_k_skips_chunks_without_an_embedding_yet() {
        let conn = test_db();
        let a = insert_chunk(&conn, "/tmp/a.txt", "text a", Some(&[1.0, 0.0]));
        insert_chunk(&conn, "/tmp/b.txt", "not embedded yet", None);
        let c = insert_chunk(&conn, "/tmp/c.txt", "text c", Some(&[0.9, 0.1]));

        let ranking = semantic_top_k(&conn, &[1.0, 0.0], 10).unwrap();
        assert_eq!(ranking, vec![a, c]);
    }

    #[test]
    fn semantic_top_k_respects_the_limit() {
        let conn = test_db();
        let close = insert_chunk(&conn, "/tmp/a.txt", "text a", Some(&[1.0, 0.0]));
        insert_chunk(&conn, "/tmp/b.txt", "text b", Some(&[0.0, 1.0]));
        insert_chunk(&conn, "/tmp/c.txt", "text c", Some(&[-1.0, 0.0]));

        let ranking = semantic_top_k(&conn, &[1.0, 0.0], 1).unwrap();
        assert_eq!(ranking, vec![close]);
    }

    #[test]
    fn rrf_favors_items_ranked_highly_in_multiple_lists() {
        let rankings = vec![vec![1, 2, 3], vec![2, 1, 3]];
        let fused = reciprocal_rank_fusion(&rankings);
        // id 1 and 2 are each other's #1/#2 across the two rankings, both beating id 3
        // which is last in both.
        assert!(fused[&1] > fused[&3]);
        assert!(fused[&2] > fused[&3]);
    }

    #[test]
    fn rrf_includes_items_present_in_only_one_ranking() {
        let rankings = vec![vec![1, 2], vec![2]];
        let fused = reciprocal_rank_fusion(&rankings);
        assert!(fused.contains_key(&1));
        assert!(fused[&2] > fused[&1]);
    }
}
