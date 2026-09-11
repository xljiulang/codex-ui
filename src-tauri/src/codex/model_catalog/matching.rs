//! 数据源共用的候选匹配：主 ID / 权威别名 → 系列感知模糊匹配 → 确定性排序。
//!
//! 排序规则（无版本 id 优先命中同家族最新正式版）：
//! 1. 匹配级别：主 ID 精确 → 别名精确 → 规范化精确 → 相似度；
//! 2. 模糊匹配优先保持厂商、版本系列与 coder/flash/pro 等规格 token；
//! 3. 再按相似度、变体、多余 token、发布时间与 id 排序。

use std::cmp::Ordering;
use std::collections::HashSet;

use serde_json::Value;

/// 模糊匹配阈值。
pub const FUZZY_MATCH_THRESHOLD: f64 = 0.72;
/// 已确认版本系列一致时，允许因 `coder` / `flash` / `pro` 等规格后缀造成的额外距离。
const SAME_SERIES_FUZZY_THRESHOLD: f64 = 0.60;

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

/// 会影响模型用途/规格的 token，模糊匹配时尽量保留。
const ROLE_TOKENS: &[&str] = &[
    "coder", "code", "chat", "reasoner", "reasoning", "flash", "pro", "mini", "nano",
    "sonnet", "opus", "haiku", "vision", "vl", "instruct", "thinking", "lite",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Exact,
    Alias,
    Normalized,
    Fuzzy,
}

impl MatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Alias => "alias",
            Self::Normalized => "normalized",
            Self::Fuzzy => "fuzzy",
        }
    }
}

pub struct CandidateMatch<'a> {
    pub value: &'a Value,
    pub matched_id: &'a str,
    pub kind: MatchKind,
    pub score: f64,
}

/// 一条候选：某个数据源里的一个模型。
#[derive(Debug, Clone)]
pub struct Candidate {
    primary_key: String,
    alias_keys: Vec<String>,
    normalized_primary: String,
    normalized_aliases: Vec<String>,
    is_variant: bool,
    release: Option<i64>,
    sort_id: String,
    vendor: Option<String>,
    series_markers: HashSet<String>,
    role_tokens: HashSet<String>,
}

impl Candidate {
    /// `primary_key` 为数据源主 ID，`alias_keys` 为 family/canonical_slug 等权威别名。
    pub fn new(
        primary_key: String,
        alias_keys: Vec<String>,
        release: Option<i64>,
        sort_id: impl Into<String>,
    ) -> Self {
        let normalized_primary = normalize_model_key(&primary_key);
        let normalized_aliases = alias_keys
            .iter()
            .map(|key| normalize_model_key(key))
            .filter(|key| !key.is_empty())
            .collect();
        let is_variant = std::iter::once(&primary_key)
            .chain(alias_keys.iter())
            .any(|key| has_variant_token(key));
        let vendor = provider_prefix(&primary_key);
        let analysis_key = if normalized_primary.is_empty() {
            primary_key.clone()
        } else {
            normalized_primary.clone()
        };
        Self {
            primary_key,
            alias_keys,
            normalized_primary,
            normalized_aliases,
            is_variant,
            release,
            sort_id: sort_id.into(),
            vendor,
            series_markers: version_series_markers(&analysis_key),
            role_tokens: role_tokens(&analysis_key),
        }
    }

    /// 上游状态也参与正式/实验排序；精确命中仍保持最高优先级。
    pub fn with_status(mut self, status: Option<&str>) -> Self {
        if status.is_some_and(|status| {
            matches!(
                status.trim().to_ascii_lowercase().as_str(),
                "experimental" | "preview" | "beta" | "alpha" | "deprecated"
            )
        }) {
            self.is_variant = true;
        }
        self
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

    /// 命中返回载荷与匹配证据；未命中或相似度不足返回 `None`。
    pub fn lookup(&self, model_id: &str) -> Option<CandidateMatch<'_>> {
        let (index, kind, score) = lookup_index(&self.candidates, model_id)?;
        let candidate = self.candidates.get(index)?;
        Some(CandidateMatch {
            value: self.payloads.get(index)?,
            matched_id: &candidate.sort_id,
            kind,
            score,
        })
    }
}

