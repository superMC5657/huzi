# hzast —— Tree-Walking AST 求值与 JSON 对拍（M1-C）

用 Huzi 写的 Huzi AST 建模 + 树遍历求值器：Huzi 侧手工构造 AST，
经 `eval::*` 直接求值；另有 `json::*` 手写 Canonical JSON 序列化，
与 Rust 侧（`huzi-ast` + `huzc --ast-json-test`）逐字节对拍。
全程不拼 LLVM IR、不调 clang——这正是与 `examples/hzir` 的区别：
hzir 是“字符串拼接发射文本 IR 再调 clang”的远景 spike，
hzast 是“内存 AST 直接求值”的语义对拍。两者范围互补，不互相依赖。

## Tree-Walking 范围（冻结，口径见 `RFC.md`）

`src/main.hz` 6 个自测各手工构造一个 AST 并求值断言：

| # | 自测 | AST | 期望值 |
|---|------|-----|--------|
| 1 | arithmetic（算术） | `(3+4)*5-10` | `25` |
| 2 | variables_and_assign（环境） | `let x=10; x=x+20; return x*2` | `60` |
| 3 | conditional_if（分支） | `a=15; if a>10 {r=100} else {r=200}` | `100` |
| 4 | while_loop | `while i<=10` 累加 | `55` |
| 5 | functions_and_recursion | `fact(5)`（递归，环境链传参） | `120` |
| 6 | string_operations（字符串） | `"hello "+"huzi"` | `"hello huzi"` |

说明：

- 环境：`eval::env_new` 新环境，`let/assign` 读写变量，无闭包捕获。
- 递归：以 `fact` 为唯一代表；`fib` 不在 hzast 范围内
  （`fib` 是 hzir 8 用例之一，见 `examples/hzir/README.md`）。
- JSON 子集（C2）：Expr 7 种（`num/bool/str/var/bin/un/call`）、
  Stmt 7 种（`let/assign/if/while/return/print/expr`），无 Type/Span；
  转义仅 `"` `\\` `\n` `\r` `\t` 5 种——超范围输入两侧约定非零退出，不对拍。

## 与 hzir 的区别

| | hzast（本目录） | hzir |
|---|---|---|
| 执行方式 | 内存 AST 树遍历求值 | 字符串拼接文本 LLVM IR，调 clang 汇编链接 |
| 对拍对象 | Rust 侧求值结果 / Canonical JSON | huzc 直编译产物的 stdout |
| 外部依赖 | 无（只用语言内置 + `core.assert`） | `write_file` + `process_run("clang")` |
| 口径文件 | `RFC.md`（M1-C 冻结版 v1） | `RFC.md`（一页冻结子集） |

## 用法

```bash
# 1. 编译 hzast（仓库根目录执行）
huzc/target/debug/huzc build --path examples/hzast --output /tmp/hzast

# 2. 内置断言（默认即自测；6 自测全过才打印 pass）
/tmp/hzast --selftest
/tmp/hzast   # 同上，缺省参数默认跑自测

# 3. C1 对拍：--dump-eval 6 行 vs 原生 eval_ref.hz（直写算式、不经 eval）
python examples/hzast/eval_diff.py ./huzc/target/debug/huzc /tmp/hzast

# 4. C2 对拍：16 个固定向量 ID 的单行 JSON 逐字节比较
python examples/hzast/ast_diff.py ./huzc/target/debug/huzc /tmp/hzast
```

- `--dump-eval` 打印 6 行 `name=value`
  （`arithmetic/variables/conditional/while/fact/str`），顺序固定；
  `eval_ref.hz` 由脚本即时编译执行，两侧按 `b'\n'` 切分、二进制安全比较，不 strip。
- `--dump-json <id>` 打印单个向量 ID 的单行紧凑 JSON；
  未知 ID 两侧均非零退出（Rust 侧 `eprintln+exit1`，Huzi 侧 `panic`）。
- 两个脚本任一向量 FAIL 即非零退出；`split_lines` 的 CRLF 判据与
  `examples/hzlex/dump_diff.py` 同构（全行尾 `\r` 才去 `\r`，内容 `\r` 无损）。

## 文件地图

| 文件 | 说明 |
|---|---|
| `src/main.hz` | 入口：6 自测 + `--selftest/--dump-eval/--dump-json` 分发 |
| `src/ast.hz` | Expr/Stmt/值构造子（Box 透明、无 Span 建模） |
| `src/eval.hz` | 树遍历求值器（环境 + 函数表） |
| `src/json.hz` | Canonical JSON 手写序列化（含 5 种转义） |
| `src/vectors.hz` | 16 个固定向量 ID 的 JSON 语料 |
| `src/dump.hz` | `--dump-eval/--dump-json` 打印（与自测同构，不断言） |
| `eval_ref.hz` | C1 原生参照（huzc 直编译，不经 eval） |
| `ast_diff.py` / `eval_diff.py` | C2 / C1 对拍脚本 |
| `RFC.md` | 冻结口径（唯一标准，本文件只转述） |
