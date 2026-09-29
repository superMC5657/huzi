//! `vendor/<pkg>/<version>/` 目录内容校验和(M2)。
//!
//! 口径:递归收集目录下全部文件,相对路径(`/` 分隔)排序后逐文件
//! `sha256(文件字节)` 取 hex,再外层 `sha256(拼接(relpath + 0x00 + hex + \n))`,
//! 结果记为 `sha256:<64位小写hex>`(与 `lock.rs` 校验正则同口径)。
//! 文件一律按字节读,不做 CRLF 归一;符号链接跳过(防环);不可读条目按缺失
//! 处理(落盘后即算哈希,读写必成功;校验期缺失直接失配硬错)。
//!
//! `sha2`/`hex` 依赖仅本文件使用,对外只暴露字符串口径。

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// 目录内容校验和(`sha256:<64位小写hex>`,空目录为 e3b0c…7852b855)。
pub fn dir_checksum(path: &Path) -> String {
    let files = collect_files(path);
    let mut outer = Sha256::new();
    for (rel, hex) in &files {
        outer.update(rel.as_bytes());
        outer.update([0x00]);
        outer.update(hex.as_bytes());
        outer.update([b'\n']);
    }
    format!("sha256:{}", hex::encode(outer.finalize()))
}

/// 目录内容与期望校验和是否一致(目录缺失直接 false)。
pub fn verify_dir(path: &Path, expected: &str) -> bool {
    path.is_dir() && dir_checksum(path) == expected
}

/// 递归收集 `(相对路径, 文件sha256hex)`(相对路径 `/` 分隔,BTreeMap 排序)。
fn collect_files(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    collect_into(dir, dir, &mut out);
    out
}

/// 递归下钻:目录跟随,文件按字节读出哈希,符号链接与不可读条目跳过。
fn collect_into(root: &Path, cur: &Path, out: &mut BTreeMap<String, String>) {
    if cur.is_file() && !is_symlink(cur) {
        insert_file(root, cur, out);
        return;
    }
    if !cur.is_dir() || is_symlink(cur) {
        return;
    }
    let Ok(entries) = fs::read_dir(cur) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    children.sort();
    for child in children {
        if child.is_dir() && !is_symlink(&child) {
            collect_into(root, &child, out);
        } else if child.is_file() && !is_symlink(&child) {
            insert_file(root, &child, out);
        }
    }
}

/// 单文件哈希入库(读失败按缺失跳过)。
fn insert_file(root: &Path, path: &Path, out: &mut BTreeMap<String, String>) {
    if let (Some(rel), Ok(bytes)) = (rel_name(root, path), fs::read(path)) {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        out.insert(rel, hex::encode(hasher.finalize()));
    }
}

/// 相对 `root` 的 `/` 分隔相对路径(根自身被直接传入时取文件名)。
fn rel_name(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<&str> = rel.components().filter_map(|c| c.as_os_str().to_str()).collect();
    if parts.is_empty() {
        path.file_name()?.to_str().map(|s| s.to_string())
    } else {
        Some(parts.join("/"))
    }
}

/// 是否符号链接(读不到元数据按非链接处理,上层按不可读跳过)。
fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// 建唯一临时目录(用进程 id + 自增序号隔离并行单测)。
    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "huzc_hash_{}_{}_{}",
            std::process::id(),
            tag,
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("临时目录可建");
        dir
    }

    #[test]
    fn test_empty_dir_hash_is_sha256_empty() {
        let dir = tmp_dir("empty");
        let sum = dir_checksum(&dir);
        assert_eq!(
            sum,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(verify_dir(&dir, &sum));
        assert!(!verify_dir(&dir.join("missing"), &sum));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_file_order_does_not_affect_hash() {
        // 建两个内容相同、写入顺序相反的目录,哈希必须一致(排序稳定)。
        let a = tmp_dir("order_a");
        let b = tmp_dir("order_b");
        fs::write(a.join("one.hz"), "let x = 1").unwrap();
        fs::write(a.join("two.hz"), "let y = 2").unwrap();
        fs::write(b.join("two.hz"), "let y = 2").unwrap();
        fs::write(b.join("one.hz"), "let x = 1").unwrap();
        fs::create_dir_all(a.join("sub")).unwrap();
        fs::create_dir_all(b.join("sub")).unwrap();
        fs::write(a.join("sub/inner.hz"), "inner").unwrap();
        fs::write(b.join("sub/inner.hz"), "inner").unwrap();
        assert_eq!(dir_checksum(&a), dir_checksum(&b));
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    #[test]
    fn test_single_byte_change_changes_hash() {
        let dir = tmp_dir("tamper");
        let file = dir.join("lib.hz");
        fs::write(&file, "export add").unwrap();
        let before = dir_checksum(&dir);
        assert!(verify_dir(&dir, &before));
        fs::write(&file, "export adD").unwrap();
        let after = dir_checksum(&dir);
        assert_ne!(before, after, "改一字节哈希必须变化");
        assert!(!verify_dir(&dir, &before), "篡改后旧哈希必须失配");
        assert!(verify_dir(&dir, &after));
        let _ = fs::remove_dir_all(&dir);
    }
}
