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

## 5. P4-b 增补：`for in ..` 模板（s9/s10，JSON 仍关）

- IN：`cases/s9_for_range.hz`（`for i in 0..5 { print(i) }` → `0/1/2/3/4`）
  + `cases/s10_for_sum.hz`（`for i in 1..11` 累加 → `55`）。
- 放开行（每用例一行）：
  - s9_for_range：仅 `for i in 0..5` 定形（初值锁 `0`、上界锁 `5`、`slt` 独占），体为 `print(i)` 单句。
  - s10_for_sum：仅 `for i in 1..11` 累加定形（初值锁 `1`、`slt 11` 独占、名锁 `sum/i`），
    体为 `sum=sum+i` / `i=i+1` 双句，尾 `print(sum)`。
- 脱糖（只语法糖）：`for i in a..b` 一律糖为 `i=a` 初值 + `loop.cond（slt b）`
  + `loop.body` + `loop.end` 三件套（与 `while` 同标签，不新增 IR 块种类；
  `..` 上界独占，步长锁 `1`）。
- OUT：`target/hzir_s9/s10.ll/.exe`（`src/emit_for.hz` 模板，经 `check_one` 同管）。
- JSON：`--dump-ast-json` 对 `for` 非零（P1 外，见 `examples/hzparse/RFC_P2B_DRAFT.md §2`
  超子集）；本期不做 `for` 的 JSON lower（`src/jlower*.hz` 不动），
  `hzir_diff.py` 以 `direct==template==expected` + `json-neg`（dump 须非零）验收，
  自检标记变为 `hzir all 10 pass` + `hzir json 3 pass`。
- 未放开：`for x in vec/调用` 集合遍历（留待 P4-c）、通用初值/上界/步长/变量名、
  `break/continue`；`vec/Box/Map/泛型/trait` 不做。

## 6. P4-c 增补：`vec<i32>` 堆模板（s11，JSON 仍关）

- IN：`cases/s11_vec.hz`（`vec<i32>` 三 push `10/20/30` → `len=3` / `v[1]=20` / 和 `60`）。
- 放开行：
  - s11_vec：仅此一种堆版（`push` 定值 `10/20/30`、`print(len)` / `print(v[1])` /
    `print(v[0]+v[1]+v[2])` 三句，名锁 `v`）。
- 堆语义（对标 STATUS 双计数 16B 头，不做完整 RC 机）：
  - `malloc(16 + cap*4)`（`cap` 锁 `4` → `32B`）；`[raw+0]` 弱计数 `i64=1`、
    `[raw+8]` 强计数 `i64=1`；用户区起于 `raw+16`。
  - 本形无别名、无弱引用，计数恒 `1`，尾部 `free(raw)` 一次回收全部
    （两阶段坍缩为一次释放；`free_vec` 语义由编译器直编译侧覆盖）。
  - 超此即砍回栈版：若 `realloc`/扩容/别名 `retain`/`drop` 任一超期，s11 回退为
    `[3 x i32]` 栈数组模板（本次 spike 未触发，直接堆版全绿）。
- OUT：`target/hzir_s11.ll/.exe`（`src/emit_vec.hz` 模板，经 `check_one` 同管）。
- JSON：`--dump-ast-json` 对 `vec` 非零（P1 外，`expr outside subset`）；不做 JSON lower；
  `hzir_diff.py` 以 `direct==template==expected` + `json-neg` 验收，
  自检标记变为 `hzir all 11 pass` + `hzir json 3 pass`。
- 未放开：`vec` 非 `i32` 元、元素为结构体、`Box`/`Map`、扩容/`realloc`、
  `free_vec/ref_count/weak_count` 文本建模、`for x in v` 通用脱糖；
  其他容器无模板项（其 `--dump-ast-json` 同为非零，hzir 侧天然未覆盖）。

## 7. P4-d 增补：泛型单态化 + 方法修饰直调（s12/s13，JSON 仍关）

- IN：`cases/s12_generic.hz`（`id<T>` 经 `id<i32>(42)` → `42`）
  + `cases/s13_trait.hz`（`Wrap{v:7}` 经 `w.get()` → `7`）。
- 放开行：
  - s12_generic：仅 `id<T>(x:T)->T` 单形参单调用点（实参锁 `i32`/`42`），
    模板为文本单态化 `define i32 @id_i32` 直调（`f<T>→f_i32` 改名，不做约束求解）。
  - s13_trait：仅 `Wrap{i32}` + `Get::get` 单方法单调用点（字段锁 `v`/`7`），
    模板为修饰名直调 `define i32 @Wrap__get(%struct.Wrap)`（值传参，
    `extractvalue 0` 取字段，不做分发与约束求解）。
- OUT：`target/hzir_s12/s13.ll/.exe`（`src/emit_gen.hz` 模板，经 `check_one` 同管；
  `main.hz` 拆出 `run_case_extra` 承接 9~13，`run_case` 只留 1~8，保 70 行）。
- JSON：两者 `--dump-ast-json` 皆非零（泛型调用 / `struct` 皆 P1 外）；不做 JSON lower；
  `hzir_diff.py` 以 `direct==template==expected` + `json-neg` 验收，
  自检标记变为 `hzir all 13 pass` + `hzir json 3 pass`。
- 未放开：多类型参数/多调用点、泛型结构体/枚举、`where` 约束、特化、
  多方法/多 trait/默认方法、动态分发；`Box`/`Map` 不做。
