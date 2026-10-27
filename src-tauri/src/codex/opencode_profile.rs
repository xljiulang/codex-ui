//! OpenCode 客户端传输与请求身份配置。
//!
//! 这里集中保存与 OpenCode 版本绑定的信息，避免协议实现、版本常量和
//! TLS/HTTP2 指纹散落在兼容代理的各处。当前基线为 OpenCode 2.1.4。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use axum::http::header::{HeaderName, HeaderValue};
use serde_json::Value;
use sha1::{Digest, Sha1};
use wreq::{AlpnProtos, ExtensionType, SslCurve, TlsConfig, TlsVersion};

use super::path_util::clean_path;

/// 模拟的 OpenCode 客户端版本。
pub(crate) const OPENCODE_VERSION: &str = "2.1.4";
/// OpenCode 2.1.4 桌面客户端 User-Agent。
pub(crate) const OPENCODE_USER_AGENT: &str = "opencode/latest/2.1.4/desktop";
/// `x-opencode-client` 固定值。
pub(crate) const OPENCODE_CLIENT: &str = "desktop";

/// HTTP 普通请求头的发送顺序（OpenCode 2.1.4 的 `opencode.ai` 连接使用 HTTP/1.1）。
pub(crate) const OPENCODE_HEADER_ORDER: &[&str] = &[
    "authorization",
    "content-type",
    "user-agent",
    "b3",
    "traceparent",
    "x-opencode-client",
    "x-opencode-project",
    "x-opencode-session",
    "x-opencode-session-id",
    "x-session-affinity",
    "x-session-id",
    "connection",
    "accept",
    "accept-encoding",
];
const THREAD_PROJECT_LIMIT: usize = 1024;

/// `threadId → OpenCode 项目 id` 登记表，由 app-server 的线程响应更新。
#[derive(Debug, Default)]
pub(crate) struct ThreadProjectRegistry {
    projects: Mutex<HashMap<String, String>>,
}

impl ThreadProjectRegistry {
    /// 从一个包含 `thread.id` 与 `thread.cwd` 的响应中登记项目。
    pub(crate) fn record_from_response(&self, response: &Value) -> Option<String> {
        let thread = response.get("thread").unwrap_or(response);
        let thread_id = thread.get("id")?.as_str()?.trim();
        let cwd = thread.get("cwd")?.as_str()?.trim();
        if thread_id.is_empty() || cwd.is_empty() {
            return None;
        }
        let project_id = project_id_for_directory(Path::new(cwd));
        self.record(thread_id, project_id)
    }

    /// 登记线程项目；达到上限时整体清空后重填，限制无界增长。
    pub(crate) fn record(&self, thread_id: &str, project_id: String) -> Option<String> {
        let thread_id = thread_id.trim();
        if thread_id.is_empty() || project_id.len() != 40 {
            return None;
        }
        let mut projects = self
            .projects
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if projects.len() >= THREAD_PROJECT_LIMIT && !projects.contains_key(thread_id) {
            projects.clear();
        }
        projects.insert(thread_id.to_string(), project_id);
        Some(thread_id.to_string())
    }

    /// 查线程对应的 OpenCode 项目 id。
    pub(crate) fn get(&self, thread_id: &str) -> Option<String> {
        self.projects
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(thread_id)
            .cloned()
    }
}

const OPENCODE_CIPHER_LIST: &str = concat!(
    "TLS_AES_128_GCM_SHA256:",
    "TLS_AES_256_GCM_SHA384:",
    "TLS_CHACHA20_POLY1305_SHA256:",
    "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256:",
    "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256:",
    "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384:",
    "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384:",
    "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256:",
    "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256:",
    "TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA:",
    "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA:",
    "TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA:",
    "TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA:",
    "TLS_RSA_WITH_AES_128_GCM_SHA256:",
    "TLS_RSA_WITH_AES_256_GCM_SHA384:",
    "TLS_RSA_WITH_AES_128_CBC_SHA:",
    "TLS_RSA_WITH_AES_256_CBC_SHA",
);

const OPENCODE_SIGALGS_LIST: &str = concat!(
    "ecdsa_secp256r1_sha256:",
    "rsa_pss_rsae_sha256:",
    "rsa_pkcs1_sha256:",
    "ecdsa_secp384r1_sha384:",
    "rsa_pss_rsae_sha384:",
    "rsa_pkcs1_sha384:",
    "rsa_pss_rsae_sha512:",
    "rsa_pkcs1_sha512:",
    "ed25519:",
    "ecdsa_secp521r1_sha512:",
    "rsa_pkcs1_sha1",
);

