# huzi-src 标准库 · 可执行开发计划（对标 Rust library/）

> 目标：`huzi-src/` 成为 Huzi 的 `library/`（见用户截图：`core/alloc/std/test` 分层），编译器内置只留水电（LLVM/RC/socket/线程原语），凡能用 Huzi 写的全放这里。
> 基线：现 `huzi-src/` 仅 `README.md + sample.hz`（`max/min_i32`，且用 `//` 注释）；`vec/map` 已支持参数与字段（`5adbd61`，语法为 `vec<i32>` / 大写 `Map`）；泛型推导已补（`e206301/51b1b47`）。
> 约束：`huzc/AGENTS.md`（单文件 ≤500 行、单函数 ≤70 行；`cargo build` 零警告；`bash test.sh` 全绿；中文一事一提交）+ `huzi-src/README.md` 的 Self-hosted 定位。
> 语言天花板（取舍，非 bug）：无泛型枚举（`Result<T,E>` 做不了）→ 用元组约定；`HashMap` 仅 `str->i32`；字符串字节语义；`spawn` 只传整数；无精确 GC（RC + 手动打破）。
> 本次复核补丁（9 条）：① std 解析根 ② 新类型签名 ③ 自测链路 ④ 删 mod.hz ⑤ 可变语义冻结 ⑥ 注释统一 `#` ⑦ JSON/字节语义预警 ⑧ LSP 同步 ⑨ 版本钉住。

## 目标目录（落地形态）

```
huzi-src/
├── README.md          ← 重写：分层说明 core/alloc/std/test + 边界（什么不进 std）+ 版本号 + huzi.toml 引用方式
├── core/              ← 对应 Rust core：无堆、无 OS
│   ├── result.hz      ← (ok: bool, val: i32, err: str) 约定 + unwrap_or
│   ├── option.hz      ← (has: bool, val) 约定（i32/str 特化各一份，先不泛化）
│   └── assert.hz      ← assert_true/assert_eq(i32/str)，自测与示例公用
├── alloc/             ← 对应 Rust alloc：要堆（vec/Box/str 上层）
│   ├── vec_algo.hz    ← sort/binary_search/reverse/dedup/sum/max/min（i32/str 各一份，先行）
│   ├── mapx.hz        ← get_or_default/merge/contains_key（str->i32 现状版，注意 Map 大写）
│   ├── queue.hz       ← 用 vec<i32> 拼的栈/队列（push/pop/clear 复用内置）
│   └── stringx.hz     ← join/repeat/pad/starts_with/ends_with/lines/split_lines
├── std/               ← 对应 Rust std：要 OS（只调 builtin，不碰 LLVM）
│   ├── fsx.hz         ← exists/read_lines/write_lines/path_join/ext_name
│   ├── env_cli.hz     ← 需 env_get：env_or/args_parse（子命令+flag 最小版）
│   ├── timex.hz       ← 需 localtime/format_time：now_str/elapsed_ms
│   ├── log.hz         ← info/warn/error（依赖 timex，未好前用 print 顶，占位）
│   ├── framing.hz     ← TCP 行帧/长度前缀封包（基于 tcp_send/recv）
│   ├── csv.hz         ← 纯 Huzi 可写：parse_line/stringify
│   └── json.hz        ← 需 parse_int/parse_float：平面对象子集先行，占位（仅 ASCII，见 S3 预警）
└── test/              ← 对应 coretests/alloctests：库自测（经 huzi-src/test.sh 运行，见 S0）
    ├── core_test.hz
    ├── vec_algo_test.hz
    ├── stringx_test.hz
    └── fsx_test.hz
```

说明：`compiler-builtins/backtrace/panic_*` 对应物（RC、`free_*`、`panic`、socket/线程原语）永不进 `huzi-src`，留 `huzc/crates/huzi-codegen`；`proc_macro/simd/stdarch/vendor` 直接没有，不建空壳。**无 `mod.hz` 总入口**：Huzi 无 re-export，`import` 只绑定末段名，聚合文件会误导，已删除，改由 `README.md` 做文档索引。

---

