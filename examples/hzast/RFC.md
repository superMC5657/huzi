# RFC: hzast M1-C 求值/JSON 对拍（冻结版 v1）

> 对标 A1（`--dump-parse-stats` + `parse_diff.py`）：C1 先对拍 6 自测手工构 AST 的求值结果；
> C2 对拍 `expr_to_json`/`stmt_to_json` 与 Rust 侧 `--dump-ast-json`。
> 先冻结格式再动手；转义与 Box/Span 取舍是难点，本文件为唯一口径。

## 1. C1 口径（冻结）：6 自测手工构 AST

`src/main.hz` 6 自测各构造一个手工 AST 并经 `eval::*` 求值，断言值如下：

| # | 用例 | 手工 AST | 期望值 |
|---|------|----------|--------|
| 1 | arithmetic | `(3+4)*5-10` | `25` |
| 2 | variables_and_assign | `let x=10; x=x+20; return x*2` | `60` |
| 3 | conditional_if | `let a=15; let r=0; if a>10 {r=100} else {r=200}; return r` | `100` |
| 4 | while_loop | `sum=0; i=1; while i<=10 {sum+=i; i+=1}; return sum` | `55` |
| 5 | functions_and_recursion | `fact(n); fact(5)` | `120` |
| 6 | string_operations | `"hello "+"huzi"` | `"hello huzi"` |

对拍方法（`eval_diff.py`，复用 A1 的 `<huzc-bin> <hzast-bin>` 双二进制模式）：

- `hzast --dump-eval` 打印 6 行 `name=value`（`arithmetic=25` … `str=hello huzi`），顺序固定，行尾 `\n`。
- `huzc build` 产物 `eval_ref.hz`（同目录单文件原生程序，直写同一算式，不经 `eval`）打印同样 6 行。
- 脚本按 `b'\n'` 切分、二进制安全比较（复用 `dump_diff.py` 的 `split_lines` 判据，不 strip），6/6 一致即 PASS。
- 任一侧非零退出即 FAIL，不比较内容。

## 2. C2 口径（冻结）：JSON 子集 v1

### 2.1 覆盖边界（现仅子集，非缺口）

- Expr：`num/bool/str/var/bin/un/call`（7 种；BACKLOG 记 `5/19` 为旧数，现按 `ast.hz` 实际 7 种验）。
- Stmt：`let/assign/if/while/return/print/expr`（7 种）。
- Type：0/16，本期不做（`ast.hz` 无 Type 建模，JSON 不含类型字段）。
- 其余 Rust 侧 `Expr/Stmt` 变体（`match/closure/struct` 等）本期遇到即 Rust 侧非零退出 + Huzi 侧 `""`，不对拍。

### 2.2 Canonical JSON（逐字节一致，两种实现必须同串）

- 紧凑、无空白、键序固定、末尾单 `\n`（`print` 自带换行；对拍按行切分后比较）。
- 形状：

```
expr := {"kind":"num","value":INT}
      | {"kind":"bool","value":true|false}
      | {"kind":"str","value":STRING}
      | {"kind":"var","name":STRING}
      | {"kind":"bin","op":STRING,"left":expr,"right":expr}
      | {"kind":"un","op":STRING,"operand":expr}
      | {"kind":"call","name":STRING,"args":[expr,...]}
stmt := {"kind":"let","name":STRING,"expr":expr}
      | {"kind":"assign","name":STRING,"expr":expr}
      | {"kind":"if","cond":expr,"then":[stmt,...],"else":[stmt,...]}
      | {"kind":"while","cond":expr,"body":[stmt,...]}
      | {"kind":"return","expr":expr}
      | {"kind":"print","expr":expr}
      | {"kind":"expr","expr":expr}
```

- `num` 为十进制 `i32`（`to_string` 口径，不补零）；`bool` 小写；数组 `[]` 空即 `[]`。
- `if` 的 `else` 空即 `[]`（Huzi `else_body` 空 vec 对应）；`call` 空参即 `[]`。

### 2.3 转义（冻结，与 `std.jsonq::escape_json_str` 同口径）

- 仅转义 5 种：`"` → `\"`、`\\` → `\\`、`\n` → `\n`、` \r` → `\r`、`\t` → `\t`。
- 其余控制字符（`\b \f \u00XX`）与裸控制字节不在本期语料内；若出现，两侧行为未定义（不对拍）。
- 非 ASCII 按 UTF-8 字节透传（Huzi 侧 `substring(s,i,i+1)` 逐字节拼接；Rust 侧逐字节匹配同 5 种，其余字节原样透传，不做 `\u` 转义）。与 `serde_json` 默认的透传一致，刻意不用 `serde_json` 以免其 `\b \f` 多转义导致幽灵 diff。
- `op/name` 字段同样经此转义（本期 `op` 仅 `+ - * / % == != < <= > >= !`，无转义触发；`name` 仅 `[A-Za-z0-9_]`）。

### 2.4 Box/Span 取舍（冻结）

- Box 透明：Huzi `Box<Expr>` 与 Rust `Box<Expr>` 均不落盘（`bin.left/right`、`un.operand` 直接内联子 JSON）。
- Span 丢弃：Rust `Spanned<Stmt>` 只取 `.node`；Huzi 侧本无位置。JSON 不含任何行列字段。
- `null` 右子（Huzi `expr_un` 的 `right=null`）不落盘，统一为 `operand` 单字段。

### 2.5 语料（向量 ID，非 .hz 文件）

M1-C 不含 Huzi 侧通用 `.hz → JSON` 解析器（那是 hzparse 域），故语料为固定向量 ID（`ast_diff.py` 循环）：

```
expr_num, expr_bool, expr_str, expr_str_esc, expr_var,
expr_bin, expr_un, expr_call,
stmt_let, stmt_assign, stmt_if, stmt_while, stmt_return, stmt_print, stmt_expr,
prog_fact
```

- 每个 ID 两侧各构造等价 AST（Huzi 经 `ast::*` 手工构；Rust 经 `huzi_ast::*` 合成，见 `ast_json::test_vector`），经同一序列化函数落盘，逐字节比较。
- `prog_fact` 为唯一程序级向量：`fact` 单函数 + `main: return fact(5)`，验 `fn/program` 拼装（含 `params/body` 键序）。
- Rust 文件模式（`--dump-ast-json -i <file>`，整程序 `{"fns":...,"main":...}`）本期仅供人工排查，不进对拍（Huzi 侧无解析器，无法对等）。

对拍方法（`ast_diff.py`）：`huzc --ast-json-test <id>` vs `hzast --dump-json <id>`，单行 JSON 二进制安全比较（同 `split_lines`），16/16 即 PASS。

## 3. 刻意不做（非缺口，是冻结边界）

- 无 Type JSON、无 Span、无 `match/closure/struct/enum` 等全量变体。
- 无错误恢复：未知 ID 两侧均非零退出（Rust `eprintln+exit1`，Huzi `panic` 非零退出）。
- 无 `serde_json` 依赖：两侧手写转义，保持零新增依赖。
- 不碰 `codegen/llc/lsp/huzi-src/log`（预存 `mul` constexpr 债禁碰）。

## 4. 已落地 API 清单（只用这些）

`len/push/concat/to_string/substring/split/arg_count/arg/print`、`vec<>/Box/null`、`core.assert`；Huzi 侧转义复用 `substring(s,i,i+1)` 单字节切片模式（与 `jsonq` 同构，不 `import std`，保持闭环）。
