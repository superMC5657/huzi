# Huzi 标准库（huzi-src）

版本 `0.1.0`。用 Huzi 自身编写的官方标准库；编译器只保留系统底层能力（内存分配释放、socket、线程创建等），其余数据结构与算法都写在这里。

## 三层结构

| 层 | 内容 |
|----|------|
| `core` | 无堆纯逻辑：断言、Result、Option |
| `alloc` | 堆容器算法：向量排序、字符串扩展、Map 扩展、栈与队列 |
| `std` | 系统能力封装：文件、HTTP、环境变量、时间、日志、TCP 封包、CSV、JSON |

每层的入口都是 `lib.hz`，对外 API 经 `export` 重导出。

## 怎么用

标准库开箱即用，无需在 `huzi.toml` 里声明：

```huzi
import core
import alloc
import std

fn main() -> i32 {
    core::assert_true(true)
    let joined = alloc::stringx::join(parts, "-")
    let record = std::log::format_record("INFO", "2026", "ok")
    return 0
}
```

也可以按模块导入：`import std.log`、`import alloc.vec_algo`、`import core.assert`。
