# AGENTS.md — 编码与重构约定

本文件供人类开发者与 AI 编码代理共同遵守。Huzc 是 Huzi 语言的编译器(Rust workspace + LLVM/inkwell 后端),改动前请先通读本文件。

## 项目结构

```
crates/
├── huzc/           # 编译器 CLI 入口(main + cli/paths/linker)
├── huzi-lexer/     # 词法分析
├── huzi-parser/    # 语法分析
├── huzi-ast/       # AST 定义
├── huzi-codegen/   # LLVM 代码生成
├── huzi-error/     # 错误类型
└── huzi-lsp/       # 语言服务器 (LSP)
```

## 代码组织规则

### 1. 单文件与单方法长度上限

- **单文件不超过约 500 行**;超过时按职责拆分为目录模块(`<name>/mod.rs` + 子文件)。
- **单方法(函数)不超过约 70 行**;超过时按阶段/分支提炼为更小的子函数,主函数只保留编排逻辑。
- **用fd替换掉find**;windows下find有问题

### 2. 模块拆分方式

- 一个大 `impl` 拆到多个文件时,类型定义与入口保留在 `mod.rs`,子模块用 `impl super::TypeName` 承接方法(依赖"子模块可访问父模块私有字段"规则,不必放宽可见性)。
- 子模块中被跨模块调用的方法统一标 `pub(super)`;**不要**为了让别的 crate 使用而把内部方法改成 `pub`,对外 API 只经 `lib.rs` 的 `pub use` 暴露。
- 子文件只引入实际用到的导入;提交前必须清零 unused import 警告。
- 拆分应让相邻的代码块(如同组内置函数、同类型语句)落在同一文件,保持人类可读的分组,例如:

```
huzi-codegen/src/codegen/
├── mod.rs          # CodeGen 结构、compile() 入口、作用域栈、公开 API
├── types.rs        # 类型注册/布局、type_to_llvm 等类型工具
├── stmt.rs         # 语句编译
├── expr.rs         # 表达式编译
├── builtins.rs     # prelude 声明 + print/read_*/数学/字符串内置函数
└── aggregates.rs   # 结构体/枚举/match/数组
```

### 3. 重构纪律:纯搬移优先

- 文件/方法拆分时**代码逐行原样搬移,不改任何逻辑、不改变量名、不重排语句顺序**。
- 对外行为必须完全不变;如确需行为变更,单独开一次提交,不与重构混合。
- 顺带拆解超长方法时,提取出的子函数命名要描述"做什么"(如 `emit_for_condition`、`resolve_enum_variant`),并配文档注释说明阶段职责。

### 4. 同类型实参要防错位

把语句块、指针等同类型参数提取为函数参数时(如 `emit_for_condition(..., body_block, loop_block, ...)`),**调用处与签名的参数顺序必须逐一核对**——顺序错位不产生编译错误,只产生错误的 IR。

## 验证流程(重构/改动的验收标准)

1. `cargo build --workspace` — **零错误、零警告**。
2. 编译并运行 test/examples/ 下全部非交互示例,确认 exit=0;交互类示例(如 `10_guess_number_game.hz`)跳过(可直接运行 `bash test.sh`)。
3. `git diff` 复核:确认是纯搬移(删除行与新增行内容对应),没有夹带逻辑改动。

> 本项目不配置 CI(GitHub Actions workflow 已移除,后续不做 CI):上述门禁全部在本地执行,提交前自行跑完 1-3。

## 提交约定

- 提交信息用中文,格式:`重构:|修复:|新增:<摘要>`,正文列出具体拆分/变更点。
- 一次提交只做一件事:纯重构(行为零变化)与功能/行为变更分开提交。
