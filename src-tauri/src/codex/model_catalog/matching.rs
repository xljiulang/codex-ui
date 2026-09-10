//! 数据源共用的候选匹配：别名键 → 匹配级别 → 确定性排序。
//!
//! 排序规则（无版本 id 优先命中同家族最新正式版）：
//! 1. 匹配级别：别名键精确 → 规范化精确 → 相似度（阈值 0.72）；
//! 2. 同级别按 `is_variant`（变体/别名靠后）→ 多余 token 数（越少越像基础名）
//!    → 发布时间（越新越好）→ 候选 id 字典序，保证结果确定。

use std::cmp::Ordering;
use std::collections::HashSet;

use serde_json::Value;

/// 模糊匹配阈值。
pub const FUZZY_MATCH_THRESHOLD: f64 = 0.72;

/// 视为“变体/别名”的 token（影响排序，不影响匹配级别）。
///
/// 注意 `vision` 不算：它是模型能力而不是版本/实验标记。
const VARIANT_TOKENS: &[&str] = &[
    "exp",
    "experimental",
    "preview",
    "beta",
    "alpha",
    "rc",
    "nightly",
    "dev",
    "free",
    "batch",
    "latest",
];

/// 一条候选：某个数据源里的一个模型。
#[derive(Debug, Clone)]
pub struct Candidate {
    keys: Vec<String>,
    normalized_keys: Vec<String>,
    is_variant: bool,
    release: Option<i64>,
    sort_id: String,
}

impl Candidate {
    /// `keys` 为别名键（如 OpenRouter 的 `id` / `canonical_slug`、models.dev 的 `id` / `family`）。
    pub fn new(keys: Vec<String>, release: Option<i64>, sort_id: impl Into<String>) -> Self {
        let normalized_keys: Vec<String> = keys
            .iter()
            .map(|key| normalize_model_key(key))
            .filter(|key| !key.is_empty())
            .collect();
        let is_variant = keys.iter().any(|key| has_variant_token(key));
        Self {
            keys,
            normalized_keys,
            is_variant,
            release,
            sort_id: sort_id.into(),
        }
    }

}

/// 候选集合：内部保持候选顺序，`lookup` 返回命中的原始载荷。
#[derive(Default)]
pub struct CandidateStore {
    candidates: Vec<Candidate>,
    payloads: Vec<Value>,
}

impl CandidateStore {
    pub fn new(items: Vec<(Candidate, Value)>) -> Self {
        let (candidates, payloads) = items.into_iter().unzip();
        Self {
            candidates,
            payloads,
        }
    }

    /// 候选数量（供测试断言内置快照规模）。
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    /// 命中返回原始载荷；未命中或相似度不足返回 `None`。
    pub fn lookup(&self, model_id: &str) -> Option<&Value> {
        let index = lookup_index(&self.candidates, model_id)?;
        self.payloads.get(index)
    }
}

fn lookup_index(candidates: &[Candidate], model_id: &str) -> Option<usize> {
    let query = model_id.trim();
    if query.is_empty() {
        return None;
    }
    let query_key = normalize_model_key(query);
    let query_tokens: HashSet<String> = tokenize(&query_key).into_iter().collect();

    let mut best: Option<(Rank, usize)> = None;
    for (index, candidate) in candidates.iter().enumerate() {
        let Some(rank) = rank_candidate(candidate, query, &query_key, &query_tokens) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((best_rank, _)) => rank < *best_rank,
        };
        if better {
            best = Some((rank, index));
        }
    }
    best.map(|(_, index)| index)
}

/// 排序键：越小越优。
#[derive(Debug, PartialEq, Eq)]
struct Rank {
    level: u8,
    score_millis: i64,
    is_variant: bool,
    extra_tokens: usize,
    release: i64,
    sort_id: String,
}

impl Ord for Rank {
    fn cmp(&self, other: &Self) -> Ordering {
        self.level
            .cmp(&other.level)
            .then_with(|| other.score_millis.cmp(&self.score_millis))
            .then_with(|| self.is_variant.cmp(&other.is_variant))
            .then_with(|| self.extra_tokens.cmp(&other.extra_tokens))
            .then_with(|| other.release.cmp(&self.release))
            .then_with(|| self.sort_id.cmp(&other.sort_id))
    }
}

