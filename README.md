# Huzi 标准库 (huzi-src)

版本：`0.1.0`

`huzi-src` 是 Huzi 语言官方自举标准库（Self-hosted Standard Library），对标 Rust 的 `library/` 体系。编译器内置仅保留系统底层机制（LLVM 原语、RC 引用计数、Socket 与线程原语、系统调用），凡可用 Huzi 语言本身编写的数据结构与算法均收录于此。

## 目录分层

```
huzi-src/
├── README.md          # 分层架构说明、边界规范与版本索引
├── core/              # 核心无堆层：纯逻辑约定与无依赖断言
│   ├── lib.hz         # 包规范库入口 (统一 export 子模块与符号)
│   ├── huzi.toml      # 包元数据清单
│   ├── assert/
│   │   └── lib.hz     # assert_true / assert_eq_*
│   ├── result/
│   │   └── lib.hz     # Result 约定与 unwrap_or
│   └── option/
│       └── lib.hz     # Option 约定与 unwrap_or
├── alloc/             # 内存与容器层：依赖堆分配算法
│   ├── lib.hz         # 包规范库入口 (统一 export 子模块与符号)
│   ├── huzi.toml      # 包元数据清单
│   ├── vec_algo/
│   │   └── lib.hz     # 向量排序、二分查找、反转等
│   ├── stringx/
│   │   └── lib.hz     # 字符串高阶扩展 (join/repeat/starts_with)
│   ├── mapx/
│   │   └── lib.hz     # Map 默认值与合并
│   └── queue/
│       └── lib.hz     # 栈与队列组合结构
├── std/               # 系统与 I/O 层：薄封装 OS 能力
│   ├── lib.hz         # 包规范库入口 (统一 export 子模块与符号)
│   ├── huzi.toml      # 包元数据清单
│   ├── fsx/
│   │   └── lib.hz     # 文件行读写与路径工具
│   ├── env_cli/
│   │   └── lib.hz     # 环境变量与命令行参数解析
│   ├── timex/
│   │   └── lib.hz     # 本地时间格式化与耗时统计
│   ├── log/
│   │   └── lib.hz     # 分级日志输出
│   ├── framing/
│   │   └── lib.hz     # TCP 行帧封包收发
│   ├── csv/
│   │   └── lib.hz     # CSV 格式解析与序列化
│   └── json/
│       └── lib.hz     # 平面 JSON 序列化与解析 (仅 ASCII)
└── test/              # 库单元与集成自测试
    ├── core_test.hz
    ├── package_entry_test.hz
    ├── vec_algo_test.hz
    ├── stringx_test.hz
    ├── fsx_test.hz
    ├── timex_test.hz
    ├── env_cli_test.hz
    ├── csv_test.hz
    ├── json_test.hz
    ├── framing_test.hz
    ├── mapx_test.hz
    ├── queue_test.hz
    ├── log_test.hz
    └── kv_store_test.hz
```

## 架构边界说明

1. **什么不进 `huzi-src`**：
   - 底层运行时：RC 引用计数分配与释放（`free_*`）、底层 panic 终止处理、TCP 原始 socket 及 OS 线程创建（`spawn`）由 `huzc/crates/huzi-codegen` 直接提供。
   - 不设空壳目录：当前阶段不支持的 SIMD、过程宏等特性不预建占位目录。
2. **规范库入口与零开销重导出（Re-export）**：
   - 各标准子包均对标 Rust `lib.rs` 规范，提供 `lib.hz`（由 `huzi.toml` 的 `lib_entry` 指定）。
   - 通过 `export <submod>`（暴露子模块层级）与 `export <submod>::*`（通配扁平化导出）声明对外 API，在编译期由 LLVM Codegen 直接生成符号别名（零运行时包装函数开销）。
   - **完全向下兼容**：既支持包级导入，也支持传统的子路径直接导入。

## 引用方式

`huzi-src` 是 Huzi 语言官方自举标准库，由编译器 `huzc` 默认探测与解析（解析优先级：环境变量 `HUZI_LIB` → 编译器相邻目录 `../huzi-src` → 工作区相对路径 → `~/.huzi/huzi-src`）。

消费方项目**无需在 `huzi.toml` 的 `[dependencies]` 中额外声明标准库**。

### 方式一：顶级包导入（推荐，统一入口）

```huzi
import core
import alloc
import std

fn main() -> i32 {
    // 直接调用扁平重导出的顶层符号
    core::assert_true(true)
    let sorted = alloc::vec_sort_i32(v)
    let escaped = std::escape_quotes("hello")

    // 或通过子模块路径限定访问
    core::assert::assert_eq_i32(10, 10)
    let joined = alloc::stringx::join(parts, "-")
    let record = std::log::format_record("INFO", "2026", "ok")
    return 0
}
```

### 方式二：细粒度全路径导入（保留向下兼容）

```huzi
import std.log
import std.fsx
import alloc.vec_algo
import core.assert
```

消费方项目的 `huzi.toml` 保持纯净，仅在引入第三方外部扩展包时配置 `[dependencies]`：

```toml
[package]
name = "my_project"
version = "0.1.0"
entry = "src/main.hz"
```
