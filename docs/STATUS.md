# Huzi 项目完成状态（STATUS）

> 只回答“做完什么、没做什么”。实现细节见 `USAGE.md`（用户手册）与 `dev/开发文档.md`（技术架构）。
> 更新日期：2026-09-29，分支 `master`。

## 1. 总体结论

* 核心语言 + 编译器后端 + 工具链已闭环，可构建、可发版。
* 路线图 7 项已全部完成：`defer` / `fmt` / `RC` / `泛型` / `Trait` / `包管理` / `网络并发`（明细见 git 历史中的 `TODO.md`）。
* `Roadmap Q0 / Q1 / Q2` 全部达成：本地全量门禁、性能基准门禁、边界负例补齐、泛型实参自动推导、循环引用打破示例、vec/map 参数与字段支持、生态目录去误导与手册拆分。
* 内存管理终极演进：落地 Swift 风格自动置零弱引用（`weak Box<T>`），双计数 16 字节头部布局，强引用析构后弱引用自动置零（Auto-Zeroing），根本性杜绝循环引用泄漏。
* CI 状态：GitHub Actions workflow 已移除，项目后续**不做 CI**；质量门禁以本地门禁为准：cwd=`huzc` 执行 `bash test.sh`（回归）与 `test/bench_compare.py`（性能基准），或 cwd=仓库根执行 `bash check.sh` / `bash huzc/test.sh`（路径前加 `huzc/` 前缀）。

## 2. 已完成清单

### 语言核心

