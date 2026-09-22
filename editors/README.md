# Huzi 编辑器生态与支持清单

本目录集中管理 Huzi 编程语言的编辑器与集成开发环境（IDE）支持配置。

## 编辑器支持矩阵

| 编辑器 / IDE | 支持状态 | 交付形态 | 说明 |
|-------------|---------|---------|------|
| **Visual Studio Code / Cursor** | **完全支持 (官方)** | 插件源码及 `.vsix` 安装包（位于 `editors/vscode/`） | 支持语法高亮、代码片段、LSP 诊断、悬停签名、跳转定义、成员补全与语义高亮 |
| **Neovim / Vim** | 需手动配置 | 依赖 `huzi-lsp` | 未单独打包。用户可通过 `nvim-lspconfig` 或 `coc.nvim` 直接挂载编译出的 `huzi-lsp` 可执行文件 |
| **JetBrains (IntelliJ / CLion / etc.)** | 暂不支持 | 规划中 | 尚无官方 IntelliJ 插件，推荐使用 LSP 统一插件或 VS Code |
| **Emacs** | 需手动配置 | 依赖 `huzi-lsp` | 用户可通过 `eglot` 或 `lsp-mode` 配置调用 `huzi-lsp` |
| **Sublime Text** | 需手动配置 | 依赖 `huzi-lsp` | 用户可通过 Sublime `LSP` 插件配置 `huzi-lsp` |

## 快速安装 VS Code 插件

1. 构建或获取 `huzi-lsp`（位于 `huzc/crates/huzi-lsp`）。
2. 进入 `editors/vscode/` 目录：
   - 可直接安装预构建包：`code --install-extension huzi-0.3.0.vsix`
   - 或本地开发模式：运行 `npm install` 并通过 VS Code 按 `F5` 启动扩展宿主调试。

## LSP 补全与跳转分级（huzi-lsp）

| 级别 | 内容 | 状态 |
|------|------|------|
| L1 保底 | 补全三板斧（关键字/同文件符号、`Point.` 字段/`Enum::` 变体/`math::` 函数、未知基通用成员兜底）＋跳转（import 行到文件头、`模块::函数` 到模块内 fn 符号）＋语义高亮（keyword/variable/function/type 图例），单测锁定 | ✅ |
| L2 精化 | 存量行为只加单测锁定（枚举 `::` 变体、字段前缀过滤、越界永不 panic、空白/非法 import 不跳、`:` 后类型与 `::` 后函数高亮），不改行为 | ✅ |
| L3 std import 感知 | `import std.json` 可解析到 `huzi-src`（读 `huzi.toml lib_entry`，参照 `modules.rs probe_entry_file`）；补全：`json::` 读模块文件符号、`std::` 读 `std/lib.hz` export 表；跳转：见下 | 补全✅/跳转规划中 |
| L4 Trait/impl 成员 | `Point.` 补全 impl 方法、`Trait::` 补全 trait 方法（读 `symbols.rs` trait/impl 表）；语义高亮小幅扩展（`export` 归 keyword） | ✅ |
