# Huzi for VS Code

Huzi（`.hz`）的 VS Code 插件：语法高亮 + 语言服务（诊断、补全、跳转、悬停）。

## 功能

- `.hz` 文件识别与语法高亮（关键字、类型、字符串、数字、注释 `//` 与 `#`）
- 内置函数与 `模块::符号` 路径高亮
- 诊断红线、悬停签名、跳转定义、成员补全
- 括号/引号自动闭合，`{}` 块自动缩进
- 右键运行当前 `.hz` 文件

关键字与内置函数表和编译器保持同步；加新关键字或函数时同步 `syntaxes/huzi.tmLanguage.json`。

## 安装

```bash
# 方式一：链接到扩展目录（改完重启 VS Code 即生效）
# Windows (PowerShell)
New-Item -ItemType Junction -Path "$HOME\.vscode\extensions\huzi-0.1.0" -Target "$PWD"
# Linux / macOS
ln -s "$PWD" ~/.vscode/extensions/huzi-0.1.0
```

```bash
# 方式二：打包成 .vsix 分发
npm install -g @vscode/vsce
vsce package
# 在 VS Code 中：扩展面板 → … → 从 VSIX 安装
```

装好后打开任意 `.hz` 文件，确认右下角语言模式显示为 `Huzi`。

## 语言服务（LSP）

打开 `.hz` 文件时自动启动 `huzi-lsp`。查找顺序：`huzi.server.path` 配置 → 扩展自带 `server/` 目录 → `PATH` 中的同名二进制。

本地调试：

```bash
# 1. 构建 server（仓库根目录）
cargo build --manifest-path huzc/Cargo.toml -p huzi-lsp

# 2. 在 VS Code 设置里指向刚构建的二进制，例如：
# "huzi.server.path": "Z:/rust_workplace/huzi/huzc/target/debug/huzi-lsp.exe"

# 3. 在本目录编译扩展并按 F5 调试
npm install
npm run compile
```

输出面板 `Huzi LSP` 可看 server 日志；命令面板 `Huzi: Restart Language Server` 可重启服务。请勿提交 `.vsix` 文件。

## 运行文件

右键 `.hz` 文件 → `Huzi: Run File`，在 `Huzi Run` 终端里编译并运行。

编译器路径在设置里配置（`huzi.huzc.path`），不配则默认用 `PATH` 里的 `huzc`。
