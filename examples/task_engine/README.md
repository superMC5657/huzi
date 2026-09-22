# Huzi 综合业务 Demo：任务调度与多维指标分析引擎 (Task Engine)

本项目是一个基于 **Huzi 编程语言** 及官方默认标准库 **`huzi-src`** 构建的综合性业务级 Demo 工程。
项目完全采用官方包管理清单 **[`huzi.toml`](huzi.toml)** 维护工程配置与编译入口。由于 `huzi-src` 是 Huzi 工具链默认内置的标准库，**无需在 `huzi.toml` 的 `dependencies` 中额外声明**，直接开箱即用。

---

## 🌟 核心特性与技术矩阵

本 Demo 将实际任务流水线与指标分析场景作为载体，全方位调用了以下语言与标准库特性：

### 1. Huzi 语言核心语法特性
| 特性 | 代码位置 / 场景 |
|---|---|
| **模块化与多文件组织** | [`src/models.hz`](src/models.hz) 拆分业务领域模型，[`src/main.hz`](src/main.hz) 导入并编排调度 |
| **结构体 (`struct`)** | 声明 `struct Task` 承载业务实体数据（ID、名称、类别、优先级、业务量） |
| **Trait 与静态分发** | 声明 `trait Formattable` 与 `trait Prioritizable`，通过 `impl` 为结构体注入方法调用 (`t.format_summary()`, `t.is_urgent()`) |
| **带载荷枚举 (Payload ADT)** | `enum TaskState { Pending, Processing(str), Done(i32, str), Failed(str) }` 精确描述任务生命周期状态 |
| **模式匹配 (`match`)** | `match state { ... }` 深入解构变体内部关联的 Worker 工号、耗时数值、错误描述文本 |
| **延迟善后保证 (`defer`)** | 注册逆序 (LIFO) 延迟任务：在方法退出时自动计算并输出总体运行时长、释放工作流锁与缓存 |
| **动态容器与哈希映射** | `vec<i32>` 动态数组；`map_new()`、`map_put()`、`map_get()`、`map_has()` 哈希表用于分类聚合 |

### 2. `huzi-src` 官方默认标准库能力
| 标准库分层与模块 | 导入语句 | 核心调用与业务作用 |
|---|---|---|
| **`std.log`** | `import std.log` | `log::info(...)` 输出结构化时间戳业务日志 |
| **`std.timex`** | `import std.timex` | `timex::now_str()` 获取格式化时间，`timex::now_ts()` 与 `timex::elapsed_ms()` 统计流水线耗时 |
| **`std.fsx`** | `import std.fsx` | `fsx::write_lines(...)` 与 `fsx::read_lines(...)` 实现磁盘报表文件的高效写入与读取 |
| **`std.json`** | `import std.json` | `json::stringify_map(...)` 序列化分类汇总字典；`json::parse_i32_map(...)` 反序列化还原并验证 |
| **`alloc.vec_algo`** | `import alloc.vec_algo` | 向量算法：`vec_sort_i32` 原地快排、`vec_sum_i32` 累计总和、`vec_max_i32`/`vec_min_i32` 计算极值、`vec_binary_search_i32` 二分搜索 |
| **`alloc.stringx`** | `import alloc.stringx` | `stringx::repeat(...)` 与 `stringx::pad_right(...)` 格式化对齐终端看板表格 |
| **`alloc.queue`** | `import alloc.queue` | `queue::queue_push(...)` 与 `queue::queue_pop(...)` 模拟 FIFO 任务调度流水线 |
| **`core.assert`** | `import core.assert` | `assert::assert_true(...)` 与 `assert::assert_eq_i32(...)` 对算法与持久化还原数据进行端到端校验 |

---

## 📦 项目清单配置 ([`huzi.toml`](huzi.toml))

标准库由编译器直接解析，清单保持最简、最清晰的纯净结构：

```toml
[package]
name = "task_engine"
version = "0.1.0"
entry = "src/main.hz"
```

---

## 🛠️ 构建与运行指南

### 1. 使用项目清单一键编译
利用 `huzc build` 自动识别当前目录的 `huzi.toml` 进行编译：

```bash
# 在仓库根目录执行
huzc/target/debug/huzc build --path examples/task_engine
```

