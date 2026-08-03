use crate::store::{blob_to_embedding, dot, Db, SearchResult};
use std::collections::HashMap;

const BM25_K1: f32 = 1.5;
const BM25_B: f32 = 0.75;
/// Standard Reciprocal Rank Fusion damping constant (as used by Elasticsearch/TREC).
const RRF_K: f32 = 60.0;

struct ChunkRow {
    id: i64,
    path: String,
    chunk_index: i64,
    text: String,
    /// `None` until the background embedding step catches up - still keyword-searchable
    /// via BM25 in the meantime, just absent from the semantic ranking.
    embedding: Option<Vec<f32>>,
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(str::to_string)
        .collect()
}

/// Ranks chunk ids by semantic similarity to the query embedding, best first. Chunks
/// whose embedding hasn't been computed yet are excluded (they're only reachable via
/// BM25 until then) rather than penalized with a fabricated low score.
fn semantic_ranking(chunks: &[ChunkRow], query_embedding: &[f32]) -> Vec<i64> {
    let mut scored: Vec<(i64, f32)> = chunks
        .iter()
        .filter_map(|c| c.embedding.as_ref().map(|e| (c.id, dot(query_embedding, e))))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.into_iter().map(|(id, _)| id).collect()
}

/// Ranks chunk ids by BM25 score against the tokenized query, best first. Chunks that
/// share no term with the query are excluded rather than ranked with a score of zero.
fn bm25_ranking(chunks: &[ChunkRow], query_tokens: &[String]) -> Vec<i64> {
    if chunks.is_empty() || query_tokens.is_empty() {
        return Vec::new();
    }

    let term_counts: Vec<HashMap<String, u32>> = chunks
        .iter()
        .map(|c| {
            let mut counts = HashMap::new();
            for token in tokenize(&c.text) {
                *counts.entry(token).or_insert(0) += 1;
            }
            counts
        })
        .collect();

    let doc_lens: Vec<f32> = term_counts
        .iter()
        .map(|m| m.values().sum::<u32>() as f32)
        .collect();
    let avg_dl = doc_lens.iter().sum::<f32>() / doc_lens.len() as f32;
    let n = chunks.len() as f32;

    let mut idf: HashMap<&str, f32> = HashMap::new();
    for token in query_tokens {
        let df = term_counts.iter().filter(|m| m.contains_key(token)).count() as f32;
        idf.insert(token.as_str(), ((n - df + 0.5) / (df + 0.5) + 1.0).ln());
    }

    let mut scored: Vec<(i64, f32)> = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let doc_len = doc_lens[i];
        let mut score = 0.0f32;
        for token in query_tokens {
            let tf = *term_counts[i].get(token).unwrap_or(&0) as f32;
            if tf == 0.0 {
                continue;
            }
            score += idf[token.as_str()] * (tf * (BM25_K1 + 1.0))
                / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * doc_len / avg_dl));
        }
        if score > 0.0 {
            scored.push((chunk.id, score));
        }
    }

    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.into_iter().map(|(id, _)| id).collect()
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

#[tauri::command]
pub fn hybrid_search_cmd(
    db: tauri::State<Db>,
    query: String,
    query_embedding: Vec<f32>,
    limit: i64,
) -> Result<Vec<SearchResult>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, path, chunk_index, text, embedding FROM chunks")
        .map_err(|e| e.to_string())?;

    let chunks: Vec<ChunkRow> = stmt
        .query_map([], |r| {
            let blob: Option<Vec<u8>> = r.get(4)?;
            Ok(ChunkRow {
                id: r.get(0)?,
                path: r.get(1)?,
                chunk_index: r.get(2)?,
                text: r.get(3)?,
                embedding: blob.map(|b| blob_to_embedding(&b)),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let query_tokens = tokenize(&query);
    let rankings = vec![
        semantic_ranking(&chunks, &query_embedding),
        bm25_ranking(&chunks, &query_tokens),
    ];
    let fused = reciprocal_rank_fusion(&rankings);

    let mut fused_vec: Vec<(i64, f32)> = fused.into_iter().collect();
    fused_vec.sort_by(|a, b| b.1.total_cmp(&a.1));
    fused_vec.truncate(limit.max(0) as usize);

    let by_id: HashMap<i64, &ChunkRow> = chunks.iter().map(|c| (c.id, c)).collect();
    Ok(fused_vec
        .into_iter()
        .filter_map(|(id, score)| {
            by_id.get(&id).map(|c| SearchResult {
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

    fn chunk(id: i64, text: &str) -> ChunkRow {
        ChunkRow {
            id,
            path: format!("/tmp/{id}.txt"),
            chunk_index: 0,
            text: text.to_string(),
            embedding: None,
        }
    }

    fn chunk_with_embedding(id: i64, embedding: Vec<f32>) -> ChunkRow {
        ChunkRow {
            id,
            path: format!("/tmp/{id}.txt"),
            chunk_index: 0,
            text: String::new(),
            embedding: Some(embedding),
        }
    }

    #[test]
    fn bm25_ranks_exact_term_match_first() {
        let chunks = vec![
            chunk(1, "the quick brown fox jumps over the lazy dog"),
            chunk(2, "error code E1234 occurred during startup"),
            chunk(3, "completely unrelated content about gardening"),
        ];
        let ranking = bm25_ranking(&chunks, &tokenize("E1234"));
        assert_eq!(ranking.first(), Some(&2));
    }

    #[test]
    fn bm25_excludes_chunks_with_no_term_overlap() {
        let chunks = vec![chunk(1, "hello world"), chunk(2, "goodbye moon")];
        let ranking = bm25_ranking(&chunks, &tokenize("world"));
        assert_eq!(ranking, vec![1]);
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

    #[test]
    fn semantic_ranking_skips_chunks_without_an_embedding_yet() {
        let chunks = vec![
            chunk_with_embedding(1, vec![1.0, 0.0]),
            chunk(2, "not embedded yet"), // still text-only, embedding is None
            chunk_with_embedding(3, vec![0.9, 0.1]),
        ];
        let ranking = semantic_ranking(&chunks, &[1.0, 0.0]);
        assert_eq!(ranking, vec![1, 3]);
    }
}
