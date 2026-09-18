# Huzi 标准库 (huzi-src)

版本：`0.1.0`

`huzi-src` 是 Huzi 语言官方自举标准库（Self-hosted Standard Library），对标 Rust 的 `library/` 体系。编译器内置仅保留系统底层机制（LLVM 原语、RC 引用计数、Socket 与线程原语、系统调用），凡可用 Huzi 语言本身编写的数据结构与算法均收录于此。

## 目录分层

```
huzi-src/
├── README.md          # 分层架构说明、边界规范与版本索引
├── core/              # 核心无堆层：纯逻辑约定与无依赖断言
│   ├── assert.hz      # assert_true / assert_eq_*
│   ├── result.hz      # Result 约定与 unwrap_or
│   └── option.hz      # Option 约定与 unwrap_or
├── alloc/             # 内存与容器层：依赖堆分配算法
│   ├── vec_algo.hz    # 向量排序、二分查找、反转等
│   ├── stringx.hz     # 字符串高阶扩展 (join/repeat/starts_with)
│   ├── mapx.hz        # Map 默认值与合并
│   └── queue.hz       # 栈与队列组合结构
├── std/               # 系统与 I/O 层：薄封装 OS 能力
│   ├── fsx.hz         # 文件行读写与路径工具
│   ├── env_cli.hz     # 环境变量与命令行参数解析
│   ├── timex.hz       # 本地时间格式化与耗时统计
│   ├── log.hz         # 分级日志输出
│   ├── framing.hz     # TCP 行帧封包收发
│   ├── csv.hz         # CSV 格式解析
│   └── json.hz        # 平面 JSON 序列化与解析 (仅 ASCII)
└── test/              # 库单元与集成自测试
    ├── core_test.hz
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
2. **模块入口规则**：
   - 本库不使用 `mod.hz` 聚合入口文件。Huzi 语言目前无 re-export 导出机制，`import` 直接绑定目标文件末段名，所有模块使用点分全路径导入（如 `import core.result`、`import alloc.vec_algo`）。

## 引用方式

消费方项目在 `huzi.toml` 中通过 `path` + `version` 精确钉住版本：

```toml
[package]
name = "my_project"
version = "0.1.0"
entry = "src/main.hz"

[dependencies]
huzi_std = { version = "0.1.0", path = "../huzi-src" }
```