fn lookup_index(candidates: &[Candidate], model_id: &str) -> Option<(usize, MatchKind, f64)> {
    let query = model_id.trim();
    if query.is_empty() {
        return None;
    }
    let query_key = normalize_model_key(query);
    let query_token_count = tokenize(&query_key).len();

    let query_vendor = provider_prefix(query);
    let query_series = version_series_markers(&query_key);
    let query_roles = role_tokens(&query_key);

    let mut best: Option<(Rank, usize, MatchKind, f64)> = None;
    for (index, candidate) in candidates.iter().enumerate() {
        let Some((rank, kind, score)) = rank_candidate(
            candidate,
            query,
            &query_key,
            query_token_count,
            query_vendor.as_deref(),
            &query_series,
            &query_roles,
        ) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((best_rank, _, _, _)) => rank < *best_rank,
        };
        if better {
            best = Some((rank, index, kind, score));
        }
    }
    best.map(|(_, index, kind, score)| (index, kind, score))
}

/// 排序键：越小越优。
#[derive(Debug, PartialEq, Eq)]
struct Rank {
    level: u8,
    vendor_mismatch: bool,
    series_mismatch: bool,
    missing_roles: usize,
    extra_roles: usize,
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
            .then_with(|| self.vendor_mismatch.cmp(&other.vendor_mismatch))
            .then_with(|| self.series_mismatch.cmp(&other.series_mismatch))
            .then_with(|| self.missing_roles.cmp(&other.missing_roles))
            .then_with(|| self.extra_roles.cmp(&other.extra_roles))
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
    query_token_count: usize,
    query_vendor: Option<&str>,
    query_series: &HashSet<String>,
    query_roles: &HashSet<String>,
) -> Option<(Rank, MatchKind, f64)> {
    let mut kind = None;
    let mut score = 1.0_f64;

    if candidate.primary_key == query {
        kind = Some(MatchKind::Exact);
    } else if candidate.alias_keys.iter().any(|key| key == query) {
        kind = Some(MatchKind::Alias);
    } else if !query_key.is_empty()
        && (candidate.normalized_primary == query_key
            || candidate
                .normalized_aliases
                .iter()
                .any(|key| key == query_key))
    {
        kind = Some(MatchKind::Normalized);
    } else if !query_key.is_empty() {
        let best = std::iter::once(&candidate.normalized_primary)
            .chain(candidate.normalized_aliases.iter())
            .map(|key| similarity(query_key, key))
            .fold(0.0_f64, f64::max);
        let same_series = !query_series.is_empty()
            && !candidate.series_markers.is_empty()
            && !query_series.is_disjoint(&candidate.series_markers);
        let threshold = if same_series {
            SAME_SERIES_FUZZY_THRESHOLD
        } else {
            FUZZY_MATCH_THRESHOLD
        };
        if best >= threshold {
            kind = Some(MatchKind::Fuzzy);
            score = best;
        }
    }
    let kind = kind?;
    let level = match kind {
        MatchKind::Exact => 0,
        MatchKind::Alias => 1,
        MatchKind::Normalized => 2,
        MatchKind::Fuzzy => 3,
    };

    // family 等短别名只负责建立权威别名关系，不能让旧模型因别名更短而压过
    // 同系列更新版本；模糊排序始终以规范模型 ID 的形态计算多余 token。
    let extra_tokens = tokenize(&candidate.normalized_primary)
        .len()
        .saturating_sub(query_token_count);

    let vendor_mismatch = query_vendor
        .zip(candidate.vendor.as_deref())
        .is_some_and(|(query, candidate)| query != candidate);
    let series_mismatch = !query_series.is_empty()
        && (candidate.series_markers.is_empty()
            || query_series.is_disjoint(&candidate.series_markers));
    let missing_roles = query_roles.difference(&candidate.role_tokens).count();
    let extra_roles = candidate.role_tokens.difference(query_roles).count();

    Some((
        Rank {
            level,
            vendor_mismatch,
            series_mismatch,
            missing_roles,
            extra_roles,
            score_millis: (score * 1000.0).round() as i64,
            is_variant: candidate.is_variant,
            extra_tokens,
            release: candidate.release.unwrap_or(0),
            sort_id: candidate.sort_id.clone(),
        },
        kind,
        score,
    ))
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

fn provider_prefix(input: &str) -> Option<String> {
    let (provider, _) = input.trim().split_once('/')?;
    let provider = provider.trim_start_matches('~').trim().to_ascii_lowercase();
    (!provider.is_empty()).then_some(provider)
}

/// 把 `qwen4` / `v5` 这类紧连的字母数字也拆成语义 token。
fn semantic_tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in tokenize(input) {
        let mut current = String::new();
        let mut digit = None;
        for ch in token.chars() {
            let ch_digit = ch.is_ascii_digit();
            if digit.is_some_and(|previous| previous != ch_digit) && !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            current.push(ch);
            digit = Some(ch_digit);
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

/// 提取版本数字前的系列标记：`deepseek-v5` → `v`，`qwen4` → `qwen`。
fn version_series_markers(input: &str) -> HashSet<String> {
    let tokens = semantic_tokens(input);
    let mut markers = HashSet::new();
    for pair in tokens.windows(2) {
        if pair[0].chars().all(|ch| ch.is_ascii_alphabetic())
            && pair[1].chars().all(|ch| ch.is_ascii_digit())
        {
            markers.insert(pair[0].clone());
        }
    }
    markers
}

fn role_tokens(input: &str) -> HashSet<String> {
    semantic_tokens(input)
        .into_iter()
        .filter(|token| ROLE_TOKENS.contains(&token.as_str()))
        .collect()
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
                        Candidate::new((*id).to_string(), Vec::new(), *release, *id),
                        json!({ "id": id }),
                    )
                })
                .collect(),
        )
    }

    fn matched_id<'a>(store: &'a CandidateStore, query: &str) -> Option<&'a str> {
        store.lookup(query).map(|matched| matched.matched_id)
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
                    "deepseek-v4-flash".to_string(),
                    vec!["deepseek-flash".to_string()],
                    Some(1_700_000_000),
                    "deepseek-v4-flash",
                ),
                json!({ "id": "deepseek-v4-flash" }),
            ),
            (
                Candidate::new(
                    "deepseek-v4-flash-vision-exp".to_string(),
                    vec!["deepseek-flash".to_string()],
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
    fn fuzzy_match_prefers_same_version_series_and_role() {
        let store = store(&[
            ("deepseek/deepseek-r1", Some(1_900_000_000)),
            ("deepseek/deepseek-v3.2", Some(1_800_000_000)),
            ("qwen/qwen3-chat", Some(1_900_000_000)),
            ("qwen/qwen3-coder", Some(1_800_000_000)),
        ]);
        assert_eq!(
            matched_id(&store, "deepseek-v5"),
            Some("deepseek/deepseek-v3.2")
        );
        assert_eq!(
            matched_id(&store, "qwen4-coder"),
            Some("qwen/qwen3-coder")
        );
    }

    #[test]
    fn fuzzy_match_prefers_latest_formal_release_in_same_series() {
        let store = store(&[
            ("openai/gpt-5.4", Some(1_700_000_000)),
            ("openai/gpt-5.5", Some(1_800_000_000)),
            ("openai/gpt-5.6-preview", Some(1_900_000_000)),
        ]);
        let matched = store.lookup("gpt-5.7").unwrap();
        assert_eq!(matched.matched_id, "openai/gpt-5.5");
        assert_eq!(matched.kind, MatchKind::Fuzzy);
    }

    #[test]
    fn fuzzy_match_uses_primary_id_shape_when_family_aliases_differ() {
        let store = CandidateStore::new(vec![
            (
                Candidate::new(
                    "gpt-5.5".to_string(),
                    vec!["gpt".to_string()],
                    release_from_date("2026-04-23"),
                    "gpt-5.5",
                ),
                json!({"id":"gpt-5.5"}),
            ),
            (
                Candidate::new(
                    "gpt-5.6".to_string(),
                    vec!["gpt-sol".to_string()],
                    release_from_date("2026-07-09"),
                    "gpt-5.6",
                ),
                json!({"id":"gpt-5.6"}),
            ),
        ]);
        assert_eq!(matched_id(&store, "gpt-5.7"), Some("gpt-5.6"));
    }

    #[test]
    fn fuzzy_match_ranks_explicit_beta_status_after_formal_model() {
        let formal = Candidate::new(
            "openai/gpt-5.5".to_string(),
            Vec::new(),
            Some(1_800_000_000),
            "openai/gpt-5.5",
        );
        let beta = Candidate::new(
            "openai/gpt-5.6".to_string(),
            Vec::new(),
            Some(1_900_000_000),
            "openai/gpt-5.6",
        )
        .with_status(Some("beta"));
        let store = CandidateStore::new(vec![
            (formal, json!({"id":"openai/gpt-5.5"})),
            (beta, json!({"id":"openai/gpt-5.6"})),
        ]);
        assert_eq!(matched_id(&store, "gpt-5.7"), Some("openai/gpt-5.5"));
    }

    #[test]
    fn release_from_date_converts_iso_dates() {
        assert_eq!(release_from_date("1970-01-01"), Some(0));
        assert_eq!(release_from_date("2026-07-31"), Some(1_785_456_000));
        assert_eq!(release_from_date("not-a-date"), None);
    }
}