- [x] 基本类型：`i32/i64/f32/f64/bool/char/str`
- [x] 复合类型：数组 `[T;N]`、元组、结构体、枚举（含 payload）、`vec<T>`、`Map`/`Map<str, str>`/`Map<i32, i32>`（支持作为函数形参与结构体字段）、`Box<T>`（含嵌套 `Box<Box<T>>`；`T` 可为结构体或 `i32/i64/f64/bool/str` 标量，标量经前缀 `*b` 解引用读写）
- [x] 自动置零弱引用（Swift 风格 `weak Box<T>`）：双计数 16 字节头部布局（`[-16..-8]` 弱引用计数，`[-8..0]` 强引用计数）；支持 `weak field: Box<T>` 字段与 `let mut w: weak Box<T>` 局部变量；强引用析构后弱引用读取时自动置零（Auto-Zeroing 返回 `null`）；两阶段物理销毁（强引用清零释放隐式弱计数，弱引用清零释放底层 16 字节外壳）；从根本上打破父子节点、双向链表与树形结构循环引用，100% 自动确定性回收且零性能倒退（用例 `74_weak_reference`）
- [x] 控制流：`if/elif/else`、`for ..` / `for in`、`while`、`break/continue`、`match`（穷尽检查、守卫支持）、`defer`（防止嵌套 return/defer）
- [x] 模块：`import` 相对路径、去重、循环拦截、`mod::fn()` 调用
- [x] 泛型系统：泛型函数 + 泛型结构体 + 泛型枚举（支持调用点实参自动推导、变体显式标注与模式匹配解构）
- [x] Trait + impl（静态分发与签名类型完备性校验）
- [x] 错误处理：泛型 `Result<T>`（自举标准库 `core.result`）+ 后缀 `?` 运算符（成功解包 value、失败提前返回整个 Result，defer 照常执行；规则见 `rfc/rfc_result_question.md`）
- [x] 模块内泛型可用：库模块可定义泛型结构体/函数，用户代码经 `mod::gen_fn(...)` 限定调用自动单态化（修复模板泄漏/签名注册顺序/限定调用单态化三处缺口）
- [x] 闭包与高阶函数：匿名函数语法（`|x, y| expr` / `|x| { ... }` / `|| expr`）、按值环境捕获、一等函数类型（`fn(T1, T2) -> Ret`）、命名函数自动升格为闭包 thunk、标准库高阶原语 `vec_map` / `vec_filter` / `vec_fold`（用例 `64_closures`，负例 `closure_arity_mismatch`）
- [x] 固有方法块（Inherent `impl` blocks）与 UFCS（统一函数调用语法糖）：结构体可直接声明固有方法块 `impl TypeName { ... }` 而无需预先声明 Trait；支持 `receiver.method(...)` 多阶分派至固有方法、向量专有前缀（如 `vec_map`）、内置操作（`len`, `trim`, `contains`, `push`, `pop` 等）及任意同名顶层自由函数；原生支持流畅链式调用（用例 `65_inherent_impl_ufcs`，负例 `inherent_conflict`, `inherent_no_such_method`）
- [x] 核心枚举与结构体流式固有方法：`Option<T>` 现代泛型枚举（`Some(T)` / `None`）及其固有方法（`is_some`, `is_none`, `unwrap`, `unwrap_or`, `map`, `and_then`）；`Result<T>` 固有方法（`is_ok`, `is_err`, `unwrap`, `unwrap_or`, `err_msg`），与 `?` 运算符 100% 互通（用例 `66_fluent_option_result_vec`）
- [x] 集合高阶流式管道（自举标准库 `alloc.vec_algo`）：`vec_any` / `vec_all` / `vec_count` / `vec_take` / `vec_skip` / `vec_for_each`，配合 UFCS 支持链式流水线操作（`v.filter(...).take(2).map(...)`）
- [x] 模式匹配全面强化：支持数值/浮点/布尔/字符/字符串字面量模式；支持变量绑定模式（`x => ...`）；支持匹配守卫（`pattern if condition => body`）；支持多分支同变体/同模式条件分流；严格校验守卫条件布尔类型与类型兼容性；完备穷尽性检查（用例 `67_match_guards_literals`，负例集 `match_guard_non_bool`, `match_guarded_wildcard_non_exhaustive`, `match_literal_type_mismatch`, `match_scalar_non_exhaustive`）
- [x] 元组解构绑定：支持 `let (a, b) = expr`、局部独立可变性 `let (mut a, b)`、整组继承 `let mut (a, b)`、嵌套解构 `let (a, (b, c))` 以及通配忽略 `_`（用例 `68_tuple_destructuring`，负例 `tuple_destruct_arity_mismatch`, `tuple_destruct_non_tuple`）
- [x] 固有静态方法与 Self 别名：支持在 `impl TypeName` 块中定义首参数无 `self` 的关联静态方法（如 `Type::new(...)`、工厂方法 `Type::origin()`）；支持在 `impl` 块的方法签名（参数与 `-> Self`）以及方法体内（`Self { ... }`）使用 `Self` 作为目标类型的别名；静态分发至修饰名顶层函数并支持后续实例方法链式调用（用例 `69_static_methods`，负例 `inherent_no_such_static_method`）
- [x] 字符串插值与格式化：内置函数 `format("模板", args...)` 与 `f"..."` 语法糖；支持 `{}` 占位符、`{{` 和 `}}` 大括号转义、自动调用 `to_string`、表达式内联与链式调用；编译期参数完备性检查（用例 `70_string_format_interpolation`，负例 `format_arity_mismatch`）
- [x] 复合赋值运算符：支持 `+=`, `-=`, `*=`, `/=`, `%=`；支持变量标识符、数组索引（地址单次求值）、结构体字段与 Box 前缀解引用；支持整型与浮点数隐式拓宽（用例 `71_compound_assign`）

### 标准库内置

