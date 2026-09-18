# Huzc — Huzi 编程语言官方编译器

Huzi 是一种简洁、强类型、注重人体工学的静态编译型编程语言，语法风格类似 Python，后端基于 LLVM-18 (inkwell 0.5.0) 生成高效的原生机器代码。

`huzc` 是 Huzi 语言的官方参考编译器与工具链工程。

## 工作区全景

```
huzi/
├── huzc/               # 编译器主工程（当前目录）
│   ├── crates/         # 编译器前端、后端、错误诊断与 LSP 服务
│   ├── docs/           # 用户手册 (USAGE.md)、教程与技术架构
│   ├── test/           # 特性示例、标准输出快照与负例集
│   └── test.sh         # 全自动集成测试与性能基准脚本
├── editors/            # 编辑器支持与插件
│   ├── README.md       # 编辑器支持矩阵与不支持清单说明
│   └── vscode/         # VS Code 官方语法与 LSP 插件工程
└── huzi-src/           # 规划中的纯 Huzi 自主实现标准库源码目录
    ├── README.md       # 标准库源码规划定位说明
    └── sample.hz       # 示例标准库源码
```

## 快速开始

### 1. 构建编译器与工具链

```bash
cargo build --workspace
```

### 2. 运行回归测试

```bash
bash test.sh
```

### 3. 代码格式化

```bash
./target/debug/huzc fmt test/examples
./target/debug/huzc fmt --check test/examples
```

## 文档指引

- **用户指南与语法速查**：[`docs/USAGE.md`](docs/USAGE.md)
- **技术架构与设计文档**：[`docs/开发文档.md`](docs/开发文档.md)
- **项目完成状态**：[`STATUS.md`](STATUS.md)
- **编辑器支持清单**：[`../editors/README.md`](../editors/README.md)
- **标准库源码规划**：[`../huzi-src/README.md`](../huzi-src/README.md)
