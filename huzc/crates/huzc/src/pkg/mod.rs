//! 包管理与依赖解析 (Package management and dependency resolution)。
//!
//! 支持 `huzi.toml` 清单、`huzc add/fetch/build` 离线子命令,
//! 模块解析优先级:内置 -> 相对路径 -> `vendor/` -> `~/.huzi/packages/`。
//!
//! 模块拆分:`manifest` 清单解析、`version` 版本需求与最高满足求解、
//! `resolve` 模块解析、`solve` 传递闭包、`lock` 锁定文件读写、
//! `cmd` 子命令编排;对外仍经由本模块统一 `pub use` 暴露。

mod cmd;
mod lock;
mod manifest;
mod resolve;
mod solve;
mod version;

pub use cmd::{run_add, run_build, run_fetch};
pub use lock::{HuziLock, LockEntry, format_lock, lock_path_for, parse_lock, read_lock_file, write_lock_file};
pub use manifest::{Dependency, Manifest, format_manifest, parse_manifest};
pub use resolve::{copy_dir_all, find_manifest_file, resolve_package_module};
pub use solve::{SelectedDep, resolve_closure};
pub use version::{Comparator, ComparatorOp, SemVersion, VersionReq, parse_version_req, select_max_satisfying};
