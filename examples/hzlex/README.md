# hzlex —— 用 Huzi 写的 Huzi 词法器

自举(bootstrap)里程碑演示:用 Huzi 语言重写了编译器前端的第一个
组件——词法分析器,并让它完整解析 Huzi 自己的整个生态。

## 验证结果

- `hzlex --selftest`:内置断言覆盖关键字(含 `weak`)、标识符、`->`、
  字符串转义、浮点数、`1..2` 范围点、`t.1.2` 元组索引链与 eof;
- 对 `huzc/test/cases/` 顶层 `*.hz`(74 个)与 `huzi-src` 递归全部
  `*.hz`(38 个,含 21 个 `lib.hz` 与 17 个自测)执行词法分析:
  **112 个文件全部通过,零失败**。

## dump 格式(冻结,与 `huzc --dump-tokens` 对拍用)

每 token 一行 `行:列 类型 文本`(单空格分隔,含 eof 行;eof 文本为空,
行尾带一个空格)。类型集:`kw | ident | int | float | string | char |
punct | eof`(f-string 归入 `string`,去前缀 `f`)。文本口径:string 为
解转义后原始字节(直接打印,不转义回显);char 为码点十进制;其余为源码
拼写。列号按字节计。完整冻结定义与已知差异见 `src/main.hz` 与
`src/lexer.hz` 头注释(多字节列号、float 拼写两处与 Rust 侧已知不一致,
待 Rust 侧对齐,本侧不改)。

## 对拍

```bash
python examples/hzlex/dump_diff.py ./huzc/target/debug/huzc ./examples/hzlex/hzlex huzc/test/cases
python examples/hzlex/dump_diff.py ./huzc/target/debug/huzc ./examples/hzlex/hzlex huzi-src
```

脚本只做文本对拍(逐文件跑双方 dump 并 `diff --strip-trailing-cr`,
汇总 PASS/FAIL 计数,有 FAIL 即非零退出),不解析语义。交叉对拍的
validation owner 为 orchestrator(需等 Rust 侧 `--dump-tokens` 落盘)。

## 用法

```bash
huzc -i examples/hzlex/src/main.hz -o hzlex
./hzlex --selftest          # 内置断言
./hzlex some_file.hz        # 逐行打印 "行:列 类型 文本"
```

## 语义范围

与 `huzc/crates/huzi-lexer` 对齐的部分:`//` 与 `#` 行注释、双字符
运算符(`== != <= >= && || -> => :: ..`)、整数/小数(`1..2` 识别为
int + `..` + int)、字符串转义(`\n \t \r \" \\ \0`,其余转义取该
字节本身)、字符字面量、标识符规则。

刻意简化(不影响演示目的):非 ASCII 字节在标识符位置归入标识符
 continuation;`1.x` 类悬空小数点按 Rust 词法器 prev_was_dot 语义处理(点号后不再吞 float),
见源码注释;关键字表含 `weak`。
