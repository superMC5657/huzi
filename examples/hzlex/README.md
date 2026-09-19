# hzlex —— 用 Huzi 写的 Huzi 词法器

自举(bootstrap)里程碑演示:用 Huzi 语言重写了编译器前端的第一个
组件——词法分析器,并让它完整解析 Huzi 自己的整个生态。

## 验证结果

- `hzlex --selftest`:内置断言覆盖关键字、标识符、`->`、字符串转义、
  浮点数、`1..2` 范围点与 eof;
- 对 `huzc/test/examples/*.hz`(62 个)与 `huzi-src` 全部源码及自测
  (28 个)执行词法分析:**90 个文件全部通过,零失败**。

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
 continuation;`1.x` 类悬空小数点按 Rust 词法器对齐处理差异已
文档化于源码注释。
