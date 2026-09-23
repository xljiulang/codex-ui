//! 通用小工具：阻塞任务超时执行、工作区目录校验。
//!
//! 与路径字符串规范化（`path_util`）解耦：这里承载涉及 fs 校验与异步执行的共享逻辑，
//! 供 session_fs / git / terminal 等模块复用。

use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::codex::path_util::clean_path;

/// 默认模型缓存中「失败结果」的重试 TTL（秒）。
///
/// 成功结果永久命中（模型列表不频繁变化）；失败结果只保留这么久，过期即重新解析，
/// 避免「首次解析恰好撞上 app-server 未就绪/重连」这一瞬时故障被永久固化，
/// 让后续所有缺省模型解析都拿到同一个失败结果。
pub const DEFAULT_MODEL_FAILURE_TTL_SECS: u64 = 60;

/// 默认模型缓存的命中判定（纯函数，便于单测）：
/// - 未解析过 → `None`（需重新解析）
/// - 成功结果 → 永久命中
/// - 失败结果 → 仅在 [`DEFAULT_MODEL_FAILURE_TTL_SECS`] 内命中，过期后重新解析
///
/// 用 `saturating_duration_since` 比较（系统时钟回退时不会 panic，按「未过期」处理）。
pub fn cached_default_model(
    entry: &Option<(Result<String, String>, Instant)>,
    now: Instant,
) -> Option<Result<String, String>> {
    let (res, at) = entry.as_ref()?;
    match res {
        Ok(_) => Some(res.clone()),
        Err(_) => {
            let age = now.saturating_duration_since(*at);
            (age < Duration::from_secs(DEFAULT_MODEL_FAILURE_TTL_SECS)).then(|| res.clone())
        }
    }
}

/// `spawn_blocking` + 超时的失败原因；闭包自身的 `Err` 会原样透传，不落在这里。
#[derive(Debug)]
pub enum BlockingError {
    /// 超时（秒）
    Timeout(u64),
    /// 阻塞任务 panic（JoinError）
    Join(tokio::task::JoinError),
}

impl fmt::Display for BlockingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockingError::Timeout(secs) => write!(f, "操作超时（{secs} 秒）"),
            BlockingError::Join(e) => write!(f, "任务异常: {e}"),
        }
    }
}

/// 在阻塞线程中执行 `f` 并带超时（秒）：闭包自身的 `Result` 原样透传，
/// 仅超时或任务 panic 产生 `BlockingError`。
pub async fn spawn_blocking_timeout<T, E>(
    timeout_secs: u64,
    f: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> Result<Result<T, E>, BlockingError>
where
    T: Send + 'static,
    E: Send + 'static,
{
    let task = tokio::task::spawn_blocking(f);
    match tokio::time::timeout(Duration::from_secs(timeout_secs), task).await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => Err(BlockingError::Join(e)),
        Err(_) => Err(BlockingError::Timeout(timeout_secs)),
    }
}

/// 校验工作区为存在的绝对目录：相对路径拒绝，缺失/非目录报错
/// （错误文案中的路径统一经 clean_path 展示）。
pub fn resolve_workspace_dir(workspace: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(workspace);
    if !p.is_absolute() {
        return Err("工作目录必须为绝对路径".into());
    }
    let meta =
        std::fs::metadata(&p).map_err(|e| format!("无法访问工作目录 {}: {e}", clean_path(&p)))?;
    if !meta.is_dir() {
        return Err(format!("工作目录不是目录: {}", clean_path(&p)));
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(f)
    }

    #[test]
    fn spawn_blocking_timeout_passes_closure_result_through() {
        block_on(async {
            let ok: Result<Result<i32, String>, BlockingError> =
                spawn_blocking_timeout(10, || Ok::<i32, String>(42)).await;
            assert!(matches!(ok, Ok(Ok(42))));

            let err: Result<Result<i32, String>, BlockingError> =
                spawn_blocking_timeout(10, || Err::<i32, String>("boom".to_string())).await;
            assert!(matches!(err, Ok(Err(e)) if e == "boom"));
        });
    }

    #[test]
    fn spawn_blocking_timeout_reports_timeout() {
        block_on(async {
            let r = spawn_blocking_timeout(1, || -> Result<i32, String> {
                std::thread::sleep(Duration::from_secs(2));
                Ok(1)
            })
            .await;
            assert!(matches!(r, Err(BlockingError::Timeout(1))));
        });
    }

    #[test]
    fn spawn_blocking_timeout_reports_panic() {
        block_on(async {
            let r = spawn_blocking_timeout(10, || -> Result<i32, String> {
                panic!("boom");
            })
            .await;
            assert!(matches!(r, Err(BlockingError::Join(_))));
        });
    }

    #[test]
    fn resolve_workspace_dir_validates_absolute_existing_dir() {
        assert!(resolve_workspace_dir("relative/path").is_err());

        let missing = std::env::temp_dir().join("codexui-util-no-such-dir");
        assert!(resolve_workspace_dir(missing.to_str().unwrap()).is_err());

        let file = std::env::temp_dir().join(format!("codexui-util-file-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        assert!(resolve_workspace_dir(file.to_str().unwrap()).is_err());
        let _ = std::fs::remove_file(&file);

        assert!(resolve_workspace_dir(std::env::temp_dir().to_str().unwrap()).is_ok());
    }

    #[test]
    fn cached_default_model_misses_when_never_resolved() {
        assert_eq!(cached_default_model(&None, Instant::now()), None);
    }

    #[test]
    fn cached_default_model_success_hits_at_any_age() {
        let now = Instant::now();
        let long_after = now + Duration::from_secs(24 * 60 * 60);
        let entry = Some((Ok("gpt-x".to_string()), now));
        assert_eq!(
            cached_default_model(&entry, now),
            Some(Ok("gpt-x".to_string()))
        );
        // 成功结果不做 TTL：再久也是命中
        assert_eq!(
            cached_default_model(&entry, long_after),
            Some(Ok("gpt-x".to_string()))
        );
    }

    #[test]
    fn cached_default_model_failure_expires_after_ttl() {
        let now = Instant::now();
        let entry = Some((Err("模型列表为空".to_string()), now));
        // TTL 内：命中失败结果（避免瞬时故障触发重试风暴）
        assert_eq!(
            cached_default_model(&entry, now + Duration::from_secs(1)),
            Some(Err("模型列表为空".to_string()))
        );
        // 边界：恰好等于 TTL 视为过期
        assert_eq!(
            cached_default_model(
                &entry,
                now + Duration::from_secs(DEFAULT_MODEL_FAILURE_TTL_SECS)
            ),
            None
        );
        // 超出 TTL：重新解析（错误不再被永久固化）
        assert_eq!(
            cached_default_model(
                &entry,
                now + Duration::from_secs(DEFAULT_MODEL_FAILURE_TTL_SECS + 1)
            ),
            None
        );
    }
}
