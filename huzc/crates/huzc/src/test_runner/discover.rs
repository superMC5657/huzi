//! 测试用例发现:用例种类判定、测试名提取、期望输出定位、目录扫描。

use super::{TestCase, TestCaseKind};
use std::fs;
use std::path::{Path, PathBuf};

fn determine_test_kind(filename: &str) -> TestCaseKind {
    if filename.ends_with(".compile_fail.hz") {
        TestCaseKind::CompileFail
    } else if filename.ends_with(".runtime_fail.hz") {
        TestCaseKind::RuntimeFail
    } else {
        TestCaseKind::Regular
    }
}

fn extract_test_name(filename: &str) -> String {
    if let Some(s) = filename.strip_suffix(".compile_fail.hz") {
        return s.to_string();
    }
    if let Some(s) = filename.strip_suffix(".runtime_fail.hz") {
        return s.to_string();
    }
    if let Some(s) = filename.strip_suffix(".hz") {
        return s.to_string();
    }
    filename.to_string()
}

fn find_expected_stdout(case_path: &Path, name: &str) -> Option<PathBuf> {
    let stdout_name = format!("{}.stdout", name);
    let mut candidates = Vec::new();

    if let Some(parent) = case_path.parent() {
        candidates.push(parent.join("expected").join(&stdout_name));
        if let Some(grandparent) = parent.parent() {
            candidates.push(grandparent.join("expected").join(&stdout_name));
        }
    }
    candidates.push(PathBuf::from("test/expected").join(&stdout_name));
    candidates.push(PathBuf::from("huzc/test/expected").join(&stdout_name));
    candidates.push(case_path.with_extension("stdout"));

    candidates.into_iter().find(|c| c.is_file())
}

fn collect_cases_from_dir(dir: &Path, out: &mut Vec<TestCase>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().is_some_and(|ext| ext == "hz") {
            let filename = p.file_name().unwrap().to_string_lossy().to_string();
            let name = extract_test_name(&filename);
            let kind = determine_test_kind(&filename);
            let expected_stdout = find_expected_stdout(&p, &name);
            out.push(TestCase {
                path: p,
                name,
                kind,
                expected_stdout,
            });
        }
    }
}

pub(super) fn discover_tests(target_str: &str, filter: Option<&str>) -> Vec<TestCase> {
    let mut cases = Vec::new();
    let target = Path::new(target_str);

    if target_str.ends_with(".hz") {
        if !target.is_file() {
            eprintln!("错误: 测试文件未找到: {}", target_str);
            return cases;
        }
        let filename = target.file_name().unwrap().to_string_lossy().to_string();
        let name = extract_test_name(&filename);
        let kind = determine_test_kind(&filename);
        let expected_stdout = find_expected_stdout(target, &name);
        cases.push(TestCase {
            path: target.to_path_buf(),
            name,
            kind,
            expected_stdout,
        });
    } else {
        let base_dir = if target_str == "." {
            if Path::new("test/cases").is_dir() {
                PathBuf::from("test/cases")
            } else if Path::new("huzc/test/cases").is_dir() {
                PathBuf::from("huzc/test/cases")
            } else {
                PathBuf::from(".")
            }
        } else {
            target.to_path_buf()
        };

        collect_cases_from_dir(&base_dir, &mut cases);

        // Check if there is an adjacent neg directory
        let mut neg_dir = None;
        if let Some(parent) = base_dir.parent() {
            let adj = parent.join("neg");
            if adj.is_dir() {
                neg_dir = Some(adj);
            }
        }
        if neg_dir.is_none() && target_str == "." {
            if Path::new("test/neg").is_dir() {
                neg_dir = Some(PathBuf::from("test/neg"));
            } else if Path::new("huzc/test/neg").is_dir() {
                neg_dir = Some(PathBuf::from("huzc/test/neg"));
            }
        }
        if let Some(neg) = neg_dir {
            if neg != base_dir {
                collect_cases_from_dir(&neg, &mut cases);
            }
        }
    }

    if let Some(f) = filter {
        cases.retain(|c| c.name.contains(f));
    }
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}
