# hzir —— 文本 LLVM IR 子集发射器（远景 spike）

远景止损 spike：Huzi 侧只用字符串拼接发射文本 LLVM IR，
经 `write_file` 落盘 `.ll`，再经 `process_run("clang")` 汇编链接。
全程不碰编译器后端，只验证“文本 IR 路径”在值语义子集上的可行性。

## 范围（冻结）

- 允许：`i32` / 算术 `+ - * / %` / `fn` / `let`（含 `mut` 与遮蔽）/ `while` / `if-else` / `print`
  加上发射必需的胶水：`str` 字面量、`concat`、`write_file`、`process_run`、`trim`。
- 禁止：`vec` / `Box` / `Map` / 泛型 / `trait`；禁加 builtin 续命。
- 发射器本体（`src/main.hz`）与 8 个子集（`cases/*.hz`）均遵守以上范围。
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
huzc/target/debug/huzc build --path examples/hzir --output /tmp/hzir
/tmp/hzir
# 发射器内完成：concat -> write_file(/tmp/hzir_s*.ll)
#   -> process_run("clang ...") -> process_run(exe) -> 对比 want -> 打印 pass

# 3. 对拍（双编译 stdout 一致）
diff /tmp/direct.out <(/tmp/hzir_s1_exe)  # 以此类推 8 个
```

发射器默认写 `/tmp/hzir_s*.ll` 与 `/tmp/hzir_s*_exe`，免去仓库内建目录。
`clang` 警告 `overriding target triple` 属正常（无 triple 时的提示），不影响退出码。

## 止损

- 任一内存 / 控制流 bug 超 3 天即砍范围，先砍 7→8（多 fn 互调→fib 迭代）。
- 本次 spike 8 个全过，无需砍范围；若后续扩展失败，按此顺序回退。
- 禁加 builtin 续命：缺口只砍范围，不补原语。

## 文件地图

| 文件 | 说明 |
|---|---|
| `huzi.toml` | 包清单（`build` 入口 `src/main.hz`） |
| `src/main.hz` | 发射器：8 个 `emit_s*` + `check_one` + `run_case` + `main`（单文件 328 行，每 fn ≤70 行） |
| `cases/*.hz` | 8 个子集 Huzi 源（huzc 直编译用） |
| `expected/*.stdout` | 8 个期望输出（含末尾换行，供 `diff`） |
| `README.md` | 本文件 |
| `RFC.md` | 一页冻结子集（语法 + IR 映射 + 验收线） |