- [x] I/O：`print/read_line/read_int/read_float/is_eof`
- [x] CLI 参数：`arg_count/arg/arg_ok`
- [x] 字符串：`format/len/concat/to_string/split/substring/trim/contains/parse_int/parse_float` + 下标 + 字典序比较
- [x] 数学：`abs/sqrt/pow/sin/cos/tan/floor/ceil/round`（含 `math::` 前缀）
- [x] 系统：`rand/srand/time/localtime/env_get/exit/panic/sleep_ms/process_run`
- [x] 子进程与管道：内置原语 `process_run(cmd) -> (i32, str)` 与标准库 `std.process`（`Command` builder 模式、`Output` 解析，用例 `72_process_run`，标准库自测 `process_test`）
- [x] 跨平台路径处理：自举标准库 `std.path`（`is_sep`, `is_abs`, `path_join`, `base`, `dir`, `ext`, `stem`, `normalize`，全面兼容 Windows 与 POSIX 风格路径，标准库自测 `path_test`）
- [x] 文件：`read_file/read_file_ok/read_file_err/write_file`
- [x] 自动内存管理与双计数弱引用 (RAII Drop + Swift-style Weak)：`vec<T>` 与 `Box<T>` 出作用域、循环迭代、函数返回或 `?` 报错短路时确定性全自动释放堆内存，底层统一 16 字节双计数头部布局零 GC 停顿；内置 `weak_count(b)` 查询弱引用计数；兼容显式 `free_vec/free_box`（正例 `73_auto_drop_vec`, `74_weak_reference`）
- [x] 内存原语：`free_str/free_vec/free_box/ref_count/weak_count`（浅释放语义、二次 `free` 为 no-op；`ref_count`/`weak_count` 快照做引用计数诊断；负例 `rc_cycle_leak`；类型不匹配编译期拒绝，负例 `free_str_non_str`）
- [x] HashMap：`map_new/map_put/map_get/map_has/map_remove/map_len/map_keys`
- [x] TCP：`tcp_connect/send/recv/close/listen/accept`（实参个数/类型不匹配编译期拒绝，负例 `tcp_send_arity`）
- [x] 线程：`spawn/join`（句柄类型不匹配编译期拒绝，负例 `join_bad_type`）
- [x] 通道：`chan_new/chan_send/chan_recv`（跨线程传 str 消息，环形缓冲 + 自旋锁，句柄可作 spawn 实参；非法句柄类型编译期拒绝，负例 `chan_send_bad_handle`）
- [x] HTTP 客户端（自举标准库 `std.http`，基于 TCP）：`http_build_request/http_build_request_with_headers/http_status_code/http_body/http_parse/http_get/http_request/http_post`（POST + 自定义头 + 本地闭环覆盖 POST/404/空体），非 2xx 经 `http_parse` 返回 `Err`（负例 `http_status_non2xx`）
- [x] `for x in 调用(...)`：直接遍历返回 `vec<T>` 的函数调用结果（含 `split` 与模块函数），无需先存变量
- [x] 右值字段访问：`f(...).0`、`g(...).field` 对调用结果直接取元组/结构体字段
- [x] `str_from_bytes(vec<i32>) -> str`：字节向量构造字符串（自举标准库 UTF-8 编码的底层支撑）
- [x] let 元组字段元素类型推断：`let kinds = r.1`（元组右值字段取 vec）后可直接索引/遍历
- [x] 自举一期里程碑：`examples/hzlex` 用 Huzi 重写词法器，selftest 断言全过，全语料词法分析零失败
- [x] 自举二期里程碑：`examples/hzparse` 用 Huzi 重写递归下降解析器与语法分析器，120 个源码文件全语料解析通过率 100%
- [x] 自举三期里程碑：`examples/hzast` 纯 Huzi 抽象语法树（AST）与 Tree-Walking 解释求值器原型，闭环自测覆盖算术表达式树、变量环境绑定与修改、条件分支、While 循环累加、递归函数调用（阶乘/斐波那契）及字符串操作
- [x] 字符级 UTF-8 API（自举标准库 `alloc::stringx`）：`char_len/chars/char_at/char_sub`，中英文混排按"字"计数、遍历与截取（示例 `57_unicode_string`）

### 编译器与工具链