## S0：骨架 + 地基 + 链路 [ ]（约 3-4 天）

- **目标**：目录可用、`import` 可跑、自测链路通。
- **IN**：
  - 建 `core/alloc/std/test` 空骨架 + `README` 重写（含版本号如 `0.1.0` + `huzi.toml` 引用示例）；先写 `core/assert.hz + result.hz + option.hz`。
  - `sample.hz` 注释统一为 `#`（与 `test/examples` 一致，过 `fmt --check`）。
  - **补丁① std 解析根**：`huzc` 新增标准库根解析（优先级建议：`HUZI_LIB` 环境变量 → 可执行文件旁 `../huzi-src` → `vendor/` → `~/.huzi`），实现于 `crates/huzc/src/modules.rs:resolve_module_file_result`，`import core.result` 须解析到 `huzi-src/core/result.hz`。备选（人力不足时）：S0 先用 `huzc fetch` 把 `huzi-src` 装进 `vendor/` 顶过去，但必须在 plan 注明为临时方案。
  - **补丁⑧ LSP 同步**：`crates/huzi-lsp/src/imports.rs:resolve_import_uri` 同步同一解析顺序，否则编辑器跳转/补全对 std import 失效。
  - **补丁③ 自测链路**：新增 `huzi-src/test.sh`（编译运行 `huzi-src/test/*.hz`，读写隔离到 `huzi-src/test/out/`），并在 `huzc/.github/workflows/ci.yml` 加 job 调用它；`huzc/test.sh` 本体不动（只扫 `test/examples + neg`）。
  - **补丁⑨ 版本钉住**：`README` 写版本号；消费方经 `huzi.toml` 的 `path + version` 精确引用（沿用 P6 语义：冲突直接报错，无 semver 求解）。
- **OUT**：不加 builtin；不动语言语义。
- **函数签名（冻结）**：
  ```huzi
  fn assert_true(cond: bool) -> i32
  fn assert_eq_i32(a: i32, b: i32) -> i32
  fn assert_eq_str(a: str, b: str) -> i32
  fn unwrap_or_i32(ok: bool, val: i32, dflt: i32) -> i32
  ```
- **测试**：`huzi-src/test/core_test.hz` 经新 `huzi-src/test.sh` 全过；`huzc/test/examples/` 新增 `50_std_smoke.hz`（`import core.result` 两行即过，验证解析根）。
- **验收**：`cargo build --workspace`（零警告）+ `bash test.sh` + `bash ../huzi-src/test.sh` + `huzc fmt --check huzi-src` 全绿。

## S1：alloc 现写版（纯组合现有 builtin） [ ]（约 1 周）

- **目标**：写程序最疼的三件先止痛：排序、拼串、map 默认值。
- **IN**：仅用现有 `len/concat/split/substring/vec push/pop/map_*` 组合。
- **OUT**：不等 `HashMap` 泛化；`vec` 只做 `i32/str` 两套，不硬上泛型算法。
- **类型签名（已按 `47_container_params.hz` 现行语法修正：`vec<i32>` / 大写 `Map`）**：
  - `alloc/vec_algo.hz`：`fn vec_sort_i32(v: vec<i32>) -> i32`、`fn vec_binary_search_i32(v: vec<i32>, x: i32) -> i32`、`fn vec_reverse_i32(v: vec<i32>) -> i32`、`fn vec_sum_i32(v: vec<i32>) -> i32`
  - `alloc/stringx.hz`：`fn join(v: vec<str>, sep: str) -> str`、`fn repeat(s: str, n: i32) -> str`、`fn starts_with(s: str, p: str) -> bool`
  - `alloc/mapx.hz`：`fn map_get_or(m: Map, k: str, dflt: i32) -> i32`、`fn map_merge(dst: Map, src: Map) -> i32`
  - `alloc/queue.hz`：`fn stack_push(v: vec<i32>, x: i32) -> i32` / `fn stack_pop(v: vec<i32>) -> (bool, i32)` / `fn stack_top(v: vec<i32>) -> (bool, i32)`
