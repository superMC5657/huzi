# hzir —— 文本 LLVM IR 子集发射器（远景 spike）

远景止损 spike：Huzi 侧只用字符串拼接发射文本 LLVM IR，
经 `write_file` 落盘 `.ll`，再经 `process_run("clang")` 汇编链接。
全程不碰编译器后端，只验证“文本 IR 路径”在值语义子集上的可行性。

## 范围（冻结）

- 允许：`i32` / 算术 `+ - * / %` / `fn` / `let`（含 `mut` 与遮蔽）/ `while` / `if-else` / `print`
  加上发射必需的胶水：`str` 字面量、`concat`、`write_file`、`process_run`、`trim`
 （P4-a lower 复用既有原语：`read_file/read_file_ok` + `len/substring/to_string/parse_int/contains`，零新增）。
- 禁止：`vec` / `Box` / `Map` / 泛型 / `trait`；禁加 builtin 续命。
- 发射器本体（`src/main.hz` + `src/jlower*.hz`）与 8 个子集（`cases/*.hz`）均遵守以上范围。
  检查：`grep -rn "vec\|Box\|Map\|trait\|impl" src cases` 应无命中
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

`cases/*.hz` 为 huzc 直编译源，`expected/*.stdout` 为期望输出
（含末尾换行），`src/main.hz` 为 Huzi 侧发射器（8 个 `emit_s*` 硬编码 IR 模板）。
P4-a 新增 JSON lower：`json/*.json`（P2-b 落盘，s1/s2/s5 三快照）经
`src/jlower.hz`（core 解析 + 通用算术发射）+ `src/jlower_cases.hz`（分用例 lower）
产出 `target/hzir_s*N*j.ll`，与模板 IR 逐字节一致（s1/s2/s5 已验），其余 5 用例留模板。

## 文本 IR 约定

- 仅用 `ptr` 不透明指针 + `printf`（`%s` / `%d` 带 `\0A` 换行），无目标 triple，`clang` 自动推断。
- 每个 Huzi `print(x)` 对应一次 `printf`（单参子集，无需多参拼接）。
- `let` / `mut` 对应 `alloca + store + load`；遮蔽用 `%x` / `%x1` 双槽。
- `if` 对应 `icmp + br`；`while` 对应 `loop.cond / loop.body / loop.end` 三块。
- `fn` 直译为 `define i32 @name(i32 ...)`，调用为 `call i32 @name(...)`。

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

# 3. 对拍（双编译 stdout 一致，草稿不入门禁）
python examples/hzir/hzir_diff.py huzc/target/debug/huzc.exe examples/hzir/target/hzir.exe
# 3/3：direct==json==expected（split_lines）+ 旧.ll==新.ll + expected 末尾换行
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
| `src/main.hz` | 发射器：8 个 `emit_s*` + `check_one` + `run_case(prefix)` + `run_json_*` + `main`（旧 8 + 新 3 自检） |
| `src/jlower.hz` | P4-a core：`find/skip/expect/parse_jstr/parse_jnum` + 通用算术发射（`tmp` 线程化） |
| `src/jlower_cases.hz` | P4-a 分用例 lower：`lower_s1/s2/s5_json` + `s5_vals*`（s1/s2/s5 冻结形状） |
| `cases/*.hz` | 8 个子集 Huzi 源（huzc 直编译用） |
| `json/*.json` | P2-b JSON 快照（s1/s2/s5，`huzc --dump-ast-json` 落盘，LF + 末尾换行） |
| `expected/*.stdout` | 8 个期望输出（含末尾换行，供 `diff`） |
| `hzir_diff.py` | P4-a 对拍草稿（`direct==json==expected` + 旧 `.ll==新 .ll`，不接 `check.sh`） |
| `README.md` | 本文件 |
| `RFC.md` | 冻结子集 + P4-a §4 增补（每用例一行，s1/s2/s5 已开） |
