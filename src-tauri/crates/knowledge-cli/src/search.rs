//! 混合检索：向量余弦召回 + FTS5 关键词召回 → RRF 融合。
//!
//! `model_dir` 为当前向量模型目录（查询向量化用）。整库向量按调用缓存于本进程：
//! 进程短命，缓存随退出释放，不会把内存留在 codex-ui 里。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::embed;
use super::store::{ChunkHit, KbStore};

/// 单路召回的候选条数
pub const RECALL_TOP: usize = 50;
/// 默认返回条数
pub const DEFAULT_TOP_K: usize = 8;
/// RRF 融合常数
const RRF_K: f64 = 60.0;

struct CachedVectors {
    revision: String,
    dim: usize,
    ids: Vec<i64>,
    index_of: HashMap<i64, usize>,
    flat: Vec<f32>,
}

/// 按库缓存的全量向量（revision 变化即失效），避免每次检索重读 BLOB
#[derive(Default)]
pub struct VectorCache {
    entries: Mutex<HashMap<String, Arc<CachedVectors>>>,
}

impl VectorCache {
    fn get(&self, store: &KbStore) -> Result<Arc<CachedVectors>, String> {
        let key = store.path.to_string_lossy().to_string();
        let revision = store.revision();
        if let Ok(map) = self.entries.lock() {
            if let Some(cached) = map.get(&key) {
                if cached.revision == revision {
                    return Ok(cached.clone());
                }
            }
        }
        let (ids, flat, dim) = store.all_embeddings()?;
        let mut index_of = HashMap::with_capacity(ids.len());
        for (i, id) in ids.iter().enumerate() {
            index_of.insert(*id, i);
        }
        let cached = Arc::new(CachedVectors {
            revision,
            dim,
            ids,
            index_of,
            flat,
        });
        if let Ok(mut map) = self.entries.lock() {
            map.insert(key, cached.clone());
        }
        Ok(cached)
    }

