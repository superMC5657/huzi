//! 私有源最小读路径(M4):只读`<registry>/<pkg>/index.toml`(file://直读,https经curl).
//! 索引手写解析(版本+sha256+tarball);认证取URL自带或`~/.huzi/credentials`明文(v1).
//! OUT:publish/服务端/yank/预发布/TOFU/签名(见USAGE §6).
use super::version::SemVersion;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
/// 索引单版本:精确版本+tarball文件sha256(64小写hex)+tarball URL.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub version: SemVersion,
    pub sha256: String,
    pub tarball: String,
}
/// 默认源:HUZI_REGISTRY(空串视为未设).
pub fn default_registry() -> Option<String> {
    std::env::var("HUZI_REGISTRY").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
/// 源选择:显式声明优先,其次环境默认源.
pub fn registry_for(decl: Option<&str>) -> Option<String> {
    if let Some(d) = decl.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(d.to_string());
    }
    default_registry()
}
/// 内置保留:std/core/alloc及`std/*`等走编译器内置,不进注册表.
pub fn is_builtin_pkg(name: &str) -> bool {
    name == "std" || name == "core" || name == "alloc"
        || name.starts_with("std/") || name.starts_with("core/") || name.starts_with("alloc/")
}
/// 落盘目录:vendor/<pkg>/<ver>/,含/全名即嵌套(scope/name).
pub fn vendor_dir_for(vendor_root: &Path, pkg: &str, ver: &SemVersion) -> PathBuf {
    vendor_root.join(pkg).join(ver.to_string())
}
/// 只读索引`<registry>/<pkg>/index.toml`;offline直接错.
pub fn read_index(registry: &str, pkg: &str, offline: bool) -> Result<Vec<IndexEntry>, String> {
    if offline {
        return Err(format!("--offline拒绝读索引(包'{}',源'{}');请先联网fetch", pkg, registry));
    }
    let url = format!("{}/{}/index.toml", registry.trim_end_matches('/'), pkg);
    let text = if let Some(p) = url.strip_prefix("file://") {
        fs::read_to_string(p).map_err(|e| format!("读索引{}失败:{}", url, e))?
    } else if url.starts_with("https://") {
        read_https_text(&url, registry)?
    } else {
        return Err(format!("registry{:?}非法:仅file://与https://", registry));
    };
    parse_index_toml(&text)
}
/// 手写解析index.toml;遇`-`报预发布错.
pub fn parse_index_toml(text: &str) -> Result<Vec<IndexEntry>, String> {
    let mut out = Vec::new();
    let (mut ver, mut sha, mut tar): (Option<String>, Option<String>, Option<String>) = (None, None, None);
    for raw in text.lines().chain(["[[package]]"]) {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line == "[[package]]" {
            flush_entry(&mut ver, &mut sha, &mut tar, &mut out)?;
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(format!("索引行非法:{:?}", raw.trim()));
        };
        let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
        match k.trim() {
            "version" => ver = Some(val),
            "sha256" => sha = Some(val),
            "tarball" => tar = Some(val),
            _ => {}
        }
    }
    Ok(out)
}
/// 收拢单条目(缺字段报错;`-`报预发布;sha须64小写hex).
fn flush_entry(ver: &mut Option<String>, sha: &mut Option<String>, tar: &mut Option<String>, out: &mut Vec<IndexEntry>) -> Result<(), String> {
    if ver.is_none() && sha.is_none() && tar.is_none() {
        return Ok(());
    }
    let (Some(v), Some(s), Some(t)) = (ver.take(), sha.take(), tar.take()) else {
        return Err("索引条目缺version/sha256/tarball".to_string());
    };
    if v.contains('-') {
        return Err(format!("版本{:?}暂不支持预发布", v));
    }
    let version = SemVersion::parse(v.trim()).map_err(|e| format!("索引版本非法:{}", e))?;
    let hex = s.strip_prefix("sha256:").unwrap_or(&s).trim().to_string();
    if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(format!("索引sha256非法:{:?}", s));
    }
    if t.trim().is_empty() {
        return Err("索引tarball为空".to_string());
    }
    out.push(IndexEntry { version, sha256: hex, tarball: t.trim().to_string() });
    Ok(())
}
/// 认证token:URL自带(@或token=)优先,否则读credentials明文.
pub fn auth_token_for(registry: &str) -> Option<String> {
    if let Some(after) = registry.split("://").nth(1) {
        if let Some((user, _)) = after.split_once('@') {
            if !user.is_empty() {
                return Some(user.rsplit(':').next().unwrap_or(user).to_string());
            }
        }
        for kv in after.split(['?', '&']) {
            if let Some(t) = kv.strip_prefix("token=") {
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    credentials_lookup(registry)
}
/// 下载tarball到dest并验sha256(file字节);file拷贝/https经curl.
pub fn fetch_tarball(tarball_url: &str, registry: &str, dest: &Path, expect_hex: &str, offline: bool) -> Result<(), String> {
    if offline {
        return Err("--offline拒绝下载tarball;请先联网fetch".to_string());
    }
    let url = resolve_tarball_url(registry, tarball_url);
    if let Some(p) = dest.parent() {
        fs::create_dir_all(p).map_err(|e| format!("创建{}失败:{}", p.display(), e))?;
    }
    if let Some(path) = url.strip_prefix("file://") {
        if !Path::new(path).is_file() {
            return Err(format!("tarball{}不存在", url));
        }
        fs::copy(path, dest).map_err(|e| format!("拷贝tarball失败:{}", e))?;
    } else if url.starts_with("https://") {
        curl_to_file(&url, registry, dest)?;
    } else {
        return Err(format!("tarball{:?}非法:仅file://与https://", tarball_url));
    }
    let bytes = fs::read(dest).map_err(|e| format!("读下载文件失败:{}", e))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    let got = hex::encode(h.finalize());
    if got != expect_hex.trim() {
        return Err(format!("tarball哈希失配:期望{},实际{}", expect_hex, got));
    }
    Ok(())
}
/// tarball相对路径相对`<registry>/<pkg>/`解析,其余原样.
fn resolve_tarball_url(registry: &str, tar: &str) -> String {
    if tar.contains("://") {
        return tar.to_string();
    }
    format!("{}/{}", registry.trim_end_matches('/'), tar.trim_start_matches('/'))
}
/// https文本经curl读(带Bearer).
fn read_https_text(url: &str, registry: &str) -> Result<String, String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", url]);
    if let Some(tok) = auth_token_for(registry) {
        cmd.args(["-H", &format!("Authorization: Bearer {}", tok)]);
    }
    let out = cmd.output().map_err(|e| format!("curl执行失败:{}", e))?;
    if !out.status.success() {
        return Err(format!("读索引{}失败:curl{:?}", url, out.status.code()));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("索引非utf8:{}", e))
}
/// https文件经curl下载(带Bearer).
fn curl_to_file(url: &str, registry: &str, dest: &Path) -> Result<(), String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", url, "-o"]);
    cmd.arg(dest);
    if let Some(tok) = auth_token_for(registry) {
        cmd.args(["-H", &format!("Authorization: Bearer {}", tok)]);
    }
    let out = cmd.output().map_err(|e| format!("curl执行失败:{}", e))?;
    if !out.status.success() {
        return Err(format!("下载{}失败:curl{:?}", url, out.status.code()));
    }
    Ok(())
}
/// 读~/.huzi/credentials:每行`<registry>[=]<token>`,#注释.
fn credentials_lookup(registry: &str) -> Option<String> {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()?;
    let text = fs::read_to_string(PathBuf::from(home).join(".huzi").join("credentials")).ok()?;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (k, v) = line.split_once('=').or_else(|| line.split_once(char::is_whitespace))?;
        if registry.trim() == k.trim().trim_matches('"') || registry.trim().starts_with(k.trim().trim_matches('"')) {
            let tok = v.trim().trim_matches('"').trim();
            if !tok.is_empty() {
                return Some(tok.to_string());
            }
        }
    }
    None
}
