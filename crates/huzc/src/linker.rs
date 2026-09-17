use std::path::Path;
use std::process::Command;

use crate::cli::LinkerKind;
use crate::die;
use crate::paths::OutputPaths;

/// Run a command and handle errors
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

/// Link with lld-link. It auto-detects the MSVC/Windows SDK lib directories,
/// so no /LIBPATH is needed.
fn link_msvc(paths: &OutputPaths, debug: bool, quiet: bool) {
    // No /ENTRY override: the default console entry (mainCRTStartup) runs the
    // CRT startup and calls `main(argc, argv)` with real arguments. Forcing
    // /ENTRY:main would make the OS call main directly with garbage args.
    let mut lld_args: Vec<String> = vec![format!("/OUT:{}", paths.exe_path.to_str().unwrap())];
    if debug {
        // Keep the debug sections/DWARF info in the executable.
        lld_args.push("/DEBUG".to_string());
    }
    lld_args.extend([
        "/DEFAULTLIB:ucrt.lib".to_string(),
        "/DEFAULTLIB:msvcrt.lib".to_string(),
        "/DEFAULTLIB:legacy_stdio_definitions.lib".to_string(),
        "/DEFAULTLIB:kernel32.lib".to_string(),
        // CommandLineToArgvW (UTF-8 argv fixup in main) lives in shell32.
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

/// Link with the clang driver.
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

/// Link with MinGW's gcc driver. It provides the mingw-w64 startup files and
/// links against msvcrt by default, so no extra libs are needed.
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
        // CommandLineToArgvW (UTF-8 argv fixup in main) lives in shell32.
        mingw_args.push("-lshell32".to_string());
        mingw_args.push("-lws2_32".to_string());
    }
    if cfg!(target_os = "linux") {
        // sqrt, pow, sin, ... are in libm on glibc
        mingw_args.push("-lm".to_string());
        mingw_args.push("-lpthread".to_string());
    }
    let mingw_args_ref: Vec<&str> = mingw_args.iter().map(|s| s.as_str()).collect();
    if !quiet {
        println!("  gcc args: {}", mingw_args_ref.join(" "));
    }

    run_command("gcc", &mingw_args_ref).unwrap_or_else(|e| die(e));
}

/// Host target triple for the clang driver, matching the architecture
/// huzc itself was compiled for (so llc's host output and clang agree).
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

/// Build clang-style linker arguments for the given object file.
/// Adds the C runtime libraries needed by the generated code on Windows,
/// and libm on Linux where the math functions live in a separate library.
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
        // Libraries for C standard functions (printf, malloc, sprintf, etc.)
        args.extend([
            "-lucrt".to_string(),
            "-llegacy_stdio_definitions".to_string(),
            // SetConsoleOutputCP (UTF-8 console setup in main)
            "-lkernel32".to_string(),
            // CommandLineToArgvW (UTF-8 argv fixup in main)
            "-lshell32".to_string(),
            "-lws2_32".to_string(),
        ]);
    }
    if cfg!(target_os = "linux") {
        // sqrt, pow, sin, ... are in libm on glibc
        args.push("-lm".to_string());
        args.push("-lpthread".to_string());
    }
    args
}
