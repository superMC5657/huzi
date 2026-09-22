# Huzi 编程语言 & Huzc 编译器使用指南

## 1. 简介

Huzi 是一种简洁、强类型的静态编译型编程语言，语法风格类似 Python，编译后直接生成高效的原生机器代码。`huzc` 是其官方参考编译器，基于 Rust 开发，后端依托 LLVM-18 (inkwell 0.5.0)。

## 2. 环境要求

- Rust 1.70+
- LLVM 18
- Windows 10/11 (x86_64) / Linux / macOS
- `clang` 或 `lld-link` (用于目标代码链接)

## 3. 快速开始

> **路径基准说明**：本节所有相对路径均以 `huzc/` 目录为工作目录（cwd=`huzc`）；若在仓库根执行，请在 `test/...`、`target/...` 前加 `huzc/` 前缀（例如 `huzc/test/cases`）。

### 3.1 构建编译器

```bash
cargo build --workspace
cargo build --release
```

### 3.2 编译 Huzi 源码

```bash
# 基本用法（cwd=huzc；仓库根请用 huzc/target/debug/huzc --input huzc/test/cases/... -o huzc/test/out/...）
./target/debug/huzc --input <源文件.hz> -o <输出名称>

# 示例：编译示例程序（cwd=huzc）
./target/debug/huzc --input test/cases/01_variables_ops.hz -o test/out/01_variables_ops
```

### 3.3 运行程序

```bash
# Windows（cwd=huzc；仓库根请用 huzc/test/out/01_variables_ops.exe）
./test/out/01_variables_ops.exe

# Linux / macOS（cwd=huzc；仓库根请用 huzc/test/out/01_variables_ops）
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
# 格式化单个文件（cwd=huzc）
huzc fmt path/to/file.hz

# 递归格式化整个目录下的所有 .hz 文件（cwd=huzc；仓库根请用 huzc fmt huzc/test/cases）
huzc fmt test/cases

# 仅检查是否已符合格式（不修改文件，有未格式化文件时退出码为 1）（cwd=huzc；仓库根请用 huzc fmt --check huzc/test/cases）
huzc fmt --check test/cases
```

> **注释保留与归一化范围说明**：
> 当前 `huzc fmt` 为基于 AST 的代码美化器，保证 4 空格缩进、括号与操作符间距的严格归一化与幂等性（二次格式化零 diff）。
> `//` 与 `#` 行注释格式化后原样保留（整行注释按语句回插，行尾注释随语句拼接），详情查阅 [`guides/reference.md`](guides/reference.md)。

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

### 版本语义 (`version` 字段)

依赖的 `version` 字符串原样保留，同时会被解析为版本需求 `VersionReq`；`fetch`/`build` 按全部约束逐包求解最高满足版本（复用 `VersionReq::matches`，多约束需同时满足）：

| 写法 | 含义 | 示例 |
|------|------|------|
| `1.2.3` | 精确相等（默认；裸版本号缺省分量补 0） | `1.2` 即 `1.2.0` 精确匹配 |
| `^1.2.3` | 兼容上限：`>=1.2.3, <2.0.0`；`^0.2.3` → `<0.3.0`；`^0.0.3` → `<0.0.4` | `^1.2.0` 接受 `1.9.0`，拒绝 `2.0.0` |
| `~1.2.3` | 小版本上限：`>=1.2.3, <1.3.0`；`~1` → `>=1.0.0, <2.0.0` | `~1.2.3` 接受 `1.2.9`，拒绝 `1.3.0` |
| `>=1.0.0, <2.0.0` | 范围：逗号/空格分隔的比较符（`>`/`>=`/`<`/`<=`/`=`/`==`）需全部满足 | 接受 `1.5.0`，拒绝 `2.0.0` |
| `*` / `1.*` | 任意版本 / 前缀通配（`1.*` 即 `>=1.0.0, <2.0.0`） | `*` 接受任何版本 |

> **明确不做**：中心仓库、下载校验和、semver 自动升级（只按已有约束选最高满足版，不改写 `huzi.toml`）。

### 传递依赖与冲突

`fetch`/`build` 会读取已选版本目录下的 `huzi.toml`，把传递依赖合并进同一闭包求解：

- 候选来源：`path` 指向目录下的版本子目录（或其 `huzi.toml` 包版本；旧单目录 `path` 按精确原串兼容）；无 `path` 声明则查全局缓存 `~/.huzi/packages/<pkg>/` 与工程 `vendor/<pkg>/` 下的版本子目录。
- 同一包多约束无共同满足版本时直接报错退出（不做自动升级），如菱形依赖两边分别要求 `shared` 的 `1.0.0` 与 `2.0.0`（见 `test/pkg/app_diamond`）。
- `fetch` 按求解结果落盘 `vendor/<pkg>/<version>/`，并清理该包下落选的版本子目录；`build` 先做同一求解检查，缺 `vendor` 落盘则提示先 `fetch`。多版本示例见 `test/pkg/app_multi`（`^1.0.0` 在 `1.0.0`/`1.2.0` 中选 `1.2.0`）。

### 锁定文件 (`huzi.lock`)

`fetch` 求解成功后在工程根写 `huzi.lock`（精确记录逐包选中版本，按包名排序）：

```toml
# huzi.lock 由 `huzc fetch` 生成,请勿手动编辑。
[[package]]
name = "my_math"
version = "1.2.0"
```

`build` 校验：锁存在时必须与本次求解精确一致（版本漂移、缺失、多余条目均直接报错并提示重 `fetch`）；无锁文件时仅做闭包与 `vendor` 检查，兼容旧工程。

