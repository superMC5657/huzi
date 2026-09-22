# TODO §3 未来扩展 · 可执行开发计划

> 来源：`huzc/TODO.md §3`（内存进阶 / 类型扩展 / 生态工具链，共 7 项，全部 `[x]` 已闭环）。
> 状态基线：`§3` 全 7 项已开发完成并按计划逐一单独 commit，全 Workspace 零警告，集成与回归测试 73/73 全绿，51 个示例格式化全绿。
> 约束：`huzc/AGENTS.md` —— 单文件 ≤500 行、单函数 ≤70 行；`cargo build --workspace` 零警告；`bash test.sh` 全绿；中文提交、一事一提交。

## 依赖与顺序与提交记录

```
P1 defer (e726323) ──┐
                     ├─> P3 RC (dd6a831)（复用 P1 统一退出点）
P2 fmt (c2ab305) ────┘

P4 用户泛型 (843f383) ──> P5 Trait (b583a56)（静态分发，复用单态化）
P6 包管理 (a2f5928) ──> P7 网络/并发标准库分发 (db9fbbe)
```

---

## P1: `defer` [x] 已闭环 (commit `e726323`)

- **目标**：函数退出（`return`/正常落出）按 LIFO 执行 `defer` 栈。
- **IN**：`defer <stmt>;` 仅函数体内；多条；与 `if/for/while/break/continue` 正交。
- **OUT**：不捕获返回值；不跨函数；`exit()/panic()` 直接终止不跑 `defer`（文档写明）。
- **文件清单**：
  - `crates/huzi-ast/src/ast.rs`：`Stmt::Defer(Box<Spanned<Stmt>>)`。
  - `crates/huzi-lexer/src/token.rs`：`defer` 关键字。
  - `crates/huzi-parser/src/parser/mod.rs`：`defer` 语句解析（顶层 `defer` 报错）。
  - `crates/huzi-codegen/src/codegen/`：新增 `defer.rs`（`defer_stack` + `emit_defers()`），`mod.rs` 接线，`stmt.rs` 的 return/函数尾统一调用。
  - `docs/USAGE.md`：`defer` 语义 + 与 `exit/panic` 的交互说明。
- **测试**：
  - `test/examples/39_defer.hz` + `test/expected/39_defer.stdout`（多 defer 逆序、`return` 仍执行、循环内 defer 到函数尾）。
  - `test/neg/defer_top_level.compile_fail.hz`。
- **验收命令**：
  - `cargo build --workspace`（零警告）
  - `bash test.sh`
  - `./target/debug/huzc -i test/examples/39_defer.hz -o test/out/39_defer && ./test/out/39_defer`

## P2: `huzc fmt` [x] 已闭环 (commit `c2ab305`)

- **目标**：`huzc fmt [--check] <file|dir>`，幂等。
- **IN**：基于现有 Lexer+Parser 的 AST pretty-print；4 空格缩进；`test/examples/*.hz` 格式化后语义不变。
- **OUT**：首版允许注释归一化（文档写明）；除 LSP 格式化入口外不做编辑器联动。
- **文件清单**：
  - `crates/huzc/src/fmt.rs`（超 500 行则拆 `fmt/mod.rs` + 子文件）+ `cli.rs` 接 `--check`。
  - `crates/huzi-lsp/src/backend.rs`：可选 `formatting` 入口复用同一函数。
  - `docs/USAGE.md`：`fmt` 用法。
- **测试**：
  - 对 `test/examples/` 全量 `fmt` 两次，第二次零 diff。
  - 抽 3 个文件 `fmt` 前后编译运行输出一致。
- **验收命令**：
  - `cargo build --workspace`
  - `./target/debug/huzc fmt --check test/examples`
  - `bash test.sh`

## P3: `Box/vec/str` 引用计数 [x] 已闭环 (commit `dd6a831`)

- **目标**：堆对象带 refcount 头，`free_*` 保留为显式提前释放，默认自动。
- **IN**：`retain/release` 在赋值/传参/返回/作用域结束插入；浅释放语义不变；二次 free 仍 no-op。
- **OUT**：不做精确 GC；循环引用会漏（文档写明，推荐 `defer free` 打破）；`Box` LLVM ABI 不变（仍 `ptr`，头在负偏移）。
- **文件清单**：
  - `crates/huzi-codegen/src/codegen/runtime.rs`：`emit_retain/emit_release`。
  - `crates/huzi-codegen/src/codegen/`：`boxed.rs` / `vec.rs` / `builtins_string.rs` 分配点初始化 count=1，变量槽退出点插 `release`（复用 P1 退出点）。
  - `test/examples/40_rc.hz` + stdout。
- **测试**：别名赋值、函数传参返回计数；`35_memory_free.hz` 不改仍过；`bench_perf` 回退 <10%。
- **验收命令**：
  - `cargo build --workspace`
  - `bash test.sh`
  - `python test/bench_compare.py`（抽查）

