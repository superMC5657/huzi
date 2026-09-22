# Huzi 仓库中文入口

Huzi 是一种简洁、强类型、注重人体工学的静态编译型编程语言，语法风格类似 Python，后端基于 LLVM-18 生成高效原生机器代码。

## 模块一览

| 模块 | 路径 | 说明 |
|------|------|------|
| 编译器与工具链 | `huzc/` | 官方参考编译器 |
| 自举标准库 | `huzi-src/` | 用 Huzi 自身编写的标准库 |
| 编辑器支持 | `editors/` | VS Code 插件等编辑器支持 |
| 自举示例 | `examples/` | 用 Huzi 重写的词法器等示例 |
| 项目文档 | `docs/` | 用户手册与技术文档 |

## 快速开始

以下命令都在仓库根执行：

```bash
# 1. 构建编译器
cargo build --manifest-path huzc/Cargo.toml --workspace

# 2. 一键跑完全部门禁（约数分钟；跳过性能抽查用 SKIP_BENCH=1 bash check.sh）
bash check.sh
```

| 阶段 | 内容 |
|------|------|
| [1/4] 编译器回归 | 构建 + 示例与负例测试 |
| [2/4] 标准库自测 | 自举标准库自测 |
| [3/4] 格式化门禁 | 检查示例代码格式 |
| [4/4] 性能抽查 | release 性能相对 Rust -O 不超过 2.0x |

> Windows 用户请在 Git Bash / MSYS2 / WSL 中运行，并确保 LLVM 的 `llc` / `opt` 已在 `PATH` 中。

## 文档导航

- [用户指南与语法速查](docs/USAGE.md)
- [项目完成状态](docs/STATUS.md)
- [编码与重构约定](AGENTS.md)
- [编译器工程说明](huzc/README.md)
- [标准库分层架构](huzi-src/README.md)
- [编辑器支持清单](editors/README.md)