- [x] 五阶段流水线：Lexer → Parser → CodeGen(LLVM IR) → Verify → Linker
- [x] 标准库根解析：支持 `HUZI_LIB` 环境变量与相邻 `../huzi-src` 标准库解析
- [x] IR 优化：`--release` / `--opt-level 0-3`
- [x] 调试：`-g/--debug` DWARF，GDB/LLDB 按源码行调试
- [x] 格式化：`huzc fmt [--check]`（AST pretty-printer，幂等性保障）
- [x] fmt 保留注释：`//` 与 `#` 行注释格式化后原样保留（整行注释按语句回插，行尾注释随语句拼接），`fmt --check test/cases`（cwd=`huzc`；仓库根请用 `fmt --check huzc/test/cases`）门禁恢复可用
- [x] 包管理：`huzi.toml` + `huzc build/add/fetch`（本地 `vendor/` 离线）+ `VersionReq` 范围解析（`^`/`~`/`>=`范围/`*`）与最高满足求解 + 传递依赖合并（冲突直接报错，不做自动升级）+ `huzi.lock` 精确锁定与一致性校验
- [x] 一键运行：`huzc run` 一键编译并立即执行单源文件或工程目录，无缝透传命令行参数
- [x] 原生测试运行器：`huzc test [path] [--filter] [-r]` 原生跨平台测试运行器，脱离 bash/diff/timeout，自动收集、比对输出与负例校验
- [x] LSP：诊断/悬停/跳转/补全/语义高亮/大纲/文档格式化（`textDocument/formatting` 接入 AST 幂等美化器）
- [x] 跨平台：Windows(`lld-link/msvc/mingw`)、Linux/macOS(`clang`)——按编译器宿主平台选择运行时 API 与链接器，支持各平台本机编译，暂不支持交叉编译

### 测试与质量

- [x] 单元测试：全 Workspace 覆盖，0 警告 0 错误
- [x] 原生测试套件与集成回归全绿（`bash huzc/test.sh` 155 passed，单元测试 64 项），支持名称过滤与子集执行
- [x] 集成回归：`bash test.sh`（cwd=`huzc`；仓库根请用 `bash huzc/test.sh`；全量示例与负例测试全部通过，交互示例跳过）
- [x] 性能门禁：`test/bench_compare.py`（cwd=`huzc`；仓库根请用 `huzc/test/bench_compare.py`，huzi release / Rust -O <= 2.0x；三门禁：结果一致性、release 优于 dev、比值门禁）；`test/bench_baseline.txt` 存档历史比值，仅漂移提示（超基线 10% 打印提示），不改阈值与三门禁；运行三处：cwd=`huzc` 直跑 / `RUN_BENCH=1 bash test.sh` 顺带跑 / 仓库根 `bash check.sh` [4/4] 抽查
- [x] 构建产物：Release 产物三平台自动化归档上传
- [x] 规范门禁：单文件 ≤500 行、单函数 ≤70 行、零警告、中文一事一提交

## 3. 明确不做（非缺失，是取舍）

* 无精确 GC（不做 tracing 收集器）：采用确定性作用域析构（RAII Drop）与双计数引用计数（Swift 风格强弱引用）统一模型；`vec` 与 `Box` 出作用域自动确定性释放堆内存，别名赋值自动 retain 引用计数，零 GC 暂停开销；循环引用通过 `weak Box<T>` 弱引用彻底消除，访问失效对象自动置零（Auto-Zeroing），无需手动打破（用例 `74_weak_reference`）
* 泛型无 `where` 约束、无特化
* 包管理无中心仓库、无下载校验和、无 semver 自动升级（求解只选最高满足版，不改写清单）
* 无 UDP/TLS、无 async/协程、无跨线程共享 `vec/map`

## 4. 文档地图（去哪看什么）

| 想知道 | 去哪看 |
|---|---|
| 怎么用语言/编译器选项/内置函数 | `USAGE.md` |
| 架构/流水线/模块职责 | `dev/开发文档.md` |
| 泛型冻结规则 | `rfc/rfc_p4_generics.md` |
| 新增 builtin 同步规则 | `../AGENTS.md`（仓库根项目级约定）验证流程第 4 条（reference.md 补签名 → STATUS.md 打勾 → huzc/test/cases 示例，同提交） |
| 本文件 | 只看状态，不看方法 |
