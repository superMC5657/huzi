use std::path::Path;
use std::process::Command;

use crate::cli::LinkerKind;
use crate::die;
use crate::paths::OutputPaths;

/// 运行命令并处理错误
pub fn run_command(cmd: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run {}: {}", cmd, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("{} error:\nstdout: {}\nstderr: {}", cmd, stdout, stderr));
    }
    Ok(())
}

pub fn link(paths: &OutputPaths, linker: LinkerKind, debug: bool, quiet: bool) {
    match linker {
        LinkerKind::Msvc => link_msvc(paths, debug, quiet),
        LinkerKind::Clang => link_clang(paths, debug, quiet),
        LinkerKind::Mingw => link_mingw(paths, debug, quiet),
    }
}

/// 使用 lld-link 进行链接。它会自动探测 MSVC/Windows SDK 库目录，因此无需手动指定 /LIBPATH。
fn link_msvc(paths: &OutputPaths, debug: bool, quiet: bool) {
    // 不覆盖 /ENTRY：默认的控制台入口 (mainCRTStartup) 会执行 CRT 初始化并携带真实参数调用 main(argc, argv)。
    // 强制使用 /ENTRY:main 会导致操作系统直接以垃圾参数调用 main。
    let mut lld_args: Vec<String> = vec![format!("/OUT:{}", paths.exe_path.to_str().unwrap())];
    if debug {
        // 在可执行文件中保留调试段与 DWARF 信息。
        lld_args.push("/DEBUG".to_string());
    }
    lld_args.extend([
        "/DEFAULTLIB:ucrt.lib".to_string(),
        "/DEFAULTLIB:msvcrt.lib".to_string(),
        "/DEFAULTLIB:legacy_stdio_definitions.lib".to_string(),
        "/DEFAULTLIB:kernel32.lib".to_string(),
        // CommandLineToArgvW（main 中的 UTF-8 argv 修复逻辑）位于 shell32 中。
        "/DEFAULTLIB:shell32.lib".to_string(),
        "/DEFAULTLIB:ws2_32.lib".to_string(),
        paths.obj_path.to_str().unwrap().to_string(),
    ]);
    let lld_args_ref: Vec<&str> = lld_args.iter().map(|s| s.as_str()).collect();
    if !quiet {
        println!("  lld-link args: {}", lld_args_ref.join(" "));
    }

    run_command("lld-link", &lld_args_ref).unwrap_or_else(|e| die(e));
}

/// 使用 clang 驱动进行链接。
fn link_clang(paths: &OutputPaths, debug: bool, quiet: bool) {
    let mut clang_args =
        clang_link_args(Some(clang_target().as_str()), &paths.exe_path, &paths.obj_path);
    if debug {
        clang_args.splice(2..2, ["-g".to_string()]);
    }
    let clang_args_ref: Vec<&str> = clang_args.iter().map(|s| s.as_str()).collect();
    if !quiet {
        println!("  clang args: {}", clang_args_ref.join(" "));
    }

    run_command("clang", &clang_args_ref).unwrap_or_else(|e| die(e));
}

/// 使用 MinGW 的 gcc 驱动进行链接。它提供 mingw-w64 启动文件并默认链接 msvcrt，无需额外库。
fn link_mingw(paths: &OutputPaths, debug: bool, quiet: bool) {
    let mut mingw_args: Vec<String> = vec![
        "-o".to_string(),
        paths.exe_path.to_str().unwrap().to_string(),
    ];
    if debug {
        mingw_args.push("-g".to_string());
    }
    mingw_args.push(paths.obj_path.to_str().unwrap().to_string());
    if cfg!(target_os = "windows") {
        // CommandLineToArgvW（main 中的 UTF-8 argv 修复逻辑）位于 shell32 中。
        mingw_args.push("-lshell32".to_string());
        mingw_args.push("-lws2_32".to_string());
    }
    if cfg!(target_os = "linux") {
        // glibc 上 sqrt, pow, sin 等数学函数位于 libm
        mingw_args.push("-lm".to_string());
        mingw_args.push("-lpthread".to_string());
    }
    let mingw_args_ref: Vec<&str> = mingw_args.iter().map(|s| s.as_str()).collect();
    if !quiet {
        println!("  gcc args: {}", mingw_args_ref.join(" "));
    }

    run_command("gcc", &mingw_args_ref).unwrap_or_else(|e| die(e));
}

/// clang 驱动的主机目标三元组（Host target triple），与编译 huzc 自身的架构对齐（使 llc 输出与 clang 匹配）。
fn clang_target() -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        _ => "x86_64",
    };
    if cfg!(target_os = "windows") {
        format!("{}-pc-windows-msvc", arch)
    } else if cfg!(target_os = "macos") {
        format!("{}-apple-darwin", arch)
    } else {
        format!("{}-unknown-linux-gnu", arch)
    }
}

/// 为指定目标文件构建 clang 风格的链接器参数。
/// 在 Windows 上添加生成代码所需的 C 运行时库，在 Linux 上添加 libm（数学函数位于独立库中）。
fn clang_link_args(target: Option<&str>, exe_path: &Path, obj_path: &Path) -> Vec<String> {
    let mut args = vec![
        "-o".to_string(),
        exe_path.to_str().unwrap().to_string(),
    ];
    if let Some(t) = target {
        args.extend(["-target".to_string(), t.to_string()]);
    }
    args.push(obj_path.to_str().unwrap().to_string());
    if cfg!(target_os = "windows") {
        // C 标准库函数所需链接库（printf, malloc, sprintf 等）
        args.extend([
            "-lucrt".to_string(),
            "-llegacy_stdio_definitions".to_string(),
            // SetConsoleOutputCP（main 中的 UTF-8 控制台初始化）
            "-lkernel32".to_string(),
            // CommandLineToArgvW（main 中的 UTF-8 argv 修复逻辑）
            "-lshell32".to_string(),
            "-lws2_32".to_string(),
        ]);
    }
    if cfg!(target_os = "linux") {
        // glibc 上 sqrt, pow, sin 等数学函数位于 libm
        args.push("-lm".to_string());
        args.push("-lpthread".to_string());
    }
    args
}