## P4: 用户泛型 [x] 已闭环 (commit `843f383`)

- **目标**：泛型函数 + 泛型结构体，单态化。先冻结 1 页 RFC 再动手。
- **语法（首版最小，显式实参，无推导）**：
  ```huzi
  struct Stack<T> { v: vec<i32> }
  fn id<T>(x: T) -> T { return x }
  ```
- **IN**：显式 `Stack<i32>` / `id<i32>(...)`；实例化缓存 + 名称修饰（`id__i32`）；复用 `Box<T>/vec<T>` 逻辑；顺带修 `vec` 字段无类型语法问题。
- **OUT**：无类型推导、无泛型枚举/`match` 穷尽、无 `where` 约束、无特化。
- **文件清单**：
  - `crates/huzi-ast/src/ast.rs`：`FnStmt/StructDef` 加 `type_params`；`Type` 加 `Generic(String)` + 参数化类型。
  - `crates/huzi-parser/src/parser/`：`<T>` 解析。
  - `crates/huzi-codegen/src/codegen/`：新增 `generic.rs`（实例化缓存 + AST 替换 + 单态化编译），`types.rs`/`mod.rs` 接线。
  - `crates/huzi-error/`：实参个数错配/未知类型变量报错 + 相似名建议。
- **测试**：`41_generic_fn.hz`、`42_generic_struct.hz` + 3 个 `neg`（实参个数错、约束外类型、未定义类型变量）。
- **验收命令**：`cargo build --workspace` + `bash test.sh` + `--release` 下 `opt` 仍可用。
- **风险**：高。语法冻结前不写 codegen。

## P5: `Trait` 最小可用 [x] 已闭环 (commit `b583a56`)

- **目标**：静态分发 `trait + impl`，无虚表。
  ```huzi
  trait Printable { fn show(self) -> str }
  impl Printable for Point { fn show(self) -> str { ... } }
  ```
- **IN**：方法声明 + 实现；调用点静态直调（复用 P4 单态化）。
- **OUT**：无 `dyn`/虚表/对象安全；首版无默认方法、无泛型 trait；多 trait 冲突直接报错。
- **文件清单**：`ast.rs`（`TraitDef/ImplBlock`）、`parser`、`codegen/trait_.rs`（注意 `trait.rs` 为关键字，命名避让）、`USAGE.md`。
- **测试**：`43_trait.hz` + 冲突 `neg`。
- **验收命令**：`cargo build --workspace` + `bash test.sh`。

## P6: 包管理器 [x] 已闭环 (commit `a2f5928`)

- **目标**：`huzi.toml` + `huzc add/fetch/build` 最小闭环，本地 `vendor/` 优先，离线可用。
- **IN**：manifest（包名/版本/依赖路径）；`import pkg::mod` 解析到 `vendor/<pkg>/<ver>/`；首版只精确版本、无 semver 范围。
- **OUT**：无中心仓库、无 semver 求解、无 lockfile 传递合并（冲突直接报错）。
- **文件清单**：`crates/huzc/src/pkg.rs`（解析+缓存 `~/.huzi/`），`modules.rs` 扩展解析顺序（内置→相对→vendor→缓存），`docs/USAGE.md`。
- **测试**：`test/pkg/` 夹具包 + 包版模块示例；无网络全绿。
- **验收命令**：`cargo build --workspace` + `bash test.sh` + 包示例 end-to-end。
- **风险**：Windows `\\?\` 前缀，复用 `main.rs` 的 strip 逻辑。

## P7: 网络 + 并发标准库 [x] 已闭环 (commit `db9fbbe`)

- **目标**：`tcp_connect/send/recv/close` + `spawn/join/mutex/channel` 最小可用，Win/Linux/macOS。
- **IN**：TCP 同步阻塞（首版无 async）；线程基于 pthread/CreateThread；`str` 跨线程按值拷贝（避开跨线程 refcount 原子性，文档写明）。
- **OUT**：无 UDP/TLS、无协程调度器、无跨线程共享 `vec/map`。
- **文件清单**：`codegen/builtins_net.rs`、`builtins_thread.rs`，`linker.rs`（Winsock/pthread 链接库），`.github/workflows` 三平台。
- **测试**：`44_tcp_echo.hz`（本地回环）、`45_threads.hz`（4 线程求和 join）。
- **验收命令**：`cargo build --workspace` + `bash test.sh` + 三平台 CI 绿。
---

## 全局门禁（每阶段）

1. `cargo build --workspace` 零错误零警告。
2. `bash test.sh` 全绿（含新增 `examples/expected/neg`）。
3. `git diff` 复核；中文提交一事一提交。
4. 单文件 500 行 / 单函数 70 行，超限即拆。

## 建议里程碑

- M1（约 3 周）：P1 + P2，可用性立竿见影。
- M2：P3，内存从手动到半自动。
- M3：P4 + P5，类型系统质变（最长）。
- M4：P6 + P7，生态分发。
