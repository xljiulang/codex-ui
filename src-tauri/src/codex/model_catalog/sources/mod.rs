//! 数据源注册表。
//!
//! - 完整条目源（[`FullEntrySource`]）：能给出权威完整条目，命中即短路，不套模板；
//! - 字段源（[`FactSource`]）：只提取部分字段，按注册顺序逐字段覆盖。
//!
//! 新增数据源：实现对应 trait，然后在 [`fact_sources`]（或 [`full_entry_sources`]）
//! 里追加一行即可，合并、渲染与匹配逻辑都不需要改动。

pub mod official;
pub mod models_dev;
pub mod openrouter;

use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use super::facts::ModelFacts;
use super::matching::MatchKind;

/// 运行时缓存有效期：24 小时内不重复下载（两个字段源共用同一策略）。
pub(super) const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// 完整条目源：命中即整条复用（不套模板）。
pub trait FullEntrySource {
    fn id(&self) -> &'static str;
    /// 未命中返回 `None`；命中返回可写入目录的完整条目。
    fn full_entry(&self, model_id: &str) -> Option<FullEntryMatch>;
}

pub struct FullEntryMatch {
    pub entry: Value,
    pub matched_id: String,
    pub kind: MatchKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchScope {
    Provider,
    OfficialGlobal,
    Global,
}

impl MatchScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::OfficialGlobal => "official_global",
            Self::Global => "global",
        }
    }

    fn quality_base(self) -> u8 {
        match self {
            Self::Provider => 40,
            Self::OfficialGlobal => 30,
            Self::Global => 20,
        }
    }
}

pub struct FactMatch {
    pub facts: ModelFacts,
    pub matched_id: String,
    pub kind: MatchKind,
    pub score: f64,
    pub scope: MatchScope,
}

impl FactMatch {
    /// 作用域优先，同一作用域内再按精确/别名/模糊匹配降级。
    pub fn quality(&self) -> u8 {
        let match_bonus = match self.kind {
            MatchKind::Exact => 4,
            MatchKind::Alias => 3,
            MatchKind::Normalized => 2,
            MatchKind::Fuzzy => 1,
        };
        self.scope.quality_base() + match_bonus
    }
}

/// 字段源：只提取字段，覆盖由 [`ModelFacts::merge_from`] 统一完成。
pub trait FactSource {
    fn id(&self) -> &'static str;
    /// 未命中返回 `None`；命中返回该源能提供的字段（其余为 `None`）。
    fn extract(&self, model_id: &str) -> Option<FactMatch>;
}

/// 完整条目源列表：顺序 = 优先级（靠前者先问）。
pub fn full_entry_sources(base_url: &str) -> Vec<Box<dyn FullEntrySource>> {
    vec![Box::new(official::OfficialModelSource::new(base_url))]
}

/// 字段源列表：顺序 = 覆盖顺序（靠后者覆盖靠前者）。
pub fn fact_sources(app_dir: &Path, base_url: &str) -> Vec<Box<dyn FactSource>> {
    vec![
        Box::new(openrouter::OpenRouterSource::load(app_dir)),
        Box::new(models_dev::ModelsDevSource::load(app_dir, base_url)),
    ]
}

/// 并行刷新字段源的运行时缓存；失败静默（生成时回退内置资源）。
pub async fn refresh_caches(app_dir: &Path) {
    tokio::join!(
        openrouter::refresh_cache(app_dir),
        models_dev::refresh_cache(app_dir),
    );
}

/// 写入运行时缓存：同目录临时文件 + `sync_all` + 重命名，避免半截 JSON。
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建缓存目录失败: {e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&tmp).map_err(|e| format!("写入缓存失败: {e}"))?;
        file.write_all(bytes)
            .map_err(|e| format!("写入缓存失败: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("同步缓存失败: {e}"))?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换缓存失败: {e}"));
    }
    Ok(())
}

/// 缓存是否可直接复用（刷新判定）：文件存在、修改时间在 [`CACHE_TTL`] 内，
/// 且内容能通过 `parse_ok` 校验。内容损坏按“过期”处理，避免坏文件把 24 小时锁死。
pub(super) fn cache_is_reusable(path: &Path, parse_ok: impl FnOnce(&str) -> bool) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let fresh = SystemTime::now()
        .duration_since(modified)
        .map(|age| age < CACHE_TTL)
        .unwrap_or(false);
    if !fresh {
        return false;
    }
    std::fs::read_to_string(path)
        .map(|text| parse_ok(&text))
        .unwrap_or(false)
}

/// 测试支持：写入缓存文件并把 mtime 挪到 `age` 之前（TTL 边界用例）。
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    pub(crate) fn write_cache(path: &Path, text: &str, age: Duration) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now().checked_sub(age).unwrap())
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// 校验函数只需区分“能解析/不能解析”，这里用最简单的形态判定。
    fn parse_ok(text: &str) -> bool {
        text.contains("\"ok\"")
    }

    #[test]
    fn cache_is_reusable_requires_file_ttl_and_parsable_content() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cache").join("sample.json");

        // 缺失
        assert!(!cache_is_reusable(&path, parse_ok));

        // 新鲜且可解析
        test_support::write_cache(&path, r#"{"ok":true}"#, Duration::from_secs(60 * 60));
        assert!(cache_is_reusable(&path, parse_ok));

        // 新鲜但内容不可解析 → 视为过期
        test_support::write_cache(&path, "{ broken", Duration::from_secs(60 * 60));
        assert!(!cache_is_reusable(&path, parse_ok));

        // 超过 24 小时 → 过期
        test_support::write_cache(&path, r#"{"ok":true}"#, Duration::from_secs(25 * 60 * 60));
        assert!(!cache_is_reusable(&path, parse_ok));
    }
}
