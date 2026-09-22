# RFC: hzparse 最小 Parser 子集（冻结版 v1）

> 对标 `hzlex` 自举里程碑：用 Huzi 写出第二个编译器前端组件——语法分析器子集。
> 输入复用 `hzlex` 的平行数组词法约定 `(ok, kinds, texts, lines, cols)`，
> 输出“结构计数 + 错误表”，能零失败自解析 90+ 个 Huzi 源码文件。

## 1. Token 约定（与 `huzc/crates/huzi-lexer` / `hzlex` 对齐，不重造词法）

- `kind ∈ { kw | ident | int | float | string | char | punct | eof }`，`text` 为原文或解码值。
- 关键字即 `kind == "kw"` 的文本（`fn/struct/enum/trait/impl/let/mut/if/elif/else/for/in/while/return/break/continue/defer/import/export/match/true/false` 等）。
- 多字符标点已由词法层合并：`== != <= >= && || -> => :: ..`；`char` 的 `text` 为码点十进制串，解析器只看 `kind`。

## 2. 冻结语法子集（EBNF，能解析以下全部即达标）

```ebnf
program   ::= stmt* eof
stmt      ::= import | export | fn_def | struct_def | enum_def | trait_def | impl_def
            | let_stmt | return | if_stmt | for_stmt | while_stmt | break | continue
            | defer stmt | block | expr ("=" expr)?
block     ::= "{" stmt* "}"
type      ::= "[" type ";" int "]" | "(" ")" | "(" type ("," type)* ")"
            | ident ("::" ident)* ("<" type ("," type)* ">")?
expr      ::= assign
assign    ::= or ("=" assign)?
or        ::= and ("||" and)*            (* 其余层级：&& → ==,!= → <,<=,>,>= *)
            (* → |,^ → & → +,- → *,/,% → 一元 !,- → 后缀 *)
postfix   ::= primary ("[" expr "]" | "." (int | ident ["(" args ")"])
                      | "(" args ")" | "?")*
primary   ::= int | float | string | char | true | false | ident_expr
            | "(" expr ("," expr)* ")" | "[" (expr ("," expr)*)? "]"
            | if_expr | match_expr
ident_expr::= "box" "(" expr ")" | "null" | "vec" "<" type ">" "(" ")"
            | ident ("<" type ("," type)* ">" (call | struct_lit) | ("::" ident)+ (call)? | struct_lit)?
call      ::= "(" (expr ("," expr)*)? ")"
struct_lit::= "{" ident ":" expr ("," ident ":" expr)* ","? "}"
if_expr   ::= "if" expr block ("elif" expr block)* "else" (if_expr | block)
match_expr::= "match" expr "{" (pattern "=>" (block | expr) ","?)* "}"
pattern   ::= "_" | ident ("::" ident)? ("(" (ident ("," ident)*)? ")")?
```

顶层条目形状：`import a.b / a::b`；`export p / p::* / p::q`；`fn f<T>(x: T) -> R block`；
`struct S<T> { f: T }`；`enum E { V, W(T, U) }`；`trait T { fn m(self, ...) -> R }`；
`impl T for S { fn m(self: S, ...) -> R block }`；`let [mut] x [: T] [= expr]`；
`for x in expr [".." expr] block`；`while expr block`。

## 3. 刻意不做（非缺口，是冻结边界）

- 无 `match` 穷尽检查、无泛型单态化/约束求解：`match` 只验“模式形状 + `=>` + 体”。
- 无上下文相关语义检查：`defer` 是否在函数内、`return` 是否在 `defer` 内、变量是否定义——只查结构。
- 无错误恢复：首错即停并报告 `token 下标 + 文本`（对标 `Parser::parse` 首错语义，而非 `parse_recoverable`）。
- 无 AST 落盘：输出 8 类顶层计数 + 最大表达式嵌套深度，足以做自解析断言。

## 4. 已落地 API 清单（只用这些，不新增 builtin、不改 codegen）

`len / push / concat / to_string / substring(未用) / split(未用)`、
`vec<str>/vec<i32>` 句柄传参与下标读写、`read_file / read_file_ok`、
`arg_count / arg`、`print`、`core.assert::{assert_true, assert_eq_i32, assert_eq_str}`、
元组多返回与 `.0/.1` 取值、`if` 表达式、`for in / while`。
**缺口：无。** Trick：游标与深度用单元素 `vec<i32>` 做“可变单元”共享，
绕开结构体值语义，实现回溯（`save/restore`）与 `<` 泛型/比较二义消解。

## 5. 验收线

`./hzparse --selftest`：单元断言（词法复用 + 表达式优先级 + fn/struct/match/泛型/`?`/方法调用/元组下标各一）
全过；语料 = `huzc/test/cases/*.hz` + `mods/*.hz` + `huzi-src` 全 `.hz` + `examples/hzlex|task_engine` 源码，
**解析成功文件数 ≥ 90 且失败数为 0**（逐文件打印 `ok <path> fns=.. structs=.. depth=..`，失败即 non-zero 退出）。
`huzc fmt --check examples/hzparse` 通过。