impl PartialOrd for Rank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn rank_candidate(
    candidate: &Candidate,
    query: &str,
    query_key: &str,
    query_tokens: &HashSet<String>,
) -> Option<Rank> {
    let mut level = None;
    let mut score = 1.0_f64;

    if candidate.keys.iter().any(|key| key == query) {
        level = Some(0_u8);
    } else if !query_key.is_empty()
        && candidate
            .normalized_keys
            .iter()
            .any(|key| key == query_key)
    {
        level = Some(1_u8);
    } else if !query_key.is_empty() {
        let best = candidate
            .normalized_keys
            .iter()
            .map(|key| similarity(query_key, key))
            .fold(0.0_f64, f64::max);
        if best >= FUZZY_MATCH_THRESHOLD {
            level = Some(2_u8);
            score = best;
        }
    }
    let level = level?;

    let extra_tokens = candidate
        .normalized_keys
        .iter()
        .map(|key| {
            tokenize(key)
                .into_iter()
                .filter(|token| !query_tokens.contains(token))
                .count()
        })
        .min()
        .unwrap_or(0);

    Some(Rank {
        level,
        score_millis: (score * 1000.0).round() as i64,
        is_variant: candidate.is_variant,
        extra_tokens,
        release: candidate.release.unwrap_or(0),
        sort_id: candidate.sort_id.clone(),
    })
}

/// 规范化匹配键：去 provider 前缀/别名后缀/结尾日期，统一小写与分隔符。
pub fn normalize_model_key(input: &str) -> String {
    let mut key = input
        .trim()
        .to_ascii_lowercase()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    loop {
        let before = key.clone();
        for suffix in [
            ":free",
            ":batch",
            ":extended",
            "-free",
            "-latest",
            "-preview",
            "-beta",
            "-alpha",
        ] {
            if let Some(stripped) = key.strip_suffix(suffix) {
                key = stripped.to_string();
            }
        }
        if key == before {
            break;
        }
    }
    key = strip_trailing_date(&key).to_string();
    key.replace('_', "-").trim_matches('-').to_string()
}

fn strip_trailing_date(key: &str) -> &str {
    if let Some((prefix, last)) = key.rsplit_once('-') {
        if last.len() == 8 && last.bytes().all(|b| b.is_ascii_digit()) {
            return prefix;
        }
    }
    if key.len() >= 11 {
        let start = key.len() - 11;
        let candidate = &key[start..];
        if candidate.starts_with('-')
            && candidate[1..]
                .bytes()
                .filter(|b| *b != b'-')
                .all(|b| b.is_ascii_digit())
            && candidate[1..].bytes().filter(|b| *b == b'-').count() == 2
        {
            return &key[..start];
        }
    }
    key
}

fn tokenize(input: &str) -> Vec<String> {
    input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn has_variant_token(input: &str) -> bool {
    tokenize(input)
        .iter()
        .any(|token| VARIANT_TOKENS.contains(&token.as_str()))
}

fn similarity(left: &str, right: &str) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    let left_chars: Vec<char> = left.chars().collect();
    let right_chars: Vec<char> = right.chars().collect();
    let distance = levenshtein(&left_chars, &right_chars) as f64;
    let max_len = left_chars.len().max(right_chars.len()) as f64;
    let edit_score = 1.0 - distance / max_len;
    edit_score.max(token_overlap(left, right))
}

fn levenshtein(left: &[char], right: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0usize; right.len() + 1];
    for (i, left_char) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, right_char) in right.iter().enumerate() {
            let cost = usize::from(left_char != right_char);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn token_overlap(left: &str, right: &str) -> f64 {
    let left_tokens: HashSet<&str> = left
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    let right_tokens: HashSet<&str> = right
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    let union = left_tokens.union(&right_tokens).count();
    if union == 0 {
        return 0.0;
    }
    left_tokens.intersection(&right_tokens).count() as f64 / union as f64
}

/// `YYYY-MM-DD` → epoch 秒（跨源统一的发布时间排序键）。
pub fn release_from_date(date: &str) -> Option<i64> {
    let mut parts = date.trim().split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400)
}

