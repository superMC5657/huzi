# 下一步 Roadmap · 可执行开发计划

> 基线：`TODO §3` 全 7 项已闭环（`e726323`→`db9fbbe`），`master` 干净，`target/debug/huzc.exe` 可构建。
> 现状快照见 `huzc/STATUS.md`。本计划只列“还需开发什么”，不重复已完成项。
> 约束：`huzc/AGENTS.md` —— 单文件 ≤500 行、单函数 ≤70 行；`cargo build --workspace` 零警告；`bash test.sh` 全绿；中文提交、一事一提交。

## 依赖与顺序

```
Q0 稳定收尾（无依赖，先做）──> Q1 止痛（依赖 Q0 门禁全绿）
Q0 ──> Q2 生态定位（可与 Q1 并行，人力允许才做）
```

---

## Q0: 稳定收尾 [x] 已完成

### Q0-1 全量门禁进 CI

- **目标**：三平台（Win/Linux/macOS）`cargo test + test.sh + fmt --check` 全绿，release 产物可下载。
- **IN**：`.github/workflows` 补 `cargo build --workspace`（零警告即失败）、`bash test.sh`、`huzc fmt --check test/examples`。
- **OUT**：不改语言语义，只改流水线。
- **文件清单**：`huzc/.github/workflows/*`、`huzc/test.sh`（超时/跳过交互用例逻辑不动）。
- **测试**：CI 三平台绿；本地 `bash test.sh` 仍全绿。
- **验收**：`cargo build --workspace` + `bash test.sh` + `huzc fmt --check test/examples`。

### Q0-2 性能回退门禁

- **目标**：`test/bench_compare.py`（huzi vs rust）纳入回归，RC 引入的回退可量化。
- **IN**：`bench_perf.hz/rs` 三内核不变；阈值沿用 P3 口径（回退 <10%）。
- **OUT**：不优化 codegen，只加门禁。
- **文件清单**：`huzc/test/bench_compare.py`、`huzc/test.sh`（追加抽查或单独 job）。
- **验收**：`python test/bench_compare.py` 通过。

### Q0-3 边界负例补齐

- **目标**：覆盖最近两笔修复的同类 case，防止回退。
- **IN**：`defer` 嵌套 `return/defer`（`1cbeca7` 同类）、`impl` 内方法调用脱糖与签名校验（`38c598c` 同类）、`Box` 层数错配、`vec/map` 越界。
- **OUT**：不加新语法。
- **文件清单**：`huzc/test/neg/*.compile_fail.hz` + `*.runtime_fail.hz`，`huzc/test.sh` 负例循环自动覆盖。
- **验收**：`bash test.sh` 全绿，负例数只增不减。

## Q1: 用户止痛（按需单点做深，不并行） [x] 已完成

### Q1-1 泛型实参推导（推荐首选）
[x] 已完成 (Commit `e206301`)

- **目标**：`id(42)` 可省略 `<i32>`，推导失败回退显式写法并报错。
- **IN**：调用点按形参类型 + 实参类型推导；缓存复用 P4 单态化（`generic.rs` + 修饰名）。
- **OUT**：无 `where` 约束、无特化、无泛型枚举穷尽（沿用 P4 OUT）。
- **文件清单**：`crates/huzi-parser`（调用点实参省略）、`crates/huzi-codegen/src/codegen/generic.rs`、`crates/huzi-error`（推导失败诊断）。
- **测试**：`41_generic_fn.hz` 同语义新增推导版 + 2 个 `neg`（歧义/推导失败）。
- **验收**：`cargo build --workspace` + `bash test.sh` + `--release` 下 `opt` 仍可用。
- **风险**：中。先冻结推导规则 1 页补充 RFC 再动手。

### Q1-2 循环引用止痛（不做 GC 也行）
[x] 已完成 (Commit `b1bf246`)

- **目标**：循环引用不再静默泄漏，至少可观测、可手动打破。
- **IN**：二选一：(a) 运行时循环检测告警，或 (b) `defer free` 官方示例 + `ref_count` 诊断文档。
- **OUT**：不做精确 GC/tracing。
- **文件清单**：`codegen/runtime.rs`（若选 a）或 `docs/USAGE.md` + `test/examples/40_rc.hz`（若选 b）。
- **验收**：循环 case 有明确报错或文档示例可跑。

### Q1-3 容器泛化（三选一，别全做）
[x] 已完成 (选选项 B: vec/map 作函数参数与结构体字段, Commit `5adbd61`)

- **选项 A**：`HashMap` 从 `str->i32` 放开到至少 `str->str` / `i32->i32`。
- **选项 B**：`vec/map` 可作函数参数与结构体字段（现在只有局部变量形态）。
- **选项 C**：`Box<T>` 从仅结构体放开到基础类型。
- **OUT**：一次只做一个；ABI 变化需同步 `docs/开发文档.md` 类型映射表。
- **验收**：对应 `examples + expected + neg` 全绿。

## Q2: 生态定位 [x] 已完成

### Q2-1 `huzi-src/` 与 `editors/` 去留
[x] 已完成 (Commit `95c43e7`)

- **目标**：消除空目录误导。
- **IN**：二选一：(a) 明确 `huzi-src/` 为未来标准库源码并放 1 个示例，或 (b) 删除空目录；`editors/` 除 vscode 外注明不支持清单。
- **OUT**：不建新编辑器插件。
- **验收**：根目录无空目录，或有 README 说明定位。

### Q2-2 手册拆分与 `fmt` 保注释
[x] 已完成 (Commit `8ffc34b`)

- **目标**：`USAGE.md` 1077 行拆分为教程 + 参考；`fmt` 首版丢注释问题修掉或明确声明。
- **IN**：按现有章节拆文件，链接不断；`fmt` 若保注释需基于 Span 重写，否则文档写明归一化范围。
- **OUT**：不改语言语义。
- **验收**：`huzc fmt --check test/examples` 全绿，两次 fmt 零 diff。

---

## 全局门禁（每阶段）

1. `cargo build --workspace` 零错误零警告。
2. `bash test.sh` 全绿（含新增 `examples/expected/neg`）。
3. `git diff` 复核；中文提交一事一提交。
4. 单文件 500 行 / 单函数 70 行，超限即拆。

## 建议里程碑

- M0（约 1-2 周）：Q0-1 + Q0-3，可发版。
- M1：Q0-2 + Q1-1（推导），类型系统止痛。
- M2：Q1-2 / Q1-3 三选一 + Q2-1/Q2-2，生态去误导。