const OPENCODE_CURVES: &[SslCurve] = &[
    SslCurve::X25519_MLKEM768,
    SslCurve::X25519,
    SslCurve::SECP256R1,
    SslCurve::SECP384R1,
];

const OPENCODE_EXTENSION_ORDER: &[ExtensionType] = &[
    ExtensionType::SERVER_NAME,
    ExtensionType::EXTENDED_MASTER_SECRET,
    ExtensionType::RENEGOTIATE,
    ExtensionType::SUPPORTED_GROUPS,
    ExtensionType::EC_POINT_FORMATS,
    ExtensionType::SESSION_TICKET,
    ExtensionType::APPLICATION_LAYER_PROTOCOL_NEGOTIATION,
    ExtensionType::STATUS_REQUEST,
    ExtensionType::SIGNATURE_ALGORITHMS,
    ExtensionType::CERTIFICATE_TIMESTAMP,
    ExtensionType::KEY_SHARE,
    ExtensionType::PSK_KEY_EXCHANGE_MODES,
    ExtensionType::SUPPORTED_VERSIONS,
];

const OPENCODE_EXTENSION_INDICES: &[u8] = &{
    let mut indices = [0u8; OPENCODE_EXTENSION_ORDER.len()];
    let mut index = 0;
    while index < OPENCODE_EXTENSION_ORDER.len() {
        if let Some(position) = ExtensionType::index_of(OPENCODE_EXTENSION_ORDER[index]) {
            indices[index] = position as u8;
        }
        index += 1;
    }
    indices
};

/// OpenCode 2.1.4（Bun 1.4.4 / BoringSSL）实测 TLS ClientHello 配置。
pub(crate) fn tls_config() -> TlsConfig {
    TlsConfig::builder()
        .alpn_protos(AlpnProtos::HTTP1)
        .min_tls_version(TlsVersion::TLS_1_2)
        .max_tls_version(TlsVersion::TLS_1_3)
        .cipher_list(OPENCODE_CIPHER_LIST)
        .sigalgs_list(OPENCODE_SIGALGS_LIST)
        .curves(OPENCODE_CURVES)
        .key_shares_limit(2)
        .enable_ocsp_stapling(true)
        .enable_signed_cert_timestamps(true)
        .enable_ech_grease(false)
        .grease_enabled(false)
        .permute_extensions(false)
        .extension_permutation_indices(OPENCODE_EXTENSION_INDICES)
        .build()
}

/// 由进程当前的工作区推导 OpenCode 项目 id。
pub(crate) fn current_project_id() -> String {
    let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    project_id_for_directory(&directory)
}

/// 按 OpenCode 2.1.4 的规则推导项目 id。
///
/// Git 仓库优先使用规范化 origin：`sha1("git-remote:" + host/path)`；
/// 没有 origin 时使用首个根提交；非 Git 目录使用
/// `sha1("directory:" + canonical path)`。
pub(crate) fn project_id_for_directory(directory: &Path) -> String {
    if let Some(remote) = git_output(directory, &["remote", "get-url", "origin"])
        .and_then(|value| normalize_remote(&value))
    {
        return sha1_hex(format!("git-remote:{remote}").as_bytes());
    }
    if let Some(root) = git_output(directory, &["rev-list", "--max-parents=0", "HEAD"])
        .and_then(|value| value.lines().next().map(str::trim).map(str::to_string))
        .filter(|value| !value.is_empty())
    {
        return root;
    }
    let canonical = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf());
    sha1_hex(format!("directory:{}", clean_path(&canonical)).as_bytes())
}

/// 生成 OpenCode 2.1.4 使用的 B3 与 W3C Trace Context 请求头。
pub(crate) fn tracing_headers() -> [(HeaderName, HeaderValue); 2] {
    let mut random = [0u8; 40];
    if getrandom::fill(&mut random).is_err() {
        // OpenCode 只校验形状；系统随机源失败时仍返回合法、非空的值。
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for (index, byte) in random.iter_mut().enumerate() {
            *byte = (seed.rotate_left((index % 63) as u32) >> ((index % 8) * 8)) as u8;
        }
    }
    let trace = hex(&random[..16]);
    let span = hex(&random[16..24]);
    let parent = hex(&random[24..32]);
    [
        (
            HeaderName::from_static("b3"),
            HeaderValue::from_str(&format!("{trace}-{span}-1-{parent}"))
                .expect("生成的 b3 头必须合法"),
        ),
        (
            HeaderName::from_static("traceparent"),
            HeaderValue::from_str(&format!("00-{trace}-{span}-01"))
                .expect("生成的 traceparent 头必须合法"),
        ),
    ]
}

