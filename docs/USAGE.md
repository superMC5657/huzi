# Huzi 编程语言 & Huzc 编译器使用指南

> 本文档位于仓库根 `docs/`（项目级文档）。第 3-8 节的编译命令仍以 `huzc/` 为工作目录（cwd=`huzc`），与文档位置无关。

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

### 3.3 一键编译并运行 (huzc run)

Huzc 支持通过 `run` 子命令一键编译并立即启动可执行文件（支持单源文件或工程目录，`--` 后的参数将透明透传给程序）：

```bash
# 直接运行单源文件（cwd=huzc）
huzc run test/cases/01_variables_ops.hz

# 运行工程目录并透传参数
huzc run examples/task_engine
huzc run --path examples/task_engine -- --verbose

# 发布模式优化运行
huzc run -r test/cases/01_variables_ops.hz
```

### 3.4 原生测试运行器 (huzc test)

Huzc 内置原生跨平台测试运行器，无需依赖 bash、timeout 或 diff，支持自动收集用例、预期输出比对、负例校验（`*.compile_fail.hz` 与 `*.runtime_fail.hz`）以及名称过滤：

```bash
# 运行指定测试目录（cwd=huzc；仓库根请用 huzc test huzc/test/cases 或 huzc test huzi-src/test/cases）
huzc test test/cases
huzc test ../huzi-src/test/cases

# 运行单个测试用例
huzc test test/cases/01_variables_ops.hz

# 过滤指定名称测试（支持子串匹配）
huzc test test/cases --filter compound

# 发布模式运行测试
huzc test test/cases -r
```

### 3.5 独立运行可执行程序

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
my_json = { version = "^1.0.0", registry = "file:///opt/hz-registry" }
"acme/json" = { version = "1.0.0", registry = "https://hz.example.com" }
```

- `registry`：可选注册表源 URL，仅 `file://` 与 `https://` 合法，其余/空串/未知键解析直接 `Err`；含 `/` 的作用域全名落盘/解析原样引号包住。
- 同一依赖同时写 `path` 与 `registry` 时 `path` 胜（求解只看 `path`，不读索引）。

### 版本语义 (`version` 字段)

依赖的 `version` 字符串原样保留，同时会被解析为版本需求 `VersionReq`；`fetch`/`build` 按全部约束逐包求解最高满足版本（复用 `VersionReq::matches`，多约束需同时满足）：

| 写法 | 含义 | 示例 |
|------|------|------|
| `1.2.3` | 精确相等（默认；裸版本号缺省分量补 0） | `1.2` 即 `1.2.0` 精确匹配 |
| `^1.2.3` | 兼容上限：`>=1.2.3, <2.0.0`；`^0.2.3` → `<0.3.0`；`^0.0.3` → `<0.0.4` | `^1.2.0` 接受 `1.9.0`，拒绝 `2.0.0` |
| `~1.2.3` | 小版本上限：`>=1.2.3, <1.3.0`；`~1` → `>=1.0.0, <2.0.0` | `~1.2.3` 接受 `1.2.9`，拒绝 `1.3.0` |
| `>=1.0.0, <2.0.0` | 范围：逗号/空格分隔的比较符（`>`/`>=`/`<`/`<=`/`=`/`==`）需全部满足 | 接受 `1.5.0`，拒绝 `2.0.0` |
| `*` / `1.*` | 任意版本 / 前缀通配（`1.*` 即 `>=1.0.0, <2.0.0`） | `*` 接受任何版本 |

> **当前快照（M4）**：私有源最小读路径已点亮（`file://` + `https` 只读索引，`HUZI_REGISTRY` 默认源，`fetch` 下载验哈希落盘，`build` 纯离线）；无自动升级（只按已有约束选最高满足版，不改写 `huzi.toml`）；`vendor/` 目录哈希校验已点亮（见下）。OUT 见下（publish/服务端/yank/预发布/TOFU/签名）。演进方向见 `docs/rfc/rfc_center_registry_draft.md`（草案；升级若做也只会是显式命令，不会是 `build` 隐式行为）。

### 传递依赖与冲突

`fetch`/`build` 会读取已选版本目录下的 `huzi.toml`，把传递依赖合并进同一闭包求解：