- **补丁⑤ 可变语义（冻结）**：`vec`/`Map` 参数为句柄语义，函数内 `push` 直接影响调用方；调用点须 `let mut`，形参不标 `mut`（以 `47` 的 `append_item` 为准）；`vec_sort` 为原地排序，返回 `0` 状态码（沿用 `push/clear` 约定），不返回新 vec。
- **测试**：`huzi-src/test/vec_algo_test.hz + stringx_test.hz`（经 `huzi-src/test.sh`）；`huzc/test/examples/51_alloc_demo.hz`（排序+拼串端到端，经主 `test.sh`）。
- **验收**：双 `test.sh` 全绿；`vec_sort` 在 10k 规模可跑完（性能只求可用）。

## S2：builtin 缺口（3 个，解开 std） [ ]（需改 huzc，逐个单提交）

- **目标**：补齐 `std/` 的三把钥匙。
- **B1 `parse_int/parse_float(s: str) -> (bool, i32/f64)`**：解锁 `json/csv/env_cli`。失败返回 `(false, 0)`，沿用 `map_get/read_file_ok` 的哨兵风格，不引入异常。
- **B2 `env_get(k: str) -> (bool, str)`**：解锁 `log/cli` 的环境与配置读取。缺键返回 `(false, "")`。
- **B3 `localtime(ts: i64) -> str`**：最小格式化（`YYYY-MM-DD hh:mm:ss`），解锁 `timex/log`。时区先跟系统本地，不做 UTC 参数。
- **OUT**：不做 UDP/TLS/async；不做 `vec/map` 跨线程共享。
- **验收**：每笔独立提交 + 双 `test.sh` + 对应 `neg`（非法输入不 abort）。

## S3：std 薄封装 [ ]（B1-B3 就绪后，约 1-2 周）

- **目标**：`fsx/env_cli/timex/log/framing/csv/json` 可用最小版。
- **IN**：
  - `std/fsx.hz`：`exists/read_lines/write_lines/path_join`（基于 `read_file_ok/write_file`，纯 Huzi）。
  - `std/csv.hz`：`parse_line/stringify`（纯 Huzi，逗号+引号最小版）。
  - `std/json.hz`：平面 `str->str/i32` 对象 `stringify/parse`（依赖 B1）。**补丁⑦ 预警**：字符串字节语义，`substring` 按字节切分，UTF-8 多字节会被切错；首版声明仅支持 ASCII + `\"`/`\\` 转义，超范围输入返回 `(false, ...)` 不 abort。
  - `std/log.hz + timex.hz + env_cli.hz`：分级日志 + 时间戳 + 子命令解析（依赖 B2/B3）。
  - `std/framing.hz`：行帧收发（依赖 `tcp_*`，同步阻塞语义不变）。
- **OUT**：`http_client` 只留占位（GET 最小版放下一轮）；不做中心仓库/semver。
- **测试**：`huzi-src/test/fsx_test.hz`（读写 `huzi-src/test/out/` 隔离，不污染 `huzc/test/out/`）；`huzc/test/examples/52_std_demo.hz`（读文件→转 JSON→写回端到端）。
- **验收**：双 `test.sh` 全绿；`huzc fmt --check huzi-src` 零 diff 两次幂等。
- **文档同步**：`huzi-src/README` 更新模块状态；`huzc/docs/reference.md` 补新 builtin；`huzc/STATUS.md` 的已完成清单同步打勾，避免再出现“文档地图对不上”。

---

## 全局门禁（每阶段）

1. `cargo build --workspace` 零错误零警告。
2. `bash huzc/test.sh` 全绿 + `bash huzi-src/test.sh` 全绿（两者缺一即红）。
3. `huzc fmt --check huzi-src` 通过；注释统一 `#`；注释归一化范围按 `docs/reference.md` 声明。
4. 中文提交、一事一提交；单文件 500 行 / 单函数 70 行。

## 里程碑

- M0（S0+S1）：`huzi-src` 可用（含解析根+自测链路），写 500 行程序从疼变顺。
- M1（S2）：3 个 builtin 落地，钥匙配齐。
- M2（S3）：`std/` 最小闭环 + 1 个 1000 行级验证项目（TCP KV + 文件持久化）。
