> 冻结spike：仅i32子集（hello/arith/shadow/max/sum/fact/call/fib 8用例），外扩需先立RFC

# hzir RFC：一页冻结子集

## 1. 语法子集（EBNF）

```ebnf
prog     ::= fn+ main
fn       ::= "fn" ident "(" params ")" "->" "i32" block
params   ::= "" | param ("," param)*
param    ::= ident ":" "i32"
block    ::= "{" stmt* "}"
stmt     ::= let | assign | if | while | print | return
let      ::= "let" ["mut"] ident "=" expr
assign   ::= ident "=" expr
if       ::= "if" expr block ["else" block]
while    ::= "while" expr block
print    ::= "print" "(" expr ")"
return   ::= "return" expr
expr     ::= term (("+"|"-") term)*
term     ::= factor (("*"|"/"|"%") factor)*
factor   ::= int | ident | call | "(" expr ")"
call     ::= ident "(" args ")"
args     ::= "" | expr ("," expr)*
```

`bool` 只出现在 `if` / `while` 条件（比较 `> < >= <= == !=` 产生），不单独声明。

## 2. IR 映射

| Huzi | LLVM IR |
|---|---|
| `let x = 10` | `%x = alloca i32` + `store i32 10, ptr %x` |
| `let x = 20` 遮蔽 | 新槽 `%x1 = alloca i32`（旧槽保留） |
| `x + y` 等 | `add / sub / mul / sdiv / srem i32` |
| `if c {a} else {b}` | `icmp s* + br i1, label %then, label %else` |
| `while c {b}` | `br label %loop.cond` + `loop.cond: icmp+br` + `loop.body` + `loop.end:` |
| `fn f(a:i32)->i32` | `define i32 @f(i32 %a)` |
| `print(i)` | `call @printf(ptr @.fmt.d, i32 %v)`，`@.fmt.d = c"%d\0A\00"` |
| `print("s")` | `call @printf(ptr @.fmt.s, ptr @.str)`，`@.fmt.s = c"%s\0A\00"` |

## 3. IN / OUT

- IN：`cases/*.hz`（8 个，纯本子集）。
- OUT：`src/main.hz` 经 `concat` 产出 `/tmp/hzir_s*.ll`，`clang` 产出 `/tmp/hzir_s*_exe`。
- 验收线：8 个双编译 stdout 逐字节一致（`diff` 零退出）+ `huzc build --path examples/hzir` 可编。
- 失败即停：任一不一致先查 IR 转义（`\"` / `\\0A` / `\n`），再查块标签，不加原语。