- 源优先级（显式）：`path` > `registry=` 声明 > `HUZI_REGISTRY` 环境默认源 > 本地（工程 `vendor/<pkg>/` + 全局缓存 `~/.huzi/packages/<pkg>/`）。同名跨源 `path` 胜（有可行 `path` 只看 `path`，锁记 `source="path"`）；`registry` 声明源逐个试（错直接抛），环境源缺索引回退本地。
- 候选来源：`path` 指向目录下的版本子目录（或其 `huzi.toml` 包版本；旧单目录 `path` 按精确原串兼容）；`registry` 经只读索引 `<registry>/<pkg>/index.toml` 选版（手写解析版本列表 + 每版 `sha256` + `tarball` URL，不引 toml 库）；本地为版本子目录。
- 同一包多约束无共同满足版本时直接报错退出（不做自动升级），如菱形依赖两边分别要求 `shared` 的 `1.0.0` 与 `2.0.0`（见 `test/pkg/app_diamond`）。
- `fetch` 按求解结果落盘 `vendor/<pkg>/<version>/`（含 `/` 全名即 `vendor/<scope>/<name>/<version>/`），并清理该包下落选的版本子目录；`build` 纯离线先做同一求解检查（`offline=true`，本地有可行版直接用本地，缺版才报 `--offline` 错），缺 `vendor` 落盘则提示先 `fetch`。多版本示例见 `test/pkg/app_multi`（`^1.0.0` 在 `1.0.0`/`1.2.0` 中选 `1.2.0`）。
- 内置保留：`std`/`core`/`alloc` 及 `std/*` 等命中注册表在求解期直接拒绝（提示走内置 `import`，无需声明）。

### 私有源与索引格式（M4 最小读路径）

- 默认源：`HUZI_REGISTRY` 环境变量（空串视为未设）；声明 `registry=` 优先于它。
- 索引：只读 `<registry>/<pkg>/index.toml`（`file://` 直读文件，`https://` 经 `curl -fsSL` 读，零新依赖）；`--offline` 下读索引直接错。
- `index.toml` 手写格式（未知键忽略）：
```toml
[[package]]
version = "1.0.0"
sha256 = "<tarball文件字节sha256,64位小写hex,允许sha256:前缀>"
tarball = "tarballs/my_math-1.0.0.tar.gz" # 相对路径相对 <registry>/<pkg>/ 解析，否则须为 file:///https:// 绝对 URL
```
- `fetch` 流程（复用 M2 原子时序）：读索引 → 下载 tarball → 验 `sha256(文件字节)` → 落盘 `vendor/`（`file://` 目录直拷；`.tar.gz/.tgz` 经系统 `tar -xzf` 解包，唯一顶层含 `huzi.toml` 时下沉一层）→ 回填目录哈希 → 全成功后清理落选版本并写锁。任一步失败直接退出，不删 `vendor/`、不写锁。
- 认证（v1 最小可用，明文）：URL 自带（`https://<token>@host/...` 或 `?token=<t>`）优先，否则读 `~/.huzi/credentials`（每行 `<registry> [=] <token>`，`#` 注释），经 `curl -H "Authorization: Bearer <token>"` 携带。
- OUT（锁死不做）：`publish` 命令、服务端 API、`yank` enforcement、预发布（`SemVersion` 不动，索引遇 `-alpha` 报“暂不支持预发布”错）、TOFU pin、签名验签。

### 锁定文件 (`huzi.lock`)

`fetch` 求解成功后在工程根写 `huzi.lock`（精确记录逐包选中版本，按包名排序）：

```toml
# huzi.lock 由 `huzc fetch` 生成,请勿手动编辑。
[[package]]
name = "my_math"
version = "1.2.0"
source = "path"
checksum = "sha256:9f2c…(64位小写hex)"
```

- `source`：该包来源种类（`path` 显式本地路径 / `local` 全局缓存与 `vendor/` / `registry` 注册表索引，`fetch` 按优先级落盘，跨源 `path` 胜）；旧锁缺字段解析为 `None`，兼容。
- `registry`：注册表包的源 URL（声明 `registry=` 或 `HUZI_REGISTRY`，`fetch` 回填；`path`/`local` 包为 `None`）；旧锁缺字段为 `None`，兼容。
- `checksum`：`fetch` 落盘后回填的 `vendor/<pkg>/<version>/` 目录内容哈希（`sha256:<64位小写hex>`，非法格式解析直接报错）；旧锁缺字段解析为 `None`，兼容。
- 目录哈希口径：递归收集全部文件，相对路径（`/` 分隔）排序后逐文件取 `sha256(文件字节)` hex，再外层 `sha256(拼接(relpath + 0x00 + hex + \n))`；文件一律按字节读，不做 CRLF 归一，全程零网络。
- `fetch` 原子性：逐包落盘并算哈希全部成功后，才清理落选版本子目录并写锁；任一步失败直接退出，不删 `vendor/`、不写锁。

