# Huzc — Huzi 编程语言官方编译器

Huzi 是一种简洁、强类型、注重人体工学的静态编译型编程语言，语法风格类似 Python，后端基于 LLVM-18 (inkwell 0.5.0) 生成高效的原生机器代码。

`huzc` 是 Huzi 语言的官方参考编译器与工具链工程。

## 工作区全景（cwd=仓库根；`huzc/` 为当前目录）

```
huzi/
├── AGENTS.md           # 项目级编码与重构约定（cwd=仓库根）
├── docs/               # 项目级文档（USAGE 为入口，余下按主题分类）
│   ├── USAGE.md / STATUS.md
│   ├── guides/       # tutorial.md 教程 + reference.md 参考手册
│   ├── dev/          # 开发文档.md 技术架构
│   └── rfc/          # 泛型设计冻结文档
├── check.sh            # 仓库统一本地门禁入口（四阶段聚合，cwd=仓库根，见下文“一键全部门禁”）
├── huzc/               # 编译器主工程（当前目录，cwd=huzc 执行本文件命令）
│   ├── crates/         # 编译器前端、后端、错误诊断与 LSP 服务
│   ├── test/           # 特性示例、标准输出快照与负例集
│   └── test.sh         # 全自动集成测试与性能基准脚本（cwd=huzc）
├── examples/           # Huzi 自举示例（hzlex/hzparse/task_engine，与编译器仓库同级）
├── editors/            # 编辑器支持与插件
│   ├── README.md       # 编辑器支持矩阵与不支持清单说明
│   └── vscode/         # VS Code 官方语法与 LSP 插件工程
├── huzi-src/           # Huzi 自举标准库源码（core/alloc/std + test 自测）
│   └── README.md       # 标准库分层架构与版本索引
└── .omo/               # 内部计划与会话续跑记录（plans/ + run-continuation/，只看不用、勿删）
```

## 快速开始

### 1. 构建编译器与工具链

```bash
cargo build --workspace
```

### 2. 运行回归测试（cwd=huzc；仓库根请用 `bash huzc/test.sh`）

```bash
bash test.sh
```

> **Windows 用户**：`test.sh` 是 Bash 脚本（依赖 `bash` / `diff`、可选 `timeout` 与可写的 `/tmp`），需在 **Git Bash / MSYS2 / WSL** 中运行，纯 PowerShell 或 cmd 无法直接执行；同时确保 LLVM 的 `llc` / `opt` 与平台链接器已在 `PATH` 中。

> **关于 CI**：本项目不使用 CI（GitHub Actions workflow 已移除，后续也不再引入），回归测试与性能基准均在本地手动执行，不设服务端门禁。

> **一键全部门禁**：仓库根目录的 [`../check.sh`](../check.sh) 聚合本回归脚本、`huzi-src/test.sh`、`fmt --check` 与性能抽查，提交前在根目录运行 `bash check.sh` 即可。

### 3. 代码格式化（cwd=huzc；仓库根请在路径前加 `huzc/`，如 `huzc fmt --check huzc/test/cases`）

```bash
./target/debug/huzc fmt test/cases
./target/debug/huzc fmt --check test/cases
```

## 文档指引（项目级文档已移至仓库根 `docs/`）

- **用户指南与语法速查**：[`../docs/USAGE.md`](../docs/USAGE.md)
- **技术架构与设计文档**：[`../docs/dev/开发文档.md`](../docs/dev/开发文档.md)
- **项目完成状态**：[`../docs/STATUS.md`](../docs/STATUS.md)
- **项目级编码约定**：[`../AGENTS.md`](../AGENTS.md)（cwd=仓库根）
- **编辑器支持清单**：[`../editors/README.md`](../editors/README.md)
- **标准库分层架构**：[`../huzi-src/README.md`](../huzi-src/README.md)
