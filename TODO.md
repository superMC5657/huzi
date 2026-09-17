# Huzi 编程语言开发状态与规划

## 1. 项目概述

Huzi 是一种简洁、强类型的编译型编程语言，语法风格类似 Python，编译后直接生成高效的原生可执行文件。Huzc 是其官方编译器，采用 Rust 实现，以后端 LLVM-18 (inkwell 0.5.0) 生成目标代码。

当前项目结构：
```
huzc/
├── crates/
│   ├── huzc/           # 编译器 CLI 入口与链接编排
│   ├── huzi-ast/       # 抽象语法树与符号模型
│   ├── huzi-lexer/     # 词法分析器
│   ├── huzi-parser/    # 语法分析器 (语句级容错)
│   ├── huzi-codegen/   # LLVM 代码生成与内置函数
│   ├── huzi-error/     # 错误渲染与相似词建议
│   └── huzi-lsp/       # 语言服务器 (LSP)
├── test/
│   ├── examples/       # 语言特性示例程序
│   ├── expected/       # 预期标准输出快照
│   ├── neg/            # 编译失败与运行失败负例
│   └── out/            # 测试构建输出目录
└── docs/
    ├── USAGE.md        # 用户指南
    └── 开发文档.md      # 技术架构与开发文档
```

---

## 2. 当前功能状态

### 2.1 语言核心特性
- **基本类型**: `i32`、`i64`、`f32`、`f64`、`bool`、`char`、`str`
- **复合数据结构**:
  - 固定长度数组 `[T; N]`：支持字面量、下标读写、越界运行时检查
  - 元组 `(T1, T2, ...)`：支持解构、`.0` 字段访问与嵌套
  - 结构体 `struct Name { ... }`：支持实例化、字段访问与修改、值传递语义、整体打印
  - 枚举 `enum Name { ... }`：支持无参变体与多 payload 变体、`==`/`!=` 相等性比较
  - 动态数组 `vec`：支持 `vec(...)` 自动推导构造与 `vec<T>()` 空构造、`push` 扩容、下标访问与整体打印
  - 堆指针与自引用 `Box<T>`：支持 `box(...)` 堆分配、`null` 空值表示、字段自动解引用、判空与指针比较
- **控制流与模式匹配**:
  - `if` / `elif` / `else` 分支（支持语句与表达式）
  - `for i in start..end` 与 `for x in container` 迭代
  - `while` 循环、`break` / `continue` 跳转
  - `match` 表达式：支持枚举变体匹配、payload 变量绑定与编译期穷尽性检查
- **模块系统**:
  - `import module` 与 `import path.submodule` 相对路径模块加载
  - 循环导入拦截与按路径自动去重
  - 模块限定调用 `module::func()`

### 2.2 标准库内置函数
- **输入输出 (I/O)**: `print`、`read_line`、`read_int`、`read_float`、`is_eof`
- **命令行参数 (CLI Args)**: `arg_count`、`arg`（跨平台统一 UTF-8 参数处理）
- **字符串操作 (String)**: `len`、`concat`、`to_string`、下标访问 `s[i]`、全量字典序比较
- **数学运算 (Math)**: `abs`、`sqrt`、`pow`、`sin`、`cos`、`tan`、`floor`、`ceil`、`round`（及 `math::*`）
- **系统与运行时 (Sys / Runtime)**: `rand`、`srand`、`time`、`exit`、`sleep_ms`
- **文件 I/O (File I/O)**: `read_file`、`write_file`

### 2.3 编译器后端与工程化
- **LLVM 代码生成**: 严谨的类型映射、栈分配优化与控制流生成
- **IR 优化**: 支持 `--opt-level 0..3` 与 `--release`（自动编排 `opt -O2` pass 优化）
- **调试支持**: `-g` / `--debug` 选项生成完整 DWARF 调试元数据，无缝集成 GDB / LLDB 断点单步与变量打印
- **诊断体验**: 准确的行列号定位、源码高亮、带光标行摘录及 Levenshtein 相似词拼写建议
- **语言服务 (LSP)**: `crates/huzi-lsp` 提供语法诊断、悬停签名、跳转定义、成员补全与语义高亮
- **跨平台适配**: 完整支持 Windows (x86_64, `lld-link` / `msvc` / `mingw`)、Linux (x86_64, `clang`)、macOS (x86_64 / arm64, `clang`)
- **质量保证**: 全 Workspace 单元测试全覆盖 + `test.sh` 驱动的全量集成回归与预期输出快照校验

---

## 3. 未来扩展规划 (Roadmap)

当前语言核心功能与编译器基础已全部闭环稳定。以下为未来演进的可选方向：

### 3.1 内存管理进阶
- [x] 为堆分配（`Box` / `vec` 缓冲区）引入引用计数 (RC) 或精确垃圾收集 (GC)
- [x] 作用域显式资源释放机制（`defer` 语句）

### 3.2 类型系统扩展
- [x] 用户自定义泛型函数与泛型结构体（目前泛型仅内置于 `Box<T>` 与 `vec<T>()`）
- [x] 接口 / Trait 多态抽象机制

### 3.3 生态与工具链
- [x] 包管理器与外部依赖解析工具
- [x] 静态代码格式化工具 (formatter)
- [x] 更多标准库内置功能（网络套接字、并发协程/线程支持）