`build` 离线校验（按序，失败直接报错退出，不访问网络）：锁一致性（版本漂移、缺失、多余条目均提示重 `fetch`）→ `vendor` 落盘存在 → 目录哈希（锁中有 `checksum` 的包重算比对，篡改一字节即硬错 `vendor内容与huzi.lock不一致，请重fetch`）；无锁/旧锁无 `checksum` 时跳过对应步骤，兼容旧工程。

### 命令说明
- `huzc run [<target>] [--path <dir>] [-- <args>...]`: 编译并立即执行单源文件或包含 `huzi.toml` 的项目工程，透明透传命令行参数。
- `huzc build [--path <dir>]`: 依据 `huzi.toml` 编排编译项目并生成可执行文件（语义零变化：菱形冲突仍直接报错，不自动升级；锁漂移仍提示重 `fetch`）。
- `huzc add <package> [version] [--path <local_path>]`: 添加依赖并自动同步至本地 `vendor/`。
- `huzc fetch [--path <dir>] [--offline] [--frozen]`: 拉取/同步所有依赖至本地 `vendor/<pkg>/<version>/` 目录（含 `/` 全名即 `vendor/<scope>/<name>/<version>/`）。
  - `--offline`：拒读索引/拒下载（需注册表的包无本地可行版直接错 `--offline拒绝读索引`；本地有可行版则回退本地，`build` 即按此离线重放）。
  - `--frozen`：锁必须已存在且与求解一致，一致则直接成功（不动 `vendor/`、不写锁），缺锁/漂移直接报错；与 `--offline` 互斥，不能同时给。
- `huzc update [package] [--dry-run] [--path <dir>]`: 显式重解闭包并刷新锁定（默认不动语义）。
  - 默认（无变化）：打印 `all up to date`，exit 0，不碰 `vendor/`/`huzi.lock`/`huzi.toml`。
  - `--dry-run`：只读矩阵预览（列：包 | 约束 | 锁版本 -> 候选版本 | source | checksum 短 hash 旧 -> 新），全程只读，不写盘。
  - 显式重锁：有变化且非 `--dry-run` 时落盘 `vendor/<pkg>/<version>/` + 回填校验和 + 清理落选版本 + 写锁（复用 `fetch` M2 时序，先完整求解成功再写）；不改写 `huzi.toml` 版本串。
  - 菱形冲突（如 `test/pkg/app_diamond` 两边分要 `shared` 的 `1.0.0`/`2.0.0`）：`--dry-run` 与显式重锁双路径均沿用求解原文案直接报错（exit 非零），零副作用（不建锁、不建 `vendor/`）。

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

# 方式三：cwd=仓库根 经统一门禁跑（含 [4/5] 性能抽查；仓库根请用 huzc/test/... 路径）
bash check.sh
python huzc/test/bench_compare.py
RUN_BENCH=1 bash huzc/test.sh
```

---

## 9. 文档导航与指引
- **新手与语言教程**：请参阅 [`guides/tutorial.md`](guides/tutorial.md)，涵盖变量、控制流、函数、动态数组 `vec<T>`、确定性自动 Drop 内存管理、结构体、枚举匹配、堆指针 Box、泛型及 Trait 接口。
- **全量规范与标准库参考**：请参阅 [`guides/reference.md`](guides/reference.md)，涵盖类型系统、关键字、运算符、固有方法/UFCS、复合赋值、字符串插值及全量标准库（I/O、字符串、数学、文件、网络、多线程、子进程 `std.process`、路径 `std.path` 等）。
- **技术架构与编译器实现**：请参阅 [`dev/开发文档.md`](dev/开发文档.md)，涵盖 LLVM CodeGen、RAII Drop 析构引擎、AST、词法语法设计与链接编排。
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
- **L3 std import 感知**（✅）：`import std.json` 可解析到 `huzi-src`（读 `huzi.toml lib_entry`，参照 `modules.rs probe_entry_file`）；补全：`json::` 读模块文件符号、`std::` 读 `std/lib.hz` export 表；跳转：`resolve_import_uri` 收敛共享探查，`模块::符号` 按 fn→类型→任意落点。
- **L4 Trait/impl 成员**（✅）：`Point.` 补全 impl 方法、`Trait::` 补全 trait 方法（读 `symbols.rs` trait/impl 表）；语义高亮小幅扩展（`export` 归 keyword）。
