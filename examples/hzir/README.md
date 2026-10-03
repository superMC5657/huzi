# hzir —— 文本 LLVM IR 子集发射器（远景 spike）

远景止损 spike：Huzi 侧只用字符串拼接发射文本 LLVM IR，
经 `write_file` 落盘 `.ll`，再经 `process_run("clang")` 汇编链接。
全程不碰编译器后端，只验证“文本 IR 路径”在值语义子集上的可行性。

## 范围（冻结）

- 允许：`i32` / 算术 `+ - * / %` / `fn` / `let`（含 `mut` 与遮蔽）/ `while` / `if-else` / `print`
  加上发射必需的胶水：`str` 字面量、`concat`、`write_file`、`process_run`、`trim`
 （P4-a lower 复用既有原语：`read_file/read_file_ok` + `len/substring/to_string/parse_int/contains`，零新增）。
- 禁止：`Box` / `Map`；`vec` 仅 s11 一种堆版经 RFC §6 放开；
  泛型仅 s12 单态化经 RFC §7 放开；`trait` 仅 s13 修饰直调经 RFC §7 放开；禁加 builtin 续命。
- 发射器本体与子集均遵守以上范围（放开项逐项见 RFC 增补行）。
  检查：`grep -rn "Box\|Map" src cases` 应无命中（全禁保持）；
  `trait\|impl` 仅 `cases/s13_trait.hz` 有命中（RFC §7）；
  `vec` 仅 s11 三处有命中（`src/emit_vec.hz` + `cases/s11_vec.hz` + `src/main.hz` 接线）。
  （`mismatch` 一词已避开，见源码注释）。

## 8 子集爬坡（oracle 冻结顺序）

| # | 名 | 内容 | 期望 stdout |
|---|---|---|---|
| 1 | hello | `print("hello hzir")` | `hello hzir` |
| 2 | arith | `3+4*2` / `(3+4)*2` / `10-6/2` / `10%3` | `11 / 14 / 7 / 1` |
| 3 | shadow | `let x=10` 后 `let x=20` 遮蔽 + `mut y` 累加 | `10 / 20 / 11` |
| 4 | max | `fn max(a,b)` + `if-else` 三次调用 | `20 / 50 / 7` |
| 5 | sum | `while i<=10` 累加 | `55` |
| 6 | fact | `while n>1` 阶乘 | `120` |
| 7 | call | `add` / `mul` / `add_mul` 互调 | `7 / 12 / 17` |
| 8 | fib | `fib` 迭代（含 `if` 早退 + `while`） | `0 / 1 / 55` |
| 9 | for_range（P4-b） | `for i in 0..5` 逐行打印（脱糖 `while` 三件套） | `0 / 1 / 2 / 3 / 4` |
| 10 | for_sum（P4-b） | `for i in 1..11` 累加（脱糖 `while` 三件套） | `55` |
| 11 | vec（P4-c） | `vec<i32>` 三 push + `len` / 索引 / 求和（`malloc` + 16B 双计数头） | `3 / 20 / 60` |
| 12 | generic（P4-d） | `id<T>` 经 `id<i32>(42)` 文本单态化直调 | `42` |
| 13 | trait（P4-d） | `Wrap{v:7}` 经 `Wrap__get` 修饰名直调（值传参） | `7` |

`cases/*.hz` 为 huzc 直编译源，`expected/*.stdout` 为期望输出
（含末尾换行），`src/main.hz` 为 Huzi 侧发射器（s1-s8 的 `emit_s*` 硬编码 IR 模板 +
P4-b 起 `src/emit_for.hz` 的 s9/s10 范围循环模板，经 `check_one` 同管）。
P4-a 新增 JSON lower：`json/*.json`（P2-b 落盘，s1/s2/s5 三快照）经
`src/jlower.hz`（core 解析 + 通用算术发射）+ `src/jlower_cases.hz`（分用例 lower）
产出 `target/hzir_s*N*j.ll`，与模板 IR 逐字节一致（s1/s2/s5 已验），其余 5 用例留模板。
P4-b 新增模板 s9/s10（`for in ..` 脱糖 `while` 三件套，见 `RFC.md §5`；
`--dump-ast-json` 对 `for` 非零，故无 JSON lower，`hzir_diff.py` 以
`direct==template==expected` + `json-neg` 验收）。

## 文本 IR 约定

- 仅用 `ptr` 不透明指针 + `printf`（`%s` / `%d` 带 `\0A` 换行），无目标 triple，`clang` 自动推断。
- 每个 Huzi `print(x)` 对应一次 `printf`（单参子集，无需多参拼接）。
- `let` / `mut` 对应 `alloca + store + load`；遮蔽用 `%x` / `%x1` 双槽。
- `if` 对应 `icmp + br`；`while` 对应 `loop.cond / loop.body / loop.end` 三块。
- `for i in a..b`（P4-b s9/s10）脱糖为同三件套：`i=a` 初值 + `slt b` 独占上界（步长 `1`），不新增块种类。
- `fn` 直译为 `define i32 @name(i32 ...)`，调用为 `call i32 @name(...)`。
- `vec<i32>`（P4-c s11）：`malloc(16+cap*4)` + `[raw+0]` 弱 `1` / `[raw+8]` 强 `1` 双计数头，
  用户区 `raw+16`；尾部 `free(raw)` 一次回收（无别名无弱引用形状）。
