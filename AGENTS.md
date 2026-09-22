# AGENTS.md — huzi 项目级约定

本文件供人类开发者与 AI 编码代理共同遵守。管理对象是仓库根 `huzi` 整个项目（编译器 + 自举标准库 + 编辑器 + 示例 + 文档），改动前请先通读本文件。默认工作目录 cwd=仓库根。

## 项目结构

```
huzi/                  # 仓库根（cwd=仓库根执行本文件命令）
├── AGENTS.md          # 本文件：项目级编码与重构约定
├── docs/              # 项目级文档（USAGE 为入口，余下按主题分类）
│   ├── USAGE.md / STATUS.md
│   ├── guides/        # tutorial.md 教程 + reference.md 参考手册
│   ├── dev/           # 开发文档.md 技术架构
│   └── rfc/           # 泛型设计冻结文档
├── check.sh           # 仓库统一本地门禁入口（四阶段聚合，见“验证流程”）
├── huzc/              # 编译器主工程（Rust workspace + LLVM/inkwell 后端）
│   ├── Cargo.toml     # workspace 根（members = crates/*）
│   ├── crates/        # huzc / huzi-lexer / huzi-parser / huzi-ast / huzi-codegen / huzi-error / huzi-lsp
│   ├── test/          # 特性示例、标准输出快照与负例集
│   └── test.sh        # 编译器回归脚本（cwd=huzc，或仓库根 `bash huzc/test.sh`）
├── huzi-src/          # 自举标准库（core/alloc/std + test 自测，见 huzi-src/README.md）
├── editors/           # 编辑器支持（README.md 矩阵 + vscode/ 官方插件工程）
├── examples/          # 自举示例（hzlex/hzparse/task_engine）
└── .omo/              # 内部计划 plans/ + 会话续跑 run-continuation/，只看不用、勿删
```

各模块归属文档：编译器工程见 `huzc/README.md`；标准库分层见 `huzi-src/README.md`；编辑器矩阵见 `editors/README.md`；用户手册入口见 `docs/USAGE.md`；状态快照见 `docs/STATUS.md`。

## 代码组织规则

### 1. 单文件与单方法长度上限

- **单文件不超过约 500 行**；超过时按职责拆分为目录模块（`<name>/mod.rs` + 子文件）。Rust 与 `.hz` 同例，`.hz` 示例超长时按特性拆分用例文件。
- **单方法（函数）不超过约 70 行**；超过时按阶段/分支提炼为更小的子函数，主函数只保留编排逻辑。
- **用fd替换掉find**；windows下find有问题。

### 2. 模块拆分方式（主要约束 `huzc/crates/`）

- 一个大 `impl` 拆到多个文件时，类型定义与入口保留在 `mod.rs`，子模块用 `impl super::TypeName` 承接方法（依赖“子模块可访问父模块私有字段”规则，不必放宽可见性）。
- 子模块中被跨模块调用的方法统一标 `pub(super)`；**不要**为了让别的 crate 使用而把内部方法改成 `pub`，对外 API 只经 `lib.rs` 的 `pub use` 暴露。
- 子文件只引入实际用到的导入；提交前必须清零 unused import 警告。
- 拆分应让相邻的代码块（如同组内置函数、同类型语句）落在同一文件，保持人类可读的分组，例如：

```
huzc/crates/huzi-codegen/src/codegen/
├── mod.rs          # CodeGen 结构、compile() 入口、作用域栈、公开 API
├── types.rs        # 类型注册/布局、type_to_llvm 等类型工具
├── stmt.rs         # 语句编译
├── expr.rs         # 表达式编译
├── builtins.rs     # prelude 声明 + print/read_*/数学/字符串内置函数
└── aggregates.rs   # 结构体/枚举/match/数组
```

### 3. 各子工程边界

- `huzc/`：唯一可依赖 LLVM/系统调用的地方（RC、socket、线程原语、`free_*`、panic 终止）。凡能用 Huzi 自身写的收录进 `huzi-src/`，不进 codegen。
- `huzi-src/`：按 `core`（无堆纯逻辑）→ `alloc`（堆容器算法）→ `std`（OS 薄封装）分层；各包 `lib.hz` 为规范入口（`huzi.toml` 的 `lib_entry` 指定），经 `export` 重导出对外 API；不建空壳目录（SIMD/过程宏等不支持的不占位）。
- `editors/vscode/`：关键字/内置函数表与 `huzc/crates/huzi-lexer/src/token.rs` 及 `docs/guides/reference.md` 对齐；加关键字或 builtin 时同步 `syntaxes/huzi.tmLanguage.json`。
- `examples/`：自举示例各自闭环（源码 + 自测断言），不对编译器做反向依赖。

### 4. 重构纪律：纯搬移优先

- 文件/方法拆分时**代码逐行原样搬移，不改任何逻辑、不改变量名、不重排语句顺序**。
- 对外行为必须完全不变；如确需行为变更，单独开一次提交，不与重构混合。
- 顺带拆解超长方法时，提取出的子函数命名要描述“做什么”（如 `emit_for_condition`、`resolve_enum_variant`），并配文档注释说明阶段职责。

### 5. 同类型实参要防错位

把语句块、指针等同类型参数提取为函数参数时（如 `emit_for_condition(..., body_block, loop_block, ...)`），**调用处与签名的参数顺序必须逐一核对**——顺序错位不产生编译错误，只产生错误的 IR。

## 验证流程（重构/改动的验收标准，cwd=仓库根）

1. `cargo build --manifest-path huzc/Cargo.toml --workspace` — **零错误、零警告**（或 `cd huzc` 后 `cargo build --workspace`）。
2. `bash check.sh` — 仓库统一门禁（四阶段：`huzc/test.sh` 编译器回归 → `huzi-src/test.sh` 标准库自测 → `huzc fmt --check huzc/test/cases` 格式门禁 → `bench_compare.py` 性能抽查；跳过性能用 `SKIP_BENCH=1 bash check.sh`）。单跑编译器回归可用 `bash huzc/test.sh`（cwd=`huzc` 时为 `bash test.sh`）；交互类示例（如 `10_guess_number_game.hz`）跳过。
3. `git diff` 复核：确认是纯搬移（删除行与新增行内容对应），没有夹带逻辑改动。
4. 新增 builtin 必须同提交同步文档：`docs/guides/reference.md` 补签名 → `docs/STATUS.md` 打勾 → `huzc/test/cases/` 补示例，缺一即未通过。涉及标准库自举封装时同步 `huzi-src/` 对应 `lib.hz` 与 `test/cases/` 自测；涉及编辑器高亮时同步 `editors/vscode/syntaxes/huzi.tmLanguage.json`。

> 本项目不配置 CI（GitHub Actions workflow 已移除，后续不做 CI）：上述门禁全部在本地执行，提交前自行跑完 1-2。

## 提交约定

- 提交信息用中文，格式：`新增:|修复:|重构:|移除:<摘要>`，正文列出具体拆分/变更点。
- 一次提交只做一件事：纯重构（行为零变化）与功能/行为变更分开提交。
