# Huzi 仓库中文入口

Huzi 是一种简洁、强类型、注重人体工学的静态编译型编程语言，语法风格类似 Python，后端基于 LLVM-18 生成高效原生机器代码。本仓库为多模块工作区，新人按本文档可一次跑通本地门禁。

> **路径基准说明**：本文所有命令默认 cwd=仓库根；`huzc/` 内命令（`huzc/README.md`、`cargo build --workspace`、`bash test.sh` 等）默认 cwd=`huzc`，两者路径差一个 `huzc/` 前缀，请勿混用。项目级文档统一位于仓库根 `docs/`，项目级约定见根 `AGENTS.md`。

## 四模块一览

| 模块 | 路径 | 说明 |
|------|------|------|
| 编译器与工具链 | `huzc/` | 官方参考编译器（Rust workspace + LLVM 后端），含 `crates/`、`test/cases/`、`test.sh` |
| 自举标准库 | `huzi-src/` | Huzi 自举标准库源码（core/alloc/std + test 自测），含 `test.sh` |
| 编辑器支持 | `editors/` | 编辑器支持矩阵与 VS Code 插件工程 |
| 自举示例 | `examples/` | Huzi 重写示例（hzlex/hzparse/task_engine），与编译器仓库同级 |
| 项目文档 | `docs/` | 用户手册与技术文档（USAGE 入口 + STATUS + guides/dev/rfc） |

另有两项仓库级入口：`check.sh`（统一本地门禁）、`.omo/`（内部计划 `plans/` + 会话续跑 `run-continuation/`，只看不用、勿删）。

## 一键全部门禁（`check.sh` 四阶段）

在仓库根执行 `bash check.sh`（需 Git Bash / MSYS2 / WSL，跳过性能抽查用 `SKIP_BENCH=1 bash check.sh` 或 `bash check.sh --skip-bench`）：

| 阶段 | 内容 | 实际执行 |
|------|------|----------|
| [1/4] 编译器回归 | 构建 + 62 示例 + 60 负例（交互用例跳过） | `bash huzc/test.sh` |
| [2/4] 标准库自测 | 自举标准库自测 | `bash huzi-src/test.sh` |
| [3/4] 格式化门禁 | 检查 `huzc/test/cases` 格式 | `huzc fmt --check huzc/test/cases` |
| [4/4] 性能抽查 | huzi release 相对 Rust -O 不超过 2.0x | `python huzc/test/bench_compare.py` |

任一阶段失败即停，非零退出；全部通过打印门禁汇总。

## 快速开始（新人一次跑通）

```bash
# 1. 构建编译器（cwd=仓库根；或 cd huzc 后 cargo build --workspace）
cargo build --manifest-path huzc/Cargo.toml --workspace

# 2. 一键全部门禁（cwd=仓库根，约数分钟；赶时间可先跳过性能抽查）
bash check.sh
# 或：SKIP_BENCH=1 bash check.sh

# 3. 单独格式化门禁（cwd=仓库根）
./huzc/target/debug/huzc fmt --check huzc/test/cases
# 在 huzc 内执行时路径去掉 huzc/ 前缀：huzc fmt --check test/cases
```

> **Windows 用户**：`check.sh` / `test.sh` 依赖 `bash` / `diff`、可选 `timeout` 与可写 `/tmp`，需在 **Git Bash / MSYS2 / WSL** 中运行，纯 PowerShell 或 cmd 无法直接执行；同时确保 LLVM 的 `llc` / `opt` 与平台链接器已在 `PATH` 中。

## 文档导航

- **用户指南与语法速查**：[`docs/USAGE.md`](docs/USAGE.md)（cwd=`huzc` 视角的编译命令不变，见该文档内“路径基准说明”）
- **编译器工程说明**：[`huzc/README.md`](huzc/README.md)（cwd=`huzc`）
- **项目完成状态**：[`docs/STATUS.md`](docs/STATUS.md)
- **编码与重构约定**：[`AGENTS.md`](AGENTS.md)（cwd=仓库根，含验证流程与提交约定）
- **标准库分层架构**：[`huzi-src/README.md`](huzi-src/README.md)
- **编辑器支持清单**：[`editors/README.md`](editors/README.md)