- 泛型（P4-d s12）：调用点文本单态化，`id<T>` → `define i32 @id_i32`，直调（不做约束求解）。
- 方法（P4-d s13）：修饰名直调，`w.get()` → `call i32 @Wrap__get(%struct.Wrap %w0)`，
  结构体值传参 + `extractvalue 0` 取字段（不做分发）。

## 用法

```bash
# 1. 直编译 8 子集（huzc 路径）
for f in examples/hzir/cases/*.hz; do
  huzc/target/debug/huzc -i "$f" -o /tmp/hz_direct
  /tmp/hz_direct > /tmp/direct.out
done

# 2. 文本 IR 路径（Huzi 发射器路径）
huzc/target/debug/huzc build --path examples/hzir --output examples/hzir/target/hzir
examples/hzir/target/hzir
# 发射器内完成：concat -> write_file(target/hzir_s*.ll/.exe)
#   -> process_run("clang ...") -> process_run(exe) -> 对比 want -> 打印 pass
# JSON lower 同管：read_file(json/*.json) -> lower_s*_json -> 同上（s1/s2/s5）
# for 模板同管：emit_for::emit_s9/s10 -> 同上（s9/s10，无 JSON lower）
# vec 模板同管：emit_vec::emit_s11 -> 同上（s11 堆版，无 JSON lower）
# 泛型/方法模板同管：emit_gen::emit_s12/s13 -> 同上（s12/s13，无 JSON lower）

# 3. 对拍（双编译 stdout 一致，草稿不入门禁）
python examples/hzir/hzir_diff.py huzc/target/debug/huzc.exe examples/hzir/target/hzir.exe
# 3/3 JSON：direct==json==expected（split_lines）+ 旧.ll==新.ll + expected 末尾换行
# 5/5 模板（s9/s10/s11/s12/s13）：direct==template==expected + json-neg（dump 须非零）
```

# 3. 对拍（双编译 stdout 一致）
diff /tmp/direct.out <(examples/hzir/target/hzir_s1.exe)  # 以此类推 8 个
```

发射器默认写 `target/hzir_s*.ll` 与 `target/hzir_s*.exe`（仓库内，`target/` 已忽略；
JSON lower 为 `target/hzir_s*N*j.ll/.exe`），前缀自适应仓库根 vs `examples/hzir`。
`clang` 警告 `overriding target triple` 属正常（无 triple 时的提示），不影响退出码。

## 止损

- 任一内存 / 控制流 bug 超 3 天即砍范围，先砍 7→8（多 fn 互调→fib 迭代）。
- 本次 spike 8 个全过，无需砍范围；若后续扩展失败，按此顺序回退。
- 禁加 builtin 续命：缺口只砍范围，不补原语。

## 文件地图

| 文件 | 说明 |
|---|---|
| `huzi.toml` | 包清单（`build` 入口 `src/main.hz`） |
| `src/main.hz` | 发射器：s1-s6 本体 `emit_s*` + `check_one` + `run_case(prefix)`（1~8）+ `run_case_extra`（9~13）+ `run_json_*` + `main`（模板 13 + JSON 3 自检） |
| `src/emit_for.hz` | P4-b 范围循环模板：`emit_s9/s10`（`for in ..` 脱糖 `while` 三件套） |
| `src/emit_fns.hz` | 多函数模板（P4-c 纯搬移）：`emit_s7`（互调） + `emit_s8`（fib 迭代） |
| `src/emit_vec.hz` | P4-c 堆模板：`emit_s11`（`vec<i32>` + `malloc` 16B 双计数头） |
| `src/emit_gen.hz` | P4-d 单形模板：`emit_s12`（泛型单态化 `id_i32`） + `emit_s13`（修饰直调 `Wrap__get`） |
| `src/jlower.hz` | P4-a core：`find/skip/expect/parse_jstr/parse_jnum` + 通用算术发射（`tmp` 线程化） |
| `src/jlower_cases.hz` | P4-a 分用例 lower：`lower_s1/s2/s5_json` + `s5_vals*`（s1/s2/s5 冻结形状） |
| `cases/*.hz` | 13 个子集 Huzi 源（huzc 直编译用；s9/s10 为 P4-b `for` 模板，s11 为 P4-c `vec` 堆模板，s12/s13 为 P4-d 泛型/方法模板） |
| `json/*.json` | P2-b JSON 快照（s1/s2/s5，`huzc --dump-ast-json` 落盘，LF + 末尾换行） |
| `expected/*.stdout` | 13 个期望输出（含末尾换行，供 `diff`） |
| `hzir_diff.py` | 对拍草稿（JSON 3：`direct==json==expected` + 旧 `.ll==新 .ll`；模板 s9~s13：`direct==template==expected` + `json-neg`，不接 `check.sh`） |
| `README.md` | 本文件 |
| `RFC.md` | 冻结子集 + P4-a §4 增补（每用例一行，s1/s2/s5 已开）+ P4-b §5（s9/s10 `for` 模板）+ P4-c §6（s11 `vec` 堆模板）+ P4-d §7（s12/s13 泛型/方法模板） |
