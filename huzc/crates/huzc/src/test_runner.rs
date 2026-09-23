use crate::cli::TestArgs;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TestCaseKind {
    Regular,
    CompileFail,
    RuntimeFail,
}

#[derive(Clone, Debug)]
pub struct TestCase {
    pub path: PathBuf,
    pub name: String,
    pub kind: TestCaseKind,
    pub expected_stdout: Option<PathBuf>,
}

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

    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn collect_cases_from_dir(dir: &Path, out: &mut Vec<TestCase>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().map_or(false, |ext| ext == "hz") {
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

fn discover_tests(target_str: &str, filter: Option<&str>) -> Vec<TestCase> {
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

fn format_diff(expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let max = exp_lines.len().max(act_lines.len());
    let mut diff = String::new();
    let mut diff_count = 0;
    for i in 0..max {
        let e = exp_lines.get(i).copied().unwrap_or("<EOF>");
        let a = act_lines.get(i).copied().unwrap_or("<EOF>");
        if e != a {
            if diff_count < 5 {
                diff.push_str(&format!(
                    "  line {}:\n    expected: {:?}\n    actual:   {:?}\n",
                    i + 1,
                    e,
                    a
                ));
            }
            diff_count += 1;
        }
    }
    if diff_count > 5 {
        diff.push_str(&format!(
            "  ... and {} more differing lines\n",
            diff_count - 5
        ));
    }
    diff
}

fn compile_case(case: &TestCase, out_exe: &Path, args: &TestArgs) -> Result<(), String> {
    let huzc_exe = std::env::current_exe()
        .map_err(|e| format!("无法获取当前编译器可执行文件路径: {}", e))?;

    let mut cmd = Command::new(&huzc_exe);
    cmd.arg("-i").arg(&case.path);
    cmd.arg("-o").arg(out_exe);
    cmd.arg("--linker").arg(args.linker.to_string());
    if args.release {
        cmd.arg("--release");
    }
    cmd.env("HUZI_TEST_ENV", "huzi_env_ok");

    let output = cmd
        .output()
        .map_err(|e| format!("执行编译器失败: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        let first_err = err_msg
            .lines()
            .find(|l| l.contains("error") || l.contains("Error"))
            .unwrap_or_else(|| err_msg.lines().next().unwrap_or("compile error"))
            .to_string();
        Err(first_err)
    }
}

fn run_executable_with_timeout(
    exe_path: &Path,
    stdin_content: Option<&str>,
    timeout: Duration,
) -> Result<(i32, String), String> {
    let mut cmd = Command::new(exe_path);
    cmd.env("HUZI_TEST_ENV", "huzi_env_ok");

    if stdin_content.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("启动测试进程失败: {}", e))?;

    if let Some(input) = stdin_content {
        use std::io::Write;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes());
        }
    }

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout_buf = Vec::new();
                let mut stderr_buf = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    use std::io::Read;
                    let _ = out.read_to_end(&mut stdout_buf);
                }
                if let Some(mut err) = child.stderr.take() {
                    use std::io::Read;
                    let _ = err.read_to_end(&mut stderr_buf);
                }
                let combined = format!(
                    "{}{}",
                    String::from_utf8_lossy(&stdout_buf),
                    String::from_utf8_lossy(&stderr_buf)
                );
                return Ok((status.code().unwrap_or(1), combined));
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("运行超时（超过 {} 秒）", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("等待子进程异常: {}", e)),
        }
    }
}

fn resolve_exe_path(out_exe: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        let mut s = out_exe.to_path_buf().into_os_string();
        s.push(".exe");
        PathBuf::from(s)
    } else {
        out_exe.to_path_buf()
    }
}

