# Huzi 项目完成状态（STATUS）

> 只回答“做完什么、没做什么”。实现细节见 `docs/USAGE.md`（用户手册）与 `docs/开发文档.md`（技术架构）。
> 更新日期：2026-09-18，分支 `master`，工作树干净。

## 1. 总体结论

* 核心语言 + 编译器后端 + 工具链已闭环，可构建、可发版。
* `TODO §3` 全部 7 项已完成：`defer` / `fmt` / `RC` / `泛型` / `Trait` / `包管理` / `网络并发`。
* 当前在收尾打磨期：最近 3 笔提交为 bug 修复与拆分还债，无新功能在途。

## 2. 已完成清单

### 语言核心

- [x] 基本类型：`i32/i64/f32/f64/bool/char/str`
- [x] 复合类型：数组 `[T;N]`、元组、结构体、枚举（含 payload）、`vec`、`Box<T>`（含嵌套 `Box<Box<T>>`）
- [x] 控制流：`if/elif/else`、`for ..` / `for in`、`while`、`break/continue`、`match`（穷尽检查）、`defer`
- [x] 模块：`import` 相对路径、去重、循环拦截、`mod::fn()` 调用
- [x] 泛型函数 + 泛型结构体（显式实参）
- [x] Trait + impl（静态分发）

### 标准库内置

- [x] I/O：`print/read_line/read_int/read_float/is_eof`
- [x] CLI 参数：`arg_count/arg/arg_ok`
- [x] 字符串：`len/concat/to_string/split/substring/trim/contains` + 下标 + 字典序比较
- [x] 数学：`abs/sqrt/pow/sin/cos/tan/floor/ceil/round`（含 `math::` 前缀）
- [x] 系统：`rand/srand/time/exit/panic/sleep_ms`
- [x] 文件：`read_file/read_file_ok/read_file_err/write_file`
- [x] 内存：`free_str/free_vec/free_box/ref_count`
- [x] HashMap（`str->i32` 特化）：`map_new/put/get/has/remove/len`
- [x] TCP：`tcp_connect/send/recv/close/listen/accept`
- [x] 线程：`spawn/join`

### 编译器与工具链

- [x] 五阶段流水线：Lexer → Parser → CodeGen(LLVM IR) → Verify → Linker
- [x] IR 优化：`--release` / `--opt-level 0-3`
- [x] 调试：`-g/--debug` DWARF，GDB/LLDB 按源码行调试
- [x] 格式化：`huzc fmt [--check]`
- [x] 包管理：`huzi.toml` + `huzc build/add/fetch`（本地 `vendor/` 离线）
- [x] LSP：诊断/悬停/跳转/补全/语义高亮/大纲
- [x] 跨平台：Windows(`lld-link/msvc/mingw`)、Linux/macOS(`clang`)

### 测试与质量

- [x] 单元测试：全 Workspace 覆盖
- [x] 回归：`bash test.sh`（48 示例 + 29 负例 + 47 输出快照，交互示例跳过）
- [x] 构建产物：`target/debug/huzc.exe` 存在
- [x] 规范门禁：单文件 ≤500 行、单函数 ≤70 行、零警告、中文一事一提交

## 3. 明确不做（非缺失，是取舍）

* 无精确 GC（只有 RC，循环引用会漏）
* 泛型无推导、无泛型枚举穷尽、无 `where` 约束、无特化
* 包管理无中心仓库、无 semver 求解、无 lock 传递合并
* 无 UDP/TLS、无 async/协程、无跨线程共享 `vec/map`

## 4. 文档地图（去哪看什么）

| 想知道 | 去哪看 |
|---|---|
| 怎么用语言/编译器选项/内置函数 | `docs/USAGE.md` |
| 架构/流水线/模块职责 | `docs/开发文档.md` |
| 泛型冻结规则 | `docs/rfc_p4_generics.md` |
| 历史规划 7 项的 IN/OUT | `.omo/plans/section3-roadmap-plan.md`（内部） |
| 待办与历史方向 | `TODO.md` |
| 本文件 | 只看状态，不看方法 |
