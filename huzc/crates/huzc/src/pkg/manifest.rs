//! 清单解析:`huzi.toml` 文本 ↔ `Manifest`(手写解析,无三方依赖,离线即用)。

use super::version::{VersionReq, parse_version_req};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub entry: Option<String>,
    pub lib_entry: Option<String>,
    pub dependencies: HashMap<String, Dependency>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dependency {
    /// 版本原串(保留;精确路径 `vendor/<pkg>/<version>/` 仍用此串)。
    pub version: String,
    pub path: Option<String>,
}

impl Dependency {
    /// 将 `version` 原串解析为版本需求(新增解析视角,原串不变)。
    pub fn version_req(&self) -> Result<VersionReq, String> {
        parse_version_req(&self.version)
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            name: "app".to_string(),
            version: "0.1.0".to_string(),
            entry: None,
            lib_entry: None,
            dependencies: HashMap::new(),
        }
    }
}

/// 解析简单的 `huzi.toml` 文本 (无三方依赖,离线即用)。
pub fn parse_manifest(content: &str) -> Result<Manifest, String> {
    let mut manifest = Manifest::default();
    let mut current_section = "";

    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim();
            continue;
        }

        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();

        match current_section {
            "package" => match key {
                "name" => manifest.name = trim_quotes(val),
                "version" => manifest.version = trim_quotes(val),
                "entry" => manifest.entry = Some(trim_quotes(val)),
                "lib_entry" | "lib" => manifest.lib_entry = Some(trim_quotes(val)),
                _ => {}
            },
            "dependencies" => {
                let dep = parse_dependency_val(val);
                manifest.dependencies.insert(key.to_string(), dep);
            }
            _ => {}
        }
    }

    Ok(manifest)
}

fn trim_quotes(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn parse_dependency_val(val: &str) -> Dependency {
    let val = val.trim();
    if val.starts_with('{') && val.ends_with('}') {
        let inner = &val[1..val.len() - 1];
        let mut version = "0.1.0".to_string();
        let mut path = None;
        for pair in inner.split(',') {
            if let Some((k, v)) = pair.split_once(':') {
                let k = k.trim();
                let v = trim_quotes(v);
                if k == "version" {
                    version = v;
                } else if k == "path" {
                    path = Some(v);
                }
            } else if let Some((k, v)) = pair.split_once('=') {
                let k = k.trim();
                let v = trim_quotes(v);
                if k == "version" {
                    version = v;
                } else if k == "path" {
                    path = Some(v);
                }
            }
        }
        Dependency { version, path }
    } else {
        Dependency {
            version: trim_quotes(val),
            path: None,
        }
    }
}

pub fn format_manifest(manifest: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("[package]\n");
    out.push_str(&format!("name = \"{}\"\n", manifest.name));
    out.push_str(&format!("version = \"{}\"\n", manifest.version));
    if let Some(entry) = &manifest.entry {
        out.push_str(&format!("entry = \"{}\"\n", entry));
    }
    if let Some(lib_entry) = &manifest.lib_entry {
        out.push_str(&format!("lib_entry = \"{}\"\n", lib_entry));
    }
    out.push('\n');

    out.push_str("[dependencies]\n");
    let mut dep_keys: Vec<_> = manifest.dependencies.keys().collect();
    dep_keys.sort();
    for k in dep_keys {
        let dep = &manifest.dependencies[k];
        if let Some(path) = &dep.path {
            out.push_str(&format!(
                "{} = {{ version = \"{}\", path = \"{}\" }}\n",
                k, dep.version, path
            ));
        } else {
            out.push_str(&format!("{} = \"{}\"\n", k, dep.version));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_parse_and_format() {
        let toml = r#"
[package]
name = "my_app"
version = "1.2.0"
entry = "src/app.hz"
lib_entry = "src/lib.hz"

[dependencies]
foo = "0.1.0"
bar = { version = "2.0.0", path = "../bar" }
"#;
        let m = parse_manifest(toml).expect("parse ok");
        assert_eq!(m.name, "my_app");
        assert_eq!(m.version, "1.2.0");
        assert_eq!(m.entry.as_deref(), Some("src/app.hz"));
        assert_eq!(m.lib_entry.as_deref(), Some("src/lib.hz"));
        assert_eq!(m.dependencies.len(), 2);
        assert_eq!(m.dependencies["foo"].version, "0.1.0");
        assert_eq!(m.dependencies["foo"].path, None);
        assert_eq!(m.dependencies["bar"].version, "2.0.0");
        assert_eq!(m.dependencies["bar"].path.as_deref(), Some("../bar"));

        let formatted = format_manifest(&m);
        assert!(formatted.contains("name = \"my_app\""));
        assert!(formatted.contains("lib_entry = \"src/lib.hz\""));
        assert!(formatted.contains("bar = { version = \"2.0.0\", path = \"../bar\" }"));
        assert!(formatted.contains("foo = \"0.1.0\""));
    }

    #[test]
    fn test_dependency_version_req() {
        let dep = Dependency {
            version: "^1.2.0".to_string(),
            path: None,
        };
        let r = dep.version_req().expect("req 解析成功");
        assert!(r.matches_str("1.5.0").unwrap());
        assert!(!r.matches_str("2.0.0").unwrap());
        // version 原串保留,精确路径行为不变
        assert_eq!(dep.version, "^1.2.0");
    }
}
