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
- OUT：`src/main.hz` 经 `concat` 产出 `target/hzir_s*.ll`，`clang` 产出 `target/hzir_s*.exe`
  （P4-a 由 `/tmp` 迁移至仓库内 `target/`：Windows 无 `/tmp` 且门禁要求产物只放树内；
  前缀自适应仓库根 vs `examples/hzir`，`mkdir` 双试，exe 运行失败回退 win 反斜杠）。
- 验收线：8 个双编译 stdout 逐字节一致（`diff` 零退出）+ `huzc build --path examples/hzir` 可编。
- 失败即停：任一不一致先查 IR 转义（`\"` / `\\0A` / `\n`），再查块标签，不加原语。

## 4. P4-a 增补：JSON→文本 IR lower（先开 s1/s2/s5，其余留模板）

- IN：`json/*.json`（P2-b JSON：`huzc --dump-ast-json` 落盘，`hzparse --dump-ast-json`
  双零逐字节一致经 `split_lines` 归一，LF + 末尾换行；本期仅 s1/s2/s5 三快照）。
- 模块：`src/jlower.hz`（core：`find/skip/expect/parse_jstr(5 转义)/parse_jnum`
  + 通用算术 `num/bin(+ - * / %→add/sub/mul/sdiv/srem)` 递归发射，`tmp` 线程化）
  + `src/jlower_cases.hz`（分用例 lower，`import jlower`）。
- 放开行（每用例一行）：
  - s1_hello：仅 `print(str)` 单句，`msg/size=len+1` 全动态，含 `"`/`\` 拒收（转义 P4-b）。
  - s2_arith：4×`print(bin)` 经通用 `jl_emit_arith`（`num/bin` 任意嵌套，`%t` 连续），逐结果 `printf`。
  - s5_sum：仅 sum 形状（`let sum/let i/while i<=N/ sum=sum+i / i=i+1 /print sum`），
    `sum0/i0/bound` 动态，条件锁 `<=→sle`，步长锁 `1`，名锁 `sum/i`，`loop.cond/body/end` 三件套。
- OUT：`target/hzir_s<N>j.ll/.exe`（`N=1/2/5`），复用 `check_one`（`write_file→clang→run→trim` 比对）。
- 胶水复用（零新增原语）：`read_file/read_file_ok/write_file/process_run/trim`
  + `len/substring/concat/to_string/parse_int/contains`（编译器与标准库均不动）。
- 验收：`target/hzir.exe` 自检旧 8 + 新 3 全过 + `hzir_diff.py <huzc> <hzir>` 3/3
  （`split_lines` 逐字节，`expected` 验末尾换行，旧 `.ll==新 .ll`，`direct==json==expected`）。
- 未放开：s3/s4/s6/s7/s8 的 JSON lower（留 `emit_s*` 模板全绿）、s1 转义、s5 通用条件/步长/名、
  `for`（一律脱糖 `while`）；`vec/Box/Map/泛型/trait` 不做，`bool` 仅条件与 ok 标志。
- 本草稿不接 `check.sh`（`hzir_diff.py` 头注明）；门禁命令见其头注释。
