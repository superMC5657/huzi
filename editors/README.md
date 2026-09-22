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