fn git_output(directory: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn normalize_remote(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(url) = reqwest::Url::parse(value) {
        if url.scheme() == "file" {
            return None;
        }
        let host = url.host_str()?;
        return normalize_remote_parts(host, url.path());
    }
    let (_, rest) = value.split_once('@').unwrap_or(("", value));
    let (host, path) = rest.split_once(':')?;
    normalize_remote_parts(host, path)
}

fn normalize_remote_parts(host: &str, path: &str) -> Option<String> {
    let path = path
        .trim_start_matches('/')
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let host = host.trim().to_ascii_lowercase();
    (!host.is_empty() && !path.is_empty()).then(|| format!("{host}/{path}"))
}

fn sha1_hex(bytes: &[u8]) -> String {
    hex(&Sha1::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_normalization_matches_opencode_214() {
        let expected = "github.com/xljiulang/codex-ui";
        for remote in [
            "https://github.com/xljiulang/codex-ui.git",
            "https://github.com/xljiulang/codex-ui.git/",
            "git@github.com:xljiulang/codex-ui.git",
            "ssh://git@github.com/xljiulang/codex-ui",
        ] {
            assert_eq!(normalize_remote(remote).as_deref(), Some(expected));
        }
        assert_eq!(
            sha1_hex(format!("git-remote:{expected}").as_bytes()),
            "1371ec193610f593ec145236883cd93484e19ef5"
        );
    }

    #[test]
    fn file_remote_and_empty_values_are_ignored() {
        assert_eq!(normalize_remote("file:///D:/repo"), None);
        assert_eq!(normalize_remote("   "), None);
        assert_eq!(normalize_remote("not-a-remote"), None);
    }

    #[test]
    fn non_git_directory_uses_canonical_path_hash() {
        let dir = tempfile::TempDir::new().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        let canonical = clean_path(&canonical);
        assert_eq!(
            project_id_for_directory(dir.path()),
            sha1_hex(format!("directory:{canonical}").as_bytes())
        );
    }

    #[test]
    fn tracing_headers_share_trace_and_span() {
        let [(b3_name, b3), (traceparent_name, traceparent)] = tracing_headers();
        assert_eq!(b3_name, "b3");
        assert_eq!(traceparent_name, "traceparent");
        let b3 = b3.to_str().unwrap();
        let traceparent = traceparent.to_str().unwrap();
        let parts: Vec<&str> = b3.split('-').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(traceparent, format!("00-{}-{}-01", parts[0], parts[1]));
    }

    #[test]
    fn thread_project_registry_records_thread_response() {
        let registry = ThreadProjectRegistry::default();
        let dir = tempfile::TempDir::new().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        let canonical = clean_path(&canonical);
        let expected = sha1_hex(format!("directory:{canonical}").as_bytes());
        let response = serde_json::json!({
            "thread": {
                "id": "thread-1",
                "cwd": dir.path().to_string_lossy()
            }
        });
        assert_eq!(
            registry.record_from_response(&response).as_deref(),
            Some("thread-1")
        );
        assert_eq!(registry.get("thread-1"), Some(expected));
        assert_eq!(registry.get("missing"), None);
    }

    #[test]
    fn tls_profile_matches_captured_opencode_214_shape() {
        assert_eq!(OPENCODE_CIPHER_LIST.split(':').count(), 17);
        assert_eq!(OPENCODE_SIGALGS_LIST.split(':').count(), 11);
        assert_eq!(
            OPENCODE_CURVES,
            &[
                SslCurve::X25519_MLKEM768,
                SslCurve::X25519,
                SslCurve::SECP256R1,
                SslCurve::SECP384R1,
            ]
        );
        assert_eq!(
            OPENCODE_EXTENSION_INDICES,
            &[0, 2, 3, 4, 5, 6, 7, 8, 9, 11, 14, 15, 17]
        );
    }
}
