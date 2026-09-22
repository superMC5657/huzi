# hzparse —— 用 Huzi 写的 Huzi 语法分析器子集

自举(bootstrap)里程碑演示之二：继 `hzlex`（词法）之后，用 Huzi 语言写出
编译器前端的第二个组件——递归下降语法分析器子集（表达式 + `fn`/`struct`
子集，另含 `enum`/`trait`/`impl`/`match` 结构形状），并让它完整解析 Huzi
自己的整个生态。

## 验证结果

- `hzparse --selftest`：单元断言（词法复用、`fn`/表达式、`struct`/`enum`/
  `trait`/`impl`/`match`、泛型/`vec<T>()`/`box`/`null`/`?`/方法调用/元组下标/
  `if` 表达式，外加残缺输入负例）全过；
- 对 `huzc/test/cases/*.hz`（62 个）+ `mods/*.hz`（3 个）+ `huzi-src`
  全部源码及自测（34 个）+ `examples/hzlex|task_engine` 源码（4 个）执行
  “词法 + 语法”两段分析：**101 个文件全部通过，零失败**（验收线 ≥ 90）。

## 用法

```bash
# 在仓库根目录执行（编译器在 huzc/target/debug/huzc.exe）
huzc/target/debug/huzc build --path examples/hzparse
cd examples/hzparse
./hzparse --selftest          # 内置断言 + 全语料自解析
./hzparse ../hzlex/src/main.hz  # 单文件：打印 "ok <path> fns=.. structs=.. ..."
```

从仓库根直接跑自测亦可（二进制会自动探测 `huzc/test/...` 相对路径）：

```bash
./examples/hzparse/hzparse --selftest
```

## 文件地图

| 文件 | 说明 |
|---|---|
| `huzi.toml` | 包清单（`build` 入口 `src/main.hz`，标准库免声明） |
| `RFC.md` | 一页冻结语法子集（EBNF + IN/OUT + 验收线） |
| `src/lexer.hz` | 与 `hzlex` 同源、经 `huzc fmt` 归一化的词法器（复用 token 约定，不重造轮子） |
| `src/parser.hz` | 递归下降子集：游标原语 → 类型 → 表达式优先级链 → 语句 → 入口 |
| `src/corpus.hz` | 101 个自解析语料路径清单 |
| `src/main.hz` | CLI + 自测断言（风格对标 `hzlex` 的 `selftest`） |

## 语义范围

与 `huzc/crates/huzi-parser` 对齐的结构形状：`import`（`.`/`::`）、`export`
（含 `::*`）、泛型 `fn`/`struct`（`<T, ...>`）、数组/元组/`vec<T>`/`Box<T>`/
`Result<T>` 等类型、全部表达式优先级（含 `?` 后缀、`box`/`null`、结构体字面量、
`Enum::V(args)`、方法调用、`.0` 元组下标）、`if/elif/else`（语句与表达式两种位置）、
`for in`（含 `..` 范围与 `vec` 遍历）、`while`、`defer`、`match`（结构形状）。

刻意简化（见 `RFC.md` 第 3 节）：无 `match` 穷尽检查、无泛型求解、无
`defer`/`return` 上下文检查、无错误恢复（首错即停，对标 `Parser::parse`）。

## 实现备注：只用已落地 API，无缺口

游标与深度用单元素 `vec<i32>` 做可变单元（句柄语义共享），实现回溯与
`<` 泛型/比较二义消解；未新增任何 builtin，未改编译器与 `huzi-src`。
