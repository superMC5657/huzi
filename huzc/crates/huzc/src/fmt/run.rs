//! `huzc fmt [--check]` 的文件遍历驱动:收集 .hz 文件,逐个格式化
//! 并按 `--check` 比对或回写,保留原有的 CRLF 行尾风格。

use super::format_source;
use std::fs;
use std::path::{Path, PathBuf};

pub fn run_fmt(path: &str, check: bool) -> bool {
    let p = Path::new(path);
    if !p.exists() {
        eprintln!("Error: path '{}' does not exist", path);
        return false;
    }
    let mut files = Vec::new();
    collect_hz_files(p, &mut files);
    if files.is_empty() {
        if p.is_file() {
            files.push(p.to_path_buf());
        } else {
            eprintln!("No .hz files found in '{}'", path);
            return true;
        }
    }

    files.sort();
    let mut any_diff = false;
    let mut checked_count = 0;

    for file_path in files {
        if process_file(&file_path, check) {
            any_diff = true;
            continue;
        }
        checked_count += 1;
    }

    if check {
        if any_diff {
            eprintln!("Some files are not formatted.");
            return false;
        } else {
            println!("All {} file(s) are properly formatted.", checked_count);
            return true;
        }
    }

    true
}

/// 处理单个 .hz 文件:格式化并按 `--check` 报差异或回写(保留原
/// CRLF 行尾风格)。返回 true 表示读取/解析出错或存在差异。
fn process_file(file_path: &Path, check: bool) -> bool {
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Error reading {}: {}", file_path.display(), err);
            return true;
        }
    };

    let formatted = match format_source(&content) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("Error parsing {}: {}", file_path.display(), err);
            return true;
        }
    };

    let formatted = if content.contains("\r\n") {
        formatted.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        formatted
    };

    if content == formatted {
        return false;
    }
    if check {
        println!("Diff in {}", file_path.display());
    } else if let Err(err) = fs::write(file_path, &formatted) {
        eprintln!("Error writing {}: {}", file_path.display(), err);
    } else {
        println!("Formatted {}", file_path.display());
    }
    true
}

fn collect_hz_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|ext| ext == "hz") {
            out.push(path.to_path_buf());
        }
        return;
    }
    if path.is_dir() {
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if file_name == "target" || file_name == ".git" || file_name == ".omo" {
            return;
        }
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                collect_hz_files(&entry.path(), out);
            }
        }
    }
}