    /// 缓存条目数（测试用）
    pub fn len(&self) -> usize {
        self.entries.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn cosine(cached: &CachedVectors, idx: usize, query: &[f32]) -> f32 {
    let dim = cached.dim;
    if dim == 0 || query.len() != dim {
        return 0.0;
    }
    let offset = idx * dim;
    let mut dot = 0.0f32;
    let mut norm = 0.0f32;
    for i in 0..dim {
        let v = cached.flat[offset + i];
        dot += v * query[i];
        norm += v * v;
    }
    if norm <= 0.0 {
        return 0.0;
    }
    dot / norm.sqrt()
}

/// 向量召回：返回按余弦降序的 chunk id
fn vector_candidates(
    model_dir: &Path,
    store: &KbStore,
    cache: &VectorCache,
    query: &str,
    limit: usize,
) -> Result<Vec<i64>, String> {
    let cached = cache.get(store)?;
    if cached.ids.is_empty() {
        return Ok(Vec::new());
    }
    if cached.dim != super::paths::EMBED_DIM {
        return Err(format!(
            "索引向量维度为 {}，与当前模型（{} 维）不一致，请删除该知识库后重新建库",
            cached.dim,
            super::paths::EMBED_DIM
        ));
    }
    let query_vec = embed::embed_query(model_dir, query)?;
    let mut scored: Vec<(i64, f32)> = cached
        .ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, cosine(&cached, i, &query_vec)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    Ok(scored.into_iter().map(|(id, _)| id).collect())
}

/// 构造 FTS5 `MATCH` 表达式：整句短语 + 中文长串的三字滑窗（trigram 分词器下才有召回）
pub fn build_match_expr(query: &str) -> String {
    let mut terms: Vec<String> = Vec::new();
    let trimmed = query.trim();
    if trimmed.chars().count() >= 3 {
        terms.push(format!("\"{}\"", trimmed.replace('"', "\"\"")));
    }
    for token in trimmed.split(|c: char| c.is_whitespace() || "，。！？、；：（）【】《》,.!?;:()[]{}".contains(c))
    {
        let chars: Vec<char> = token.chars().collect();
        if chars.len() < 4 || !chars.iter().any(|c| !c.is_ascii()) {
            continue;
        }
        let mut window = 0usize;
        let mut start = 0usize;
        while start + 3 <= chars.len() && window < 12 {
            let gram: String = chars[start..start + 3].iter().collect();
            terms.push(format!("\"{gram}\""));
            start += 2;
            window += 1;
        }
    }
    terms.join(" OR ")
}

/// RRF 融合：`score = Σ 1/(k + rank)`
pub fn rrf_fuse(vec_ids: &[i64], fts_ids: &[i64], top: usize) -> Vec<(i64, f64)> {
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for (rank, id) in vec_ids.iter().enumerate() {
        *scores.entry(*id).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f64);
    }
    for (rank, id) in fts_ids.iter().enumerate() {
        *scores.entry(*id).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f64);
    }
    let mut fused: Vec<(i64, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused.truncate(top);
    fused
}

/// 混合检索：向量 + 关键词召回后融合，命中的 `score` 用与查询的余弦相似度（0~1，便于展示）
pub fn search(
    model_dir: &Path,
    store: &KbStore,
    cache: &VectorCache,
    query: &str,
    top_k: usize,
) -> Result<Vec<ChunkHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("检索词为空".into());
    }
    let (_, chunk_count) = store.counts();
    if chunk_count == 0 {
        return Ok(Vec::new());
    }
    let top_k = top_k.clamp(1, 50);
    let vec_ids = vector_candidates(model_dir, store, cache, query, RECALL_TOP)?;
    let expr = build_match_expr(query);
    let mut fts_ids: Vec<i64> = if expr.is_empty() {
        Vec::new()
    } else {
        store
            .fts_search(&expr, RECALL_TOP)?
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    };
    if fts_ids.is_empty() {
        fts_ids = store.like_search(query, RECALL_TOP)?;
    }
    let fused = rrf_fuse(&vec_ids, &fts_ids, top_k);
    let ids: Vec<i64> = fused.iter().map(|(id, _)| *id).collect();
    let mut hits = store.chunks_by_ids(&ids)?;

    // 展示分：与查询向量的余弦相似度，缺失时回落到 RRF 分数的放大值
    let cached = cache.get(store)?;
    let query_vec = embed::embed_query(model_dir, query).unwrap_or_default();
    for (i, hit) in hits.iter_mut().enumerate() {
        let id = ids.get(i).copied().unwrap_or_default();
        hit.score = match (cached.index_of.get(&id), query_vec.len() == cached.dim) {
            (Some(idx), true) => cosine(&cached, *idx, &query_vec).clamp(-1.0, 1.0),
            _ => fused
                .get(i)
                .map(|(_, s)| (*s * 10.0) as f32)
                .unwrap_or(0.0),
        };
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_expr_includes_phrase_and_chinese_windows() {
        let expr = build_match_expr("设备无法开机怎么处理");
        assert!(expr.contains("\"设备无法开机怎么处理\""));
        assert!(expr.contains("\"设备无\""));
        assert!(expr.len() > 10);
        assert_eq!(build_match_expr("a"), "");
    }

    #[test]
    fn match_expr_escapes_quotes() {
        let expr = build_match_expr("错误\"码\"测试");
        assert!(expr.contains("\"\""));
    }

    #[test]
    fn rrf_prefers_items_hit_by_both_paths() {
        let fused = rrf_fuse(&[1, 2, 3], &[3, 4], 3);
        assert_eq!(fused[0].0, 3, "两路都命中的排最前");
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn rrf_handles_single_path_results() {
        assert_eq!(rrf_fuse(&[7], &[], 5)[0].0, 7);
        assert_eq!(rrf_fuse(&[], &[9], 5)[0].0, 9);
        assert!(rrf_fuse(&[], &[], 5).is_empty());
    }
}