/// Howard Hinnant 的 civil → days 算法（避免引入时间库）。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store(items: &[(&str, Option<i64>)]) -> CandidateStore {
        CandidateStore::new(
            items
                .iter()
                .map(|(id, release)| {
                    (
                        Candidate::new(vec![(*id).to_string()], *release, *id),
                        json!({ "id": id }),
                    )
                })
                .collect(),
        )
    }

    fn matched_id<'a>(store: &'a CandidateStore, query: &str) -> Option<&'a str> {
        store
            .lookup(query)
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
    }

    #[test]
    fn normalize_model_key_removes_prefix_suffix_and_date() {
        assert_eq!(normalize_model_key("xiaomi/mimo-v2.5-free"), "mimo-v2.5");
        assert_eq!(
            normalize_model_key("xiaomi/mimo-v2.5-20260422"),
            "mimo-v2.5"
        );
        assert_eq!(
            normalize_model_key("google/gemini-3.8-flash:batch"),
            "gemini-3.8-flash"
        );
        assert_eq!(
            normalize_model_key("~deepseek/deepseek-v4-flash-latest"),
            "deepseek-v4-flash"
        );
    }

    #[test]
    fn lookup_prefers_exact_then_normalized_key() {
        let store = store(&[("xiaomi/mimo-v2.5-pro", None), ("xiaomi/mimo-v2.5", None)]);
        assert_eq!(
            matched_id(&store, "xiaomi/mimo-v2.5"),
            Some("xiaomi/mimo-v2.5")
        );
        assert_eq!(
            matched_id(&store, "mimo-v2.5-free"),
            Some("xiaomi/mimo-v2.5")
        );
    }

    #[test]
    fn lookup_skips_variant_for_versionless_query() {
        let store = store(&[
            ("deepseek/deepseek-v4-flash-vision-exp", Some(1_800_000_000)),
            ("deepseek/deepseek-v4-flash", Some(1_700_000_000)),
        ]);
        assert_eq!(
            matched_id(&store, "deepseek-flash"),
            Some("deepseek/deepseek-v4-flash")
        );
    }

    #[test]
    fn lookup_prefers_newest_release_between_same_shape_versions() {
        let store = store(&[
            ("deepseek/deepseek-v4-flash", Some(1_700_000_000)),
            ("deepseek/deepseek-v5-flash", Some(1_800_000_000)),
        ]);
        assert_eq!(
            matched_id(&store, "deepseek-flash"),
            Some("deepseek/deepseek-v5-flash")
        );
    }

    #[test]
    fn lookup_keeps_versioned_query_on_itself() {
        let store = store(&[
            ("deepseek/deepseek-v4-flash", Some(1_700_000_000)),
            ("deepseek/deepseek-v4-flash-0731", Some(1_750_000_000)),
        ]);
        assert_eq!(
            matched_id(&store, "deepseek-v4-flash-0731"),
            Some("deepseek/deepseek-v4-flash-0731")
        );
    }

    #[test]
    fn family_key_matches_versionless_query_exactly() {
        let store = CandidateStore::new(vec![
            (
                Candidate::new(
                    vec!["deepseek-v4-flash".to_string(), "deepseek-flash".to_string()],
                    Some(1_700_000_000),
                    "deepseek-v4-flash",
                ),
                json!({ "id": "deepseek-v4-flash" }),
            ),
            (
                Candidate::new(
                    vec![
                        "deepseek-v4-flash-vision-exp".to_string(),
                        "deepseek-flash".to_string(),
                    ],
                    Some(1_800_000_000),
                    "deepseek-v4-flash-vision-exp",
                ),
                json!({ "id": "deepseek-v4-flash-vision-exp" }),
            ),
        ]);
        assert_eq!(
            matched_id(&store, "deepseek-flash"),
            Some("deepseek-v4-flash")
        );
    }

    #[test]
    fn lookup_rejects_unrelated_ids() {
        let store = store(&[("deepseek/deepseek-v4-flash", None)]);
        assert!(store.lookup("unknown-model").is_none());
    }

    #[test]
    fn release_from_date_converts_iso_dates() {
        assert_eq!(release_from_date("1970-01-01"), Some(0));
        assert_eq!(release_from_date("2026-07-31"), Some(1_785_456_000));
        assert_eq!(release_from_date("not-a-date"), None);
    }
}
