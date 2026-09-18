# Huzi 编程语言 & Huzc 编译器使用指南

## 1. 简介

Huzi 是一种简洁、强类型的静态编译型编程语言，语法风格类似 Python，编译后直接生成高效的原生机器代码。`huzc` 是其官方参考编译器，基于 Rust 开发，后端依托 LLVM-18 (inkwell 0.5.0)。

## 2. 环境要求

- Rust 1.70+
- LLVM 18
- Windows 10/11 (x86_64) / Linux / macOS
- `clang` 或 `lld-link` (用于目标代码链接)

## 3. 快速开始

### 3.1 构建编译器

```bash
cargo build --workspace
cargo build --release
```

### 3.2 编译 Huzi 源码

```bash
# 基本用法
./target/debug/huzc --input <源文件.hz> -o <输出名称>

# 示例：编译示例程序
./target/debug/huzc --input test/examples/01_variables_ops.hz -o test/out/01_variables_ops
```

### 3.3 运行程序

```bash
# Windows
./test/out/01_variables_ops.exe

# Linux / macOS
./test/out/01_variables_ops
```

---

## 4. 编译器选项

| 选项 | 简写 | 说明 | 示例 |
|------|------|------|------|
| `--input <file>` | `-i` | 输入的 `.hz` 源文件 | `-i main.hz` |
| `-o <name>` | | 指定输出文件基础名（自动补齐平台后缀） | `-o build/app` |
| `--release` | `-r` | 生产模式：启用 `opt -O2` IR 优化 | `-r` |
| `--opt-level <0-3>` | | 指定 LLVM 优化级别 (0-3) | `--opt-level 3` |
| `--debug` | `-g` | 嵌入 DWARF 调试符号并保持 `-O0` | `-g` |
| `--linker <name>` | `-l` | 指定底层链接器 (`msvc`/`lld-link`/`mingw`/`clang`) | `-l mingw` |

---

## 5. 代码格式化 (huzc fmt)

Huzc 内置轻量级代码格式化工具，基于 AST 语法树 pretty-print 机制实现严格幂等的代码排版：

```bash
# 格式化单个文件
huzc fmt path/to/file.hz

# 递归格式化整个目录下的所有 .hz 文件
huzc fmt test/examples

# 仅检查是否已符合格式（不修改文件，有未格式化文件时退出码为 1）
huzc fmt --check test/examples
```

> **注释保留与归一化范围说明**：
> 当前 `huzc fmt` 为基于 AST 的代码美化器，保证 4 空格缩进、括号与操作符间距的严格归一化与幂等性（二次格式化零 diff）。
> 由于当前 AST 未捕获行级注释，格式化操作会剥除源码中的注释。若需完整保留注释，建议手动排版或查阅 [`guides/reference.md`](guides/reference.md) 了解详情。

---

## 6. 包管理器与项目清单 (huzi.toml)

Huzc 内置轻量级包管理支持，支持依赖本地解析与 `vendor/` 离线隔离：

### 清单格式 (`huzi.toml`)
```toml
[package]
name = "my_app"
version = "0.1.0"
entry = "src/main.hz" # 可选，缺省自动探测入口

[dependencies]
my_math = { version = "1.0.0", path = "../fixtures/my_math" }
```

### 命令说明
- `huzc build [--path <dir>]`: 依据 `huzi.toml` 编排编译项目并生成可执行文件。
- `huzc add <package> [version] [--path <local_path>]`: 添加依赖并自动同步至本地 `vendor/`。
- `huzc fetch [--path <dir>]`: 拉取/同步所有依赖至本地 `vendor/<pkg>/<version>/` 目录。

> **标准库开箱即用说明**：
> Huzi 官方自举标准库（`huzi-src`）由编译器默认提供并解析，在源码中可直接通过 `import std.*` / `import alloc.*` / `import core.*` 全路径导入，**无需在 `huzi.toml` 的 `[dependencies]` 中额外声明**。
> 完整的工程化多模块、特性组合与标准库调用示例，请参阅根目录下的 [`examples/`](../../examples/README.md)。

---

## 7. 源码级调试 (-g)

使用 `-g` 编译的产物包含完整 DWARF 调试元数据，无缝集成 GDB 与 LLDB：

```bash
huzc -g -i main.hz -o main
gdb ./main
(gdb) break main.hz:10     # 按源码行设置断点
(gdb) run                  # 启动执行
(gdb) print x              # 打印局部变量
(gdb) next                 # 单步跳过
```

---

## 8. 文档导航与指引

- **新手与语言教程**：请参阅 [`guides/tutorial.md`](guides/tutorial.md)，涵盖变量、控制流、函数、结构体、枚举匹配、堆指针 Box、泛型及 Trait 接口。
- **全量规范与标准库参考**：请参阅 [`guides/reference.md`](guides/reference.md)，涵盖类型系统、关键字、运算符及全量内置函数（I/O、字符串、数学、文件、网络、多线程并发等）。
- **技术架构与编译器实现**：请参阅 [`dev/开发文档.md`](dev/开发文档.md)，涵盖 LLVM CodeGen、AST、词法语法设计与链接编排。
- **项目状态快照**：请参阅同目录 [`STATUS.md`](STATUS.md)。

---

## 9. 常见问题 (FAQ)

### Q: 编译报错 "Verification failed"
A: 这是 LLVM 模块验证器捕获的 IR 错误，表明存在类型或控制流非法指令，属于编译器内部错误（bug），编译将安全终止。

### Q: 如何指定输出产物目录
A: 使用 `-o` 指定路径即可，中间 `.ll` 和 `.obj` 临时文件在编译成功后会自动清理：
```bash
huzc -i src/main.hz -o build/app
```

### Q: 支持泛型实参推导吗
A: 支持。泛型函数在调用点会根据实参类型自动推导类型参数，如 `id(42)` 会自动推导为 `id<i32>`，无需显式书写 `<i32>`。
