use clap::Parser;

/// 用于链接最终可执行文件的链接器/工具链
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq)]
pub enum LinkerKind {
    /// lld-link 搭配 MSVC/Windows SDK（自动探测 SDK 库路径）
    Msvc,
    /// 使用 clang 驱动作为链接器
    Clang,
    /// MinGW-w64（gcc 驱动）
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
    /// 平台默认值：Windows 上为 msvc，其他平台为 clang。
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
    /// 格式化 Huzi 源码文件 (.hz)
    Fmt(FmtArgs),
    /// 使用 huzi.toml 清单构建项目
    Build(BuildArgs),
    /// 向 huzi.toml 添加依赖
    Add(AddArgs),
    /// 获取并本地缓存依赖包
    Fetch(FetchArgs),
}

#[derive(clap::Args, Clone, Debug)]
pub struct FmtArgs {
    /// 仅检查格式化，不覆盖文件
    #[arg(long)]
    pub check: bool,

    /// 目标文件或目录
    pub path: String,
}

#[derive(clap::Args, Clone, Debug)]
pub struct BuildArgs {
    /// 包含 huzi.toml 的项目目录（默认为当前目录）
    #[arg(short, long, default_value = ".")]
    pub path: String,

    /// 输出的可执行文件路径
    #[arg(short, long)]
    pub output: Option<String>,

    /// 使用的链接器
    #[arg(short, long, value_enum, default_value_t = LinkerKind::platform_default())]
    pub linker: LinkerKind,

    /// 发布模式
    #[arg(short = 'r', long)]
    pub release: bool,
}

#[derive(clap::Args, Clone, Debug)]
pub struct AddArgs {
    /// 依赖包名称
    pub package: String,

    /// 包版本（如 1.0.0）
    #[arg(default_value = "0.1.0")]
    pub version: String,

    /// 本地包路径
    #[arg(long)]
    pub path: Option<String>,
}

#[derive(clap::Args, Clone, Debug)]
pub struct FetchArgs {
    /// 包含 huzi.toml 的项目目录
    #[arg(short, long, default_value = ".")]
    pub path: String,
}

/// Huzi 编程语言编译器
#[derive(Parser, Debug)]
#[command(name = "huzc")]
#[command(about = "将 Huzi 源码编译为可执行文件")]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// 输入源码文件 (.hz)
    #[arg(short, long)]
    pub input: Option<String>,

    /// 输出文件名（不含扩展名）。默认使用输入文件主干名（`--input foo/bar.hz` -> 当前目录下的 `bar[.exe]`）
    #[arg(short, long)]
    pub output: Option<String>,

    /// 使用的链接器（Windows 默认为 msvc，macOS/Linux 默认为 clang）
    #[arg(short, long, value_enum, default_value_t = LinkerKind::platform_default())]
    pub linker: LinkerKind,

    /// 发布模式：在生成代码前使用 `opt -O2` 优化 IR。未指定此标志（开发模式）时 IR 将未经优化直接传入 llc
    #[arg(short = 'r', long)]
    pub release: bool,

    /// 供 `opt` 使用的 LLVM 优化级别 (0-3)。优先级高于 `--release`：0 保持 IR 未优化，2 对应 `--release`
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..4))]
    pub opt_level: Option<u8>,

    /// 调试模式：嵌入 DWARF 调试信息（编译单元、行号表、局部变量），以便使用 GDB/LLDB 调试可执行文件。由于优化会扰乱行号归属，该选项隐含优化级别 0
    #[arg(short = 'g', long)]
    pub debug: bool,
}

impl Args {
    /// 有效输出基名：显式 `--output` 优先，否则使用输入文件主干名（`foo/bar.hz` -> `bar`）。若输入路径无法推导主干名则报错退出。
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

    /// 有效 LLVM 优化级别：显式 `--opt-level` 优先于 `--release`。`-g` 强制为 0 级别以保持行号信息精确。
    pub fn effective_opt_level(&self) -> u8 {
        if self.debug {
            return 0;
        }
        self.opt_level.unwrap_or(if self.release { 2 } else { 0 })
    }
}