fn run_single_test(
    case: &TestCase,
    out_dir: &Path,
    args: &TestArgs,
) -> Result<(), String> {
    if case.name == "10_guess_number_game" {
        println!("SKIP(interactive): {}", case.name);
        return Ok(());
    }

    let out_exe_base = out_dir.join(&case.name);
    let compile_res = compile_case(case, &out_exe_base, args);

    match case.kind {
        TestCaseKind::CompileFail => match compile_res {
            Ok(_) => Err(format!("FAIL(neg-compile-should-fail): {}", case.name)),
            Err(_) => {
                println!("PASS(neg-compile): {}", case.name);
                Ok(())
            }
        },
        TestCaseKind::RuntimeFail => {
            if let Err(e) = compile_res {
                return Err(format!("FAIL(neg-runtime-compile): {} ({})", case.name, e));
            }
            let real_exe = resolve_exe_path(&out_exe_base);
            let (code, _) = run_executable_with_timeout(&real_exe, None, Duration::from_secs(10))?;
            if code == 0 {
                Err(format!("FAIL(neg-runtime-should-fail): {}", case.name))
            } else {
                println!("PASS(neg-runtime): {}", case.name);
                Ok(())
            }
        }
        TestCaseKind::Regular => {
            if let Err(e) = compile_res {
                return Err(format!("FAIL(compile): {} ({})", case.name, e));
            }
            let real_exe = resolve_exe_path(&out_exe_base);
            let stdin_data = if case.name == "24_pipe_read" {
                Some("a\nb\n")
            } else {
                None
            };
            let (code, output) =
                run_executable_with_timeout(&real_exe, stdin_data, Duration::from_secs(10))?;
            if code != 0 {
                return Err(format!("FAIL(run/{}): {}", code, case.name));
            }
            if let Some(expected_path) = &case.expected_stdout {
                let exp = fs::read_to_string(expected_path)
                    .map_err(|e| format!("读取期望输出失败: {}", e))?;
                compare_stdout(&case.name, &exp, &output)?;
            }
            println!("PASS: {}", case.name);
            Ok(())
        }
    }
}

fn compare_stdout(case_name: &str, exp: &str, output: &str) -> Result<(), String> {
    let exp_norm = exp.replace("\r\n", "\n");
    let act_norm = output.replace("\r\n", "\n");
    if exp_norm == act_norm {
        return Ok(());
    }
    if case_name == "23_cli_args" {
        let exp_lines: Vec<&str> = exp_norm.lines().collect();
        let act_lines: Vec<&str> = act_norm.lines().collect();
        if exp_lines.len() == act_lines.len() {
            let mut match_all = true;
            for (i, (e, a)) in exp_lines.iter().zip(act_lines.iter()).enumerate() {
                if i == 1 {
                    if !a.starts_with("程序名: ") || !a.contains("23_cli_args") {
                        match_all = false;
                        break;
                    }
                } else if e != a {
                    match_all = false;
                    break;
                }
            }
            if match_all {
                return Ok(());
            }
        }
    }
    let diff = format_diff(&exp_norm, &act_norm);
    Err(format!("FAIL(stdout): {}\n{}", case_name, diff))
}

fn resolve_output_dir(case: &TestCase) -> PathBuf {
    if let Some(parent) = case.path.parent() {
        if let Some(grandparent) = parent.parent() {
            let candidate = grandparent.join("out");
            let _ = fs::create_dir_all(&candidate);
            return candidate;
        }
    }
    let fallback = PathBuf::from("test/out");
    let _ = fs::create_dir_all(&fallback);
    fallback
}

pub fn run_tests(args: &TestArgs) -> bool {
    let cases = discover_tests(&args.path, args.filter.as_deref());
    if cases.is_empty() {
        println!("未找到匹配的测试用例。路径: {}", args.path);
        return true;
    }

    // Ensure common test/out directory exists for cases that perform file I/O
    let _ = fs::create_dir_all("test/out");
    let _ = fs::create_dir_all("huzc/test/out");

    let mut pass = 0;
    let mut fail = 0;
    let mut skip = 0;

    for case in &cases {
        if case.name == "10_guess_number_game" {
            skip += 1;
            println!("SKIP(interactive): {}", case.name);
            continue;
        }

        let out_dir = resolve_output_dir(case);
        match run_single_test(case, &out_dir, args) {
            Ok(()) => pass += 1,
            Err(e) => {
                fail += 1;
                eprintln!("{}", e);
            }
        }
    }

    println!("-----------------------------");
    println!("{} passed, {} failed, {} skipped", pass, fail, skip);
    fail == 0
}
