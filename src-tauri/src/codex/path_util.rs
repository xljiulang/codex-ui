use std::path::Path;

/// 规范化路径键：统一反斜杠、去尾部分隔符、小写（Windows 大小写不敏感）
pub fn norm_key(p: &Path) -> String {
    p.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// 清洗 + 规范化路径键：先 clean_path（剥离 `\\?\` 前缀、统一反斜杠），
/// 再 norm_key（去尾分隔符、小写）。路径比较/键场景的统一入口。
pub fn norm_path_key(p: &Path) -> String {
    norm_key(Path::new(&clean_path(p)))
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

/// 相对化：先 clean_path 统一反斜杠并去尾分隔符，再按组件逐段大小写不敏感比较；
/// 相等返回 "."，在 root 内返回正斜杠相对路径，越界返回 None。
/// 基于原始大小写组件拼接，不做小写化后的字节切片（规避 Unicode 小写映射
/// 改变字节长度导致切错/panic，如 `İ`/`ẞ`）。
pub fn rel_path_of(root: &Path, path: &Path) -> Option<String> {
    let root_s = clean_path(root);
    let path_s = clean_path(path);
    let root_parts: Vec<&str> = root_s.split('\\').filter(|s| !s.is_empty()).collect();
    let path_parts: Vec<&str> = path_s.split('\\').filter(|s| !s.is_empty()).collect();
    if root_parts.len() > path_parts.len() {
        return None;
    }
    for (a, b) in root_parts.iter().zip(path_parts.iter()) {
        if a.to_lowercase() != b.to_lowercase() {
            return None;
        }
    }
    if root_parts.len() == path_parts.len() {
        return Some(".".to_string());
    }
    Some(path_parts[root_parts.len()..].join("/"))
}

/// Windows 绝对路径解析（diff 场景）：盘符绝对路径/UNC 原样返回（输出统一反斜杠），
/// 相对路径按 root 拼接；驱动器相对路径（`C:foo`）无法确定驱动器当前目录，报错。
pub fn resolve_abs_path(p: &str, root: &str) -> Result<String, String> {
    let h = p.replace('\\', "/");
    let bytes = h.as_bytes();
    let is_drive_abs = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'/';
    if is_drive_abs || h.starts_with("//") {
        return Ok(h.replace('/', "\\"));
    }
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(format!("不支持驱动器相对路径: {p}"));
    }
    let trimmed = h.trim_start_matches('/');
    let base = root.replace('\\', "/").trim_end_matches('/').to_string();
    Ok(format!("{}/{}", base, trimmed).replace('/', "\\"))
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

    #[test]
    fn norm_key_unifies_separators_case_and_trailing() {
        assert_eq!(norm_key(Path::new("C:/a/b")), "c:\\a\\b");
        assert_eq!(norm_key(Path::new("C:\\A\\B\\")), "c:\\a\\b");
        assert_eq!(norm_key(Path::new(r"\\srv\Share\")), r"\\srv\share");
    }

    #[test]
    fn norm_path_key_strips_verbatim_prefix() {
        assert_eq!(norm_path_key(Path::new(r"\\?\C:\A\B")), "c:\\a\\b");
        assert_eq!(norm_path_key(Path::new(r"\\?\UNC\Srv\Share\")), r"\\srv\share");
        assert_eq!(norm_path_key(Path::new("C:/A/B")), "c:\\a\\b");
    }

    #[test]
    fn is_inside_path_handles_case_boundary_and_prefix() {
        assert!(is_inside_path(Path::new("C:/a"), Path::new("C:\\A\\b")));
        assert!(is_inside_path(Path::new("C:\\a"), Path::new("c:/a")));
        assert!(is_inside_path(Path::new("C:\\a"), Path::new("C:\\a\\b\\c")));
        assert!(!is_inside_path(Path::new("C:\\a"), Path::new("C:\\aX")));
        assert!(!is_inside_path(Path::new("C:\\a\\b"), Path::new("C:\\a")));
    }

    #[test]
    fn rel_path_of_matches_case_insensitively() {
        assert_eq!(
            rel_path_of(Path::new("C:/root"), Path::new("C:\\Root\\a\\b.txt")).as_deref(),
            Some("a/b.txt")
        );
        assert_eq!(
            rel_path_of(Path::new("C:\\root"), Path::new("C:\\root")).as_deref(),
            Some(".")
        );
        assert_eq!(
            rel_path_of(Path::new("C:\\root"), Path::new("D:\\other\\a")),
            None
        );
        assert_eq!(
            rel_path_of(Path::new("C:\\root"), Path::new("C:\\rootX\\a")),
            None
        );
    }

    #[test]
    fn rel_path_of_handles_verbatim_and_unc() {
        assert_eq!(
            rel_path_of(Path::new(r"\\?\C:\root"), Path::new(r"\\?\C:\root\src\a.txt"))
                .as_deref(),
            Some("src/a.txt")
        );
        assert_eq!(
            rel_path_of(Path::new(r"\\srv\share"), Path::new(r"\\?\UNC\srv\share\f.txt"))
                .as_deref(),
            Some("f.txt")
        );
    }

    #[test]
    fn rel_path_of_unicode_lowercase_length_change_does_not_panic() {
        // İ 的小写（i̇）字节长度不同于原始字符，旧实现按小写化长度切片会切错/panic
        assert_eq!(
            rel_path_of(Path::new("C:\\İ"), Path::new("C:\\İ\\儿童.txt")).as_deref(),
            Some("儿童.txt")
        );
    }

    #[test]
    fn resolve_abs_path_matches_diff_semantics() {
        assert_eq!(resolve_abs_path("C:/a/b.txt", "D:\\root").unwrap(), "C:\\a\\b.txt");
        assert_eq!(
            resolve_abs_path(r"\\srv\share\f.txt", "D:\\root").unwrap(),
            "\\\\srv\\share\\f.txt"
        );
        assert_eq!(
            resolve_abs_path("src/a.txt", "D:\\root").unwrap(),
            "D:\\root\\src\\a.txt"
        );
        assert_eq!(resolve_abs_path("a.txt", "D:\\root\\").unwrap(), "D:\\root\\a.txt");
        assert!(resolve_abs_path("C:foo", "D:\\root").is_err());
    }
}
