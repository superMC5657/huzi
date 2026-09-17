use clap::Parser;

/// Linker/toolchain to use for linking the final executable
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq)]
pub enum LinkerKind {
    /// lld-link with MSVC/Windows SDK (auto-detects SDK libs)
    Msvc,
    /// clang driver as linker
    Clang,
    /// MinGW-w64 (gcc driver)
    Mingw,
}

impl std::fmt::Display for LinkerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LinkerKind::Msvc => "msvc",
            LinkerKind::Clang => "clang",
            LinkerKind::Mingw => "mingw",
        };
        f.write_str(s)
    }
}

impl LinkerKind {
    /// Platform default: msvc on Windows, clang elsewhere.
    pub fn platform_default() -> Self {
        if cfg!(target_os = "windows") {
            LinkerKind::Msvc
        } else {
            LinkerKind::Clang
        }
    }
}

#[derive(clap::Subcommand, Clone, Debug)]
pub enum Command {
    /// Format Huzi source files (.hz)
    Fmt(FmtArgs),
    /// Build project using huzi.toml manifest
    Build(BuildArgs),
    /// Add a dependency to huzi.toml
    Add(AddArgs),
    /// Fetch and vendor dependencies
    Fetch(FetchArgs),
}

#[derive(clap::Args, Clone, Debug)]
pub struct FmtArgs {
    /// Check formatting without overwriting files
    #[arg(long)]
    pub check: bool,

    /// Target file or directory
    pub path: String,
}

#[derive(clap::Args, Clone, Debug)]
pub struct BuildArgs {
    /// Project directory containing huzi.toml (defaults to current directory)
    #[arg(short, long, default_value = ".")]
    pub path: String,

    /// Output executable path
    #[arg(short, long)]
    pub output: Option<String>,

    /// Linker to use
    #[arg(short, long, value_enum, default_value_t = LinkerKind::platform_default())]
    pub linker: LinkerKind,

    /// Release mode
    #[arg(short = 'r', long)]
    pub release: bool,
}

#[derive(clap::Args, Clone, Debug)]
pub struct AddArgs {
    /// Dependency package name
    pub package: String,

    /// Package version (e.g. 1.0.0)
    #[arg(default_value = "0.1.0")]
    pub version: String,

    /// Local path to package
    #[arg(long)]
    pub path: Option<String>,
}

#[derive(clap::Args, Clone, Debug)]
pub struct FetchArgs {
    /// Project directory containing huzi.toml
    #[arg(short, long, default_value = ".")]
    pub path: String,
}

/// Huzi Programming Language Compiler
#[derive(Parser, Debug)]
#[command(name = "huzc")]
#[command(about = "Compile Huzi source code to executable")]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Input source file (.hz)
    #[arg(short, long)]
    pub input: Option<String>,

    /// Output file name (without extension). Defaults to the input
    /// file stem (`--input foo/bar.hz` -> `bar[.exe]` in cwd).
    #[arg(short, long)]
    pub output: Option<String>,

    /// Linker to use (defaults to msvc on Windows, clang on macOS/Linux)
    #[arg(short, long, value_enum, default_value_t = LinkerKind::platform_default())]
    pub linker: LinkerKind,

    /// Release mode: optimize the IR with `opt -O2` before generating code.
    /// Without this flag (dev mode) the IR is passed to llc unoptimized.
    #[arg(short = 'r', long)]
    pub release: bool,

    /// LLVM optimization level for `opt` (0-3). Overrides `--release`:
    /// 0 keeps the IR unoptimized, 2 matches `--release`.
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..4))]
    pub opt_level: Option<u8>,

    /// Debug mode: embed DWARF debug info (compile units, line tables,
    /// variables) so the executable can be debugged with GDB/LLDB.
    /// Implies opt level 0, since optimization scrambles line attribution.
    #[arg(short = 'g', long)]
    pub debug: bool,
}

impl Args {
    /// Effective output base name: explicit `--output` wins, otherwise the
    /// input file stem (`foo/bar.hz` -> `bar`). Dies when the input path
    /// has no file stem instead of silently falling back.
    pub fn effective_output(&self) -> String {
        if let Some(out) = &self.output {
            return out.clone();
        }
        let input = self.input.as_deref().unwrap_or("");
        std::path::Path::new(input)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                crate::die(format!(
                    "无法从输入路径推导输出名,请显式指定 --output: {}",
                    input
                ))
            })
    }

    /// Effective LLVM opt level: explicit `--opt-level` wins over `--release`.
    /// `-g` forces level 0 to keep line info accurate.
    pub fn effective_opt_level(&self) -> u8 {
        if self.debug {
            return 0;
        }
        self.opt_level.unwrap_or(if self.release { 2 } else { 0 })
    }
}
