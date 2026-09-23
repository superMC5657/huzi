# Huzc — Huzi 官方编译器

`huzc` 是 Huzi 语言的官方参考编译器，用 Rust 编写，后端基于 LLVM-18 生成原生机器代码。

## 项目结构

```
huzc/
├── crates/
│   ├── huzc/           # 命令行入口：参数解析、编译编排、链接
│   ├── huzi-lexer/     # 词法分析：源码转 Token 流
│   ├── huzi-parser/    # 语法分析：Token 流转 AST
│   ├── huzi-ast/       # AST 与类型定义
│   ├── huzi-codegen/   # 代码生成：AST 转 LLVM IR（含内置函数与运行时）
│   ├── huzi-error/     # 错误诊断：报错渲染与修复建议
│   └── huzi-lsp/       # 语言服务：诊断、补全、跳转、悬停
├── test/               # 特性示例、输出快照与负例
└── test.sh             # 回归测试脚本
```

## 快速开始

进入 `huzc/` 目录执行：

```bash
cargo build --workspace                            # 构建编译器
./target/debug/huzc run test/cases/01_variables_ops.hz # 一键编译并运行单源文件
./target/debug/huzc test test/cases                # 原生跨平台测试运行器
./target/debug/huzc fmt --check test/cases         # 语法树格式化门禁检查
```

提交前在仓库根跑 `bash check.sh`（或 `SKIP_BENCH=1 bash check.sh`），一次跑完四阶段门禁。

## 文档

- [用户指南]( ../docs/USAGE.md)
- [技术架构](../docs/dev/开发文档.md)
- [项目完成状态](../docs/STATUS.md)
- [编码约定](../AGENTS.md)
