# Huzi for VS Code

Huzi 编程语言（`.hz`）的 VS Code 语法高亮与基础语言配置。当前为第一阶段：只做高亮与编辑体验，不含语言服务器（诊断/补全/跳转见后续 LSP 计划）。

## 功能

- `.hz` 文件识别与语法高亮（关键字、类型、`Box`、字符串/字符、数字、注释 `//` 与 `#`）
- 内置函数高亮（`print`、`vec`/`push`、`box`、数学函数、读写与系统函数等）
- `模块::符号` 路径、`math::` 标准模块高亮
- 括号/引号自动闭合，`{}` 块自动缩进
- 扩展图标

关键字与内置函数表与 `huzc` 的 `huzi-lexer`（`huzc/crates/huzi-lexer/src/token.rs`，仓库根相对路径）及项目级文档 `docs/guides/reference.md`（仓库根相对路径，本目录视角为 `../../docs/guides/reference.md`）对齐；编译器加新关键字或内置函数时请同步 `syntaxes/huzi.tmLanguage.json` 的对应正则。

## 本地安装（开发版）

```bash
# 方式一：直接链接到扩展目录（改完重启 VS Code 即生效）
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

## 验证

1. 用 VS Code 打开 `huzc/test/cases/` 下任意 `.hz` 文件，确认右下角语言模式显示为 `Huzi`。
2. 检查高亮：`fn`/`struct`/`match` 等关键字、`"字符串"`、`// 注释`、`Box<Node>` 应有不同颜色。
3. 输入 `{` 回车应自动缩进，`"`、`(` 应自动闭合。

## 语言服务器（LSP）

打开 `.hz` 文件（`language: huzi`）时自动启动 `huzi-lsp`（stdio）：红线诊断、hover、跳转、补全可用。客户端实现见 `src/extension.ts`
（`LanguageClient` id 为 `huzi`，`documentSelector` 为 `{ scheme: 'file', language: 'huzi' }`）。

查找顺序（无硬编码绝对路径）：`huzi.server.path` 配置 → 扩展自带 `server/huzi-lsp(.exe)` → `PATH` 中的同名二进制。

### 本地调试

```bash
# 1. 构建 server（仓库根目录）
cargo build -p huzi-lsp

# 2. 在 VS Code 设置里指向刚构建的二进制（二选一）：
#    a) settings.json（Windows 示例）
#    "huzi.server.path": "Z:/rust_workplace/huzi/huzc/target/debug/huzi-lsp.exe"
#    b) unix 示例
#    "huzi.server.path": "/path/to/huzi/huzc/target/debug/huzi-lsp"

# 3. 编译并运行扩展（在本目录 editors/vscode）
npm install
npm run compile
# 然后按 F5 启动「扩展开发主机」，在新窗口打开 .hz 文件
```

- 也可把各平台二进制分别拷入本扩展 `server/` 目录（`huzi-lsp` / `huzi-lsp.exe`），不配 `path` 即可直接用；`vsce package` 会把 `server/` 与 `out/` 一起打进 vsix（见 `.vscodeignore`），请勿提交 `.vsix` 文件。
- 输出面板 `Huzi LSP` 可看 server 日志；`huzi.trace.server` 设为 `messages`/`verbose` 可追踪协议；命令面板 `Huzi: Restart Language Server` 可重启 server。
- 高亮说明：TextMate `scopeName` 保持 `source.huzi` 不变，`syntaxes/huzi.tmLanguage.json` 正则语义未动，LSP 只新增能力不替换高亮。

## 运行文件（Run）

右键 `.hz` 文件 → `Huzi: Run File`（或命令面板搜同名），在 `Huzi Run` 终端里编译并运行当前文件。

编译器路径由用户自己配置（插件不写死，默认走 `PATH`）：

```json
// settings.json（Windows 示例）
"huzi.huzc.path": "Z:/rust_workplace/huzi/huzc/target/release/huzc.exe"
// unix 示例
// "huzi.huzc.path": "/path/to/huzi/huzc/target/release/huzc"
```

不配则默认用 `PATH` 里的 `huzc`。

## 后续

- [x] LSP（诊断/补全/悬停/跳转）：server 在 `huzc/crates/huzi-lsp`（复用 `huzi-lexer` / `huzi-parser` / `huzi-error`），client 为本目录的 `src/extension.ts` + `package.json` 扩展。
