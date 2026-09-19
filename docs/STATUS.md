# Huzi 项目完成状态（STATUS）

> 只回答“做完什么、没做什么”。实现细节见 `USAGE.md`（用户手册）与 `dev/开发文档.md`（技术架构）。
> 更新日期：2026-09-19，分支 `master`，工作树干净。

## 1. 总体结论

* 核心语言 + 编译器后端 + 工具链已闭环，可构建、可发版。
* 路线图 7 项已全部完成：`defer` / `fmt` / `RC` / `泛型` / `Trait` / `包管理` / `网络并发`（明细见 git 历史中的 `TODO.md`）。
* `Roadmap Q0 / Q1 / Q2` 全部达成：CI 全量门禁、性能基准门禁、边界负例补齐、泛型实参自动推导、循环引用打破示例、vec/map 参数与字段支持、生态目录去误导与手册拆分。
* CI 状态：GitHub Actions workflow 已移除，项目后续**不做 CI**；质量门禁以本地 `bash test.sh`（回归）与 `test/bench_compare.py`（性能基准）手动执行为准。

## 2. 已完成清单

### 语言核心

- [x] 基本类型：`i32/i64/f32/f64/bool/char/str`
- [x] 复合类型：数组 `[T;N]`、元组、结构体、枚举（含 payload）、`vec<T>`、`Map`（支持作为函数形参与结构体字段）、`Box<T>`（含嵌套 `Box<Box<T>>`）
- [x] 控制流：`if/elif/else`、`for ..` / `for in`、`while`、`break/continue`、`match`（穷尽检查）、`defer`（防止嵌套 return/defer）
- [x] 模块：`import` 相对路径、去重、循环拦截、`mod::fn()` 调用
- [x] 泛型函数 + 泛型结构体（支持调用点实参自动推导与显式标注）
- [x] Trait + impl（静态分发与签名类型完备性校验）

### 标准库内置

- [x] I/O：`print/read_line/read_int/read_float/is_eof`
- [x] CLI 参数：`arg_count/arg/arg_ok`
- [x] 字符串：`len/concat/to_string/split/substring/trim/contains/parse_int/parse_float` + 下标 + 字典序比较
- [x] 数学：`abs/sqrt/pow/sin/cos/tan/floor/ceil/round`（含 `math::` 前缀）
- [x] 系统：`rand/srand/time/localtime/env_get/exit/panic/sleep_ms`
- [x] 文件：`read_file/read_file_ok/read_file_err/write_file`
- [x] 内存：`free_str/free_vec/free_box/ref_count`（支持手动打破循环引用）
- [x] HashMap：`map_new/map_put/map_get/map_has/map_remove/map_len/map_keys`
- [x] TCP：`tcp_connect/send/recv/close/listen/accept`
- [x] 线程：`spawn/join`

### 编译器与工具链

- [x] 五阶段流水线：Lexer → Parser → CodeGen(LLVM IR) → Verify → Linker
- [x] 标准库根解析：支持 `HUZI_LIB` 环境变量与相邻 `../huzi-src` 标准库解析
- [x] IR 优化：`--release` / `--opt-level 0-3`
- [x] 调试：`-g/--debug` DWARF，GDB/LLDB 按源码行调试
- [x] 格式化：`huzc fmt [--check]`（AST pretty-printer，幂等性保障）
- [x] 包管理：`huzi.toml` + `huzc build/add/fetch`（本地 `vendor/` 离线）
- [x] LSP：诊断/悬停/跳转/补全/语义高亮/大纲
- [x] 跨平台：Windows(`lld-link/msvc/mingw`)、Linux/macOS(`clang`)

### 测试与质量

- [x] 单元测试：全 Workspace 覆盖，0 警告 0 错误
- [x] 集成回归：`bash test.sh`（55 示例 + 49 负例测试全部通过，交互示例跳过）
- [x] 性能门禁：`test/bench_compare.py`（huzi release / Rust -O <= 2.0x）
- [x] 构建产物：Release 产物三平台自动化归档上传
- [x] 规范门禁：单文件 ≤500 行、单函数 ≤70 行、零警告、中文一事一提交

## 3. 明确不做（非缺失，是取舍）

* 无精确 GC：Box 走引用计数（RC），str/vec 为手动 free + 进程退出 OS 回收；RC 循环引用需手动 free_box 打破
* 泛型无 `where` 约束、无特化、无泛型枚举穷尽
* 包管理无中心仓库、无 semver 求解、无 lock 传递合并
* 无 UDP/TLS、无 async/协程、无跨线程共享 `vec/map`

## 4. 文档地图（去哪看什么）

| 想知道 | 去哪看 |
|---|---|
| 怎么用语言/编译器选项/内置函数 | `USAGE.md` |
| 架构/流水线/模块职责 | `dev/开发文档.md` |
| 泛型冻结规则 | `rfc/rfc_p4_generics.md` |
| 历史规划 7 项的 IN/OUT | `../.omo/plans/section3-roadmap-plan.md`（内部） |
| 本文件 | 只看状态，不看方法 |
