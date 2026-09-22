# Huzi 编辑器支持

| 编辑器 | 状态 | 说明 |
|--------|------|------|
| VS Code / Cursor | 官方插件（`editors/vscode/`） | 高亮、诊断、补全、跳转、悬停 |
| Neovim / Vim / Emacs / Sublime | 手动接入 `huzi-lsp` | 用各自的 LSP 客户端挂载编译出的 `huzi-lsp` |
| JetBrains 全家桶 | 暂不支持 | 先用 VS Code |

语言服务能力（诊断、补全、跳转、悬停、语义高亮）由 `huzi-lsp` 提供，详见[用户指南](../docs/USAGE.md)。

## 安装 VS Code 插件

1. 先构建出 `huzi-lsp`。
2. 进入 `editors/vscode/`：
   - 直接安装预构建的 `.vsix` 包：`code --install-extension <包名>.vsix`
   - 或本地开发：`npm install`，然后按 `F5` 启动扩展调试。
