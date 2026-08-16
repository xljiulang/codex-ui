use std::path::Path;

/// 规范化路径键：统一反斜杠、去尾部分隔符、小写（Windows 大小写不敏感）
pub fn norm_key(p: &Path) -> String {
    p.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// Windows 规范化路径转标准显示/交换路径：剥离 `\\?\UNC\...` / `\\?\C:\...` 前缀，
/// 并统一正斜杠为反斜杠（git 等外部输出可能带正斜杠）
pub fn clean_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    };
    s.replace('/', "\\")
}

/// target 是否位于 parent 内（规范化后，Windows 大小写不敏感）
pub fn is_inside_path(parent: &Path, child: &Path) -> bool {
    let p = norm_key(parent);
    let c = norm_key(child);
    c == p || (c.starts_with(&p) && c[p.len()..].starts_with('\\'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn clean_path_normalizes_separators() {
        assert_eq!(clean_path(Path::new("C:/a/b")), "C:\\a\\b");
        assert_eq!(clean_path(Path::new("C:\\a\\b")), "C:\\a\\b");
    }

    #[test]
    fn clean_path_strips_extended_prefix() {
        assert_eq!(clean_path(Path::new(r"\\?\C:\a")), "C:\\a");
        assert_eq!(clean_path(Path::new(r"\\?\UNC\srv\share")), r"\\srv\share");
    }
}