或直接进入 Demo 目录执行：
```bash
cd examples/task_engine
../../huzc/target/debug/huzc build
```

编译成功后将在 `examples/task_engine` 目录下生成可执行文件 `task_engine.exe`（Linux/macOS 下为 `task_engine`）。

### 2. 运行流水线 Demo

```bash
cd examples/task_engine

# Windows
./task_engine.exe

# Linux / macOS
./task_engine
```

### 3. 运行输出效果预览

```text
[INFO] 2026-09-19 02:22:00 - === 任务引擎与多维指标分析流水线启动 [2026-09-19 02:22:00] ===
[INFO] 2026-09-19 02:22:00 - --- [Trait] 测试实体摘要与优先级判定 ---
[INFO] 2026-09-19 02:22:00 - Task#101 [SyncUsers] (network, 业务量: 250)
[INFO] 2026-09-19 02:22:00 -   -> 是否紧急任务: true
[INFO] 2026-09-19 02:22:00 - Task#103 [CompressLog] (io, 业务量: 120)
[INFO] 2026-09-19 02:22:00 -   -> 是否紧急任务: false
[INFO] 2026-09-19 02:22:00 - --- [Queue] 任务推入 FIFO 调度队列 ---
[INFO] 2026-09-19 02:22:00 - 队列初始化就绪，待调度任务数: 6
[INFO] 2026-09-19 02:22:00 - 队列调度完毕，成功派发任务数: 6
[INFO] 2026-09-19 02:22:00 - --- [Enum & Match] 任务状态机流转与模式匹配 ---
[INFO] 2026-09-19 02:22:00 - 状态测试 1: 待调度
[INFO] 2026-09-19 02:22:00 - 状态测试 2: 执行中 [Worker-Alpha]
[INFO] 2026-09-19 02:22:00 - 状态测试 3: 成功 (88ms, 数据校验和验证通过)
[INFO] 2026-09-19 02:22:00 - 状态测试 4: 失败 (上游网关连接超时)
[INFO] 2026-09-19 02:22:00 - --- [vec_algo] 业务指标快速排序、求和、极值与二分查找 ---
[INFO] 2026-09-19 02:22:00 - 业务量总和: 4060
[INFO] 2026-09-19 02:22:00 - 单笔最大值: 1500
[INFO] 2026-09-19 02:22:00 - 单笔最小值: 120
[INFO] 2026-09-19 02:22:00 - 快速排序完成，升序首项: 120，末项: 1500
[INFO] 2026-09-19 02:22:00 - 二分查找业务量 800 的目标索引: 3
[INFO] 2026-09-19 02:22:00 - --- [Map] 业务类别多维聚合 ---
[INFO] 2026-09-19 02:22:00 - 聚合类别数: 4
[INFO] 2026-09-19 02:22:00 - 金融类别合计: 1230
[INFO] 2026-09-19 02:22:00 - --- [JSON & File I/O] 报表持久化与还原校验 ---
[INFO] 2026-09-19 02:22:00 - 生成的聚合报表 JSON: {"compute":2460,"finance":1230,"io":120,"network":250}
[INFO] 2026-09-19 02:22:00 - 报表已写入: task_report.json
[INFO] 2026-09-19 02:22:00 - 磁盘报表读取并解析校验成功，数据完整无误！

======================================================================
                智能任务调度与指标分析看板
======================================================================
ID      任务名       类别      业务量 紧急度 状态
----------------------------------------------------------------------
101     SyncUsers       network     250       5         已完成
102     CalcPayroll     finance     800       4         已完成
103     CompressLog     io          120       2         已完成
104     TrainModel      compute     1500      5         已完成
105     AuditLedger     finance     430       3         已完成
106     RenderVideo     compute     960       4         已完成
----------------------------------------------------------------------
汇总指标 | 任务总数: 6 | 总业务量: 4060 | 单笔极值: 1500 | 状态: 全部正常
======================================================================

[INFO] 2026-09-19 02:22:00 - --- [DEFER] 释放工作流锁与内存缓存池 ---
[INFO] 2026-09-19 02:22:00 - === 任务引擎工作流安全关闭，总耗时: 0 ms ===
```