### 命令说明
- `huzc build [--path <dir>]`: 依据 `huzi.toml` 编排编译项目并生成可执行文件。
- `huzc add <package> [version] [--path <local_path>]`: 添加依赖并自动同步至本地 `vendor/`。
- `huzc fetch [--path <dir>]`: 拉取/同步所有依赖至本地 `vendor/<pkg>/<version>/` 目录。

> **标准库开箱即用说明**：
> Huzi 官方自举标准库（`huzi-src`）由编译器默认提供并解析，在源码中可直接通过 `import std.*` / `import alloc.*` / `import core.*` 全路径导入，**无需在 `huzi.toml` 的 `[dependencies]` 中额外声明**。

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

## 8. 性能基准 (bench)

`test/bench_compare.py` 对比五边耗时（huzi dev/release + rustc 默认/rustc -O + Python），校验三门禁：结果一致性、release 优于 dev、huzi release / Rust -O <= 2.0x。`test/bench_baseline.txt` 仅存档上次通过门禁的比值，用于漂移提示，不参与门禁判定。

```bash
# 方式一：cwd=huzc 直接跑
python test/bench_compare.py

# 方式二：cwd=huzc 经回归脚本顺带跑
RUN_BENCH=1 bash test.sh

# 方式三：cwd=仓库根 经统一门禁跑（含 [4/4] 性能抽查；仓库根请用 huzc/test/... 路径）
bash check.sh
python huzc/test/bench_compare.py
RUN_BENCH=1 bash huzc/test.sh
```

---

## 9. 文档导航与指引
- **新手与语言教程**：请参阅 [`guides/tutorial.md`](guides/tutorial.md)，涵盖变量、控制流、函数、结构体、枚举匹配、堆指针 Box、泛型及 Trait 接口。
- **全量规范与标准库参考**：请参阅 [`guides/reference.md`](guides/reference.md)，涵盖类型系统、关键字、运算符及全量内置函数（I/O、字符串、数学、文件、网络、多线程并发等）。
- **技术架构与编译器实现**：请参阅 [`dev/开发文档.md`](dev/开发文档.md)，涵盖 LLVM CodeGen、AST、词法语法设计与链接编排。
- **项目状态快照**：请参阅同目录 [`STATUS.md`](STATUS.md)。

---

## 10. 常见问题 (FAQ)

### Q: 编译报错 "Verification failed"
A: 这是 LLVM 模块验证器捕获的 IR 错误，表明存在类型或控制流非法指令，属于编译器内部错误（bug），编译将安全终止。

### Q: 如何指定输出产物目录
A: 使用 `-o` 指定路径即可，中间 `.ll` 和 `.obj` 临时文件在编译成功后会自动清理：
```bash
huzc -i src/main.hz -o build/app
```

### Q: 支持泛型实参推导吗
A: 支持。泛型函数在调用点会根据实参类型自动推导类型参数，如 `id(42)` 会自动推导为 `id<i32>`，无需显式书写 `<i32>`。

### Q: 泛型推导失败的报错怎么看
A: 推导失败会指出调用位置（行列）、期望与实际，例如类型形参无来源时：
```text
Compile error at line 6, column 5: 泛型函数 'zero' 的类型形参 'T' 无法推导:期望由实参确定,实际没有对应推导来源;请显式指定,如 `zero<T>(...)`
```
多实参推导不一致时会对比先后结果：
```text
Compile error at line 6, column 5: 类型形参 'T' 推导冲突:期望各实参推导结果一致,实际先后为 'i32' 与 'str';帮助:统一对应实参类型,或显式写出类型实参
```
按提示统一实参类型，或改写为显式形式（如 `choose<i32>(42, 7)`）即可。

### Q: Trait 缺方法 / 方法冲突的报错怎么看
A: 缺失方法会列出期望的全部方法表，如：
```text
Compile error at line 10, column 1: 类型 'Point' 实现 trait 'Geometry' 缺少方法 'perimeter':期望实现全部 2 个方法 [area, perimeter],实际缺失;请补上 `fn perimeter(...)` 实现
```
同一方法被两个 trait 同时实现时会点名双方并给出改名建议：
```text
Compile error at line 19, column 1: 类型 'Point' 的方法 'show' 冲突:已由 trait 'Printable' 实现,当前 trait 'Displayable' 再次实现;期望每个方法只由一个 trait 提供,实际出现多次;帮助:改名其中一个方法,或通过 impl 归属区分调用
```
方法名拼写接近时会追加 `did you mean` 建议，请按建议检查拼写。

---

## 11. 编辑器 LSP（补全/跳转/语义高亮分级）

`huzi-lsp`（位于 `crates/huzi-lsp`）按四级分级提升补全与跳转精度：

- **L1 保底**：补全三板斧（关键字/同文件符号、`Point.` 字段/`Enum::` 变体/`math::` 函数、未知基通用成员兜底，坏文件仍可补关键字）；跳转（import 行到文件头、`模块::函数` 到模块内 fn 符号，失败回退同文件）；语义高亮（keyword/variable/function/type 图例，`::` 后标识符归 function）。
- **L2 精化**：存量行为只加单测锁定（枚举 `::` 变体、字段前缀过滤、越界永不 panic、空白/非法 import 不跳、`:` 后类型与 `::` 后函数高亮），不改行为。
- **L3 std import 感知**：`import std.json` 可解析到 `huzi-src`（读 `huzi.toml lib_entry`，参照 `modules.rs probe_entry_file`）；补全：`json::` 读模块文件符号、`std::` 读 `std/lib.hz` export 表（✅）；跳转到符号（规划中，见下笔）。
- **L4 Trait/impl 成员**（✅）：`Point.` 补全 impl 方法、`Trait::` 补全 trait 方法（读 `symbols.rs` trait/impl 表）；语义高亮小幅扩展（`export` 归 keyword）。
