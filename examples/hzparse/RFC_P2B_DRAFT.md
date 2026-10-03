# RFC: hzparse P2-b AST 落盘原型（草稿，供 orchestrator 验证，不入门禁）

> 范围：仅为 `hzparse --dump-ast-json -i <file>` 最小原型 + `parse_ast_diff.py` 草稿定口径。
> 不改 `check.sh`（fix-4 正占 `check.sh` 做 C2 门，避免写冲突），不碰 `examples/hzast`、`codegen`。
> 口径唯一来源仍为 `examples/hzast/RFC.md §2` 与 `examples/hzast/CONTRACT_DRAFT.md` 冻结 1-3；
> 本文件为 P2-b 增补一页，落地时合入 `examples/hzparse/RFC.md §6` 或保留为独立草稿均可。

## 1. `--dump-ast-json -i` 子集定义（先只落 P1 子集）

- P1 子集 = Expr7 / Stmt7 / Type0（`hzast RFC §2.1`）：
  - Expr7：`num/bool/str/var/bin/un/call`（`bin.op ∈ + - * / % == != < <= > >= && ||`，`un.op ∈ - ! *`）。
  - Stmt7：`let/assign/if/while/return/print/expr`（`assign` 仅 `x = v` 单赋值折叠，
    `print` 仅单参 `print(e)` 折叠，其余为 `expr`，与 `ast_json.rs:122-180` 同形）。
  - Type0：类型标注忽略（`let x: T = v`、`fn f(x: T) -> R` 照常落盘，不拒绝）；
    泛型形参忽略，泛型调用 `f<T>(...)` 拒绝。
- JSON 形状与转义/键序/换行三冻结完全复用 C2（`CONTRACT_DRAFT.md` 冻结 1-3）：
  - 紧凑、无空白、键序固定、末尾单 `\n`（`print` 自带换行；对拍按行切分后比较）。
  - 转义仅 5 种（`"` `\` `\n` `\r` `\t`，与 `ast_json.rs:10-23`、`json.hz:6-28` 同串）。
  - `prog := {"fns":[fn,...],"main":[stmt,...]}`（`fns` 为除 `main` 外顶层 `fn`，
    `main` 为 `main` 体或顶层非 `fn` 语句，与 `ast_json.rs:247-283 program_to_json` 同形）。
- huzc 口径（已落地）：`--dump-ast-json -i <file>` 经 `lex_parse_with_depth` +
  `program_to_json`，子集外节点返回 `Err` → `eprintln + exit 1`，不做截断输出
  （见 `huzc/crates/huzc/src/dump/stats_json.rs:42-61`、`ast_json.rs:59-69,105,145`）。

## 2. 超子集双非零约定（冻结）

- 子集外变体（`struct/enum/trait/impl/import/export/for/break/continue/defer/Block` 裸块、
  `float/char/box/null/vec/if-expr/match/closure/[/./?/::/</T>` 等、元组模式 `let (a,b)`、
  无初值 `let x;`、裸 `return;`、复赋值 `+=`、非标识赋值 `a.b = / a[i] = / *b =`、
  `elif`、`| ^ & << >>` 位运算等）两侧约定非零退出，不对拍内容：
  - Rust 侧 `Err → eprintln + exit 1`；Huzi 侧 `panic("hzparse: ast-json outside P1 subset...")` 非零退出。
- 语法层非零亦归入双非零（`lex_parse` 期 `exit 1`，与 `program_to_json Err exit 1` 同视为非零）：
  - `;` 空语句两侧同拒（Rust `parse-error Unexpected token: Semi`，hz `p1_block/p1_program` 直接失败；
    `p1` 不跳过 `;`，与 `parse_block` 同口径）。
  - `var {` 仅结构体字面量形（`{ ident :`）判超集，其余视为 `var` + 块首（`while i < n {` 双零；
    见 `p1expr.hz p1_suffix` 前瞻，与 `atom.hz pz_is_struct_lit` 同判据）。
- `hzparse` 现状仅九维计数、无落盘（`RFC.md §2` 冻结）；P2-b 原型新增
  `--dump-ast-json -i <file>` 最小实现（只 P1 子集，超集非零退出），不改九维口径。

## 3. `parse_ast_diff.py` 口径（草稿，不接入 `check.sh`）

- 语料 = `src/corpus.hz` 中 P1 子集交集（动态判定，无需另建清单）：
  - 对 `corpus.hz` 逐文件跑 `huzc --dump-ast-json -i <file>` vs
    `hzparse --dump-ast-json -i <file>`。
  - 双零退出 → 按 `split_lines` 切分后须 `len==1` 且逐字节 `==` 即 PASS；
    任一长度≠1 或内容 diff 即 FAIL（复用 `examples/hzlex/dump_diff.py:10-20` 判据，
    二进制安全，不 `strip`；警告：不要换成 `diff --strip-trailing-cr` 或 `sed`）。
  - 双非零退出 → PASS(neg)（超子集，不比较内容）；一零一非零 → FAIL(status)。
  - 换行：hz 侧 `print` 落 `\r\n`、Rust 侧 `println!` 落 `\n`，一律经 `split_lines` 归一后再比
    （单行 `len==1` 判据不变；裸 `==` 会因 `\r` 误红，见本地验证）。
- 另加 2 内置用例（正/超各一，与本地验证同文件）：
  - 正例 P1 最小 `fact + main`（`if/return/call/let/print` 全覆盖，见第 4 节）须双零且逐字节一致。
  - 超例 `struct S`（或 `elif/for` 任一）须双非零。
- 退出码：用法/路径错误返回 2；全部 PASS 返回 0；有 FAIL 返回 1。
- 覆盖率：复用 `parse_diff.py` 的 `fd == corpus` 校验思路，但本草稿暂不做强制覆盖率门，
  只打印 `passed/failed`（避免 P2-b 未定 P1 清单前误红；正式入门禁时再收紧）。

## 4. 本地验证（2026-10-03 实测，仓库根执行，不碰 check.sh）

- 构建：`./huzc/target/debug/huzc.exe build --path examples/hzparse --output "examples/hzparse/hzparse.exe"`
  → `[ok] examples/hzparse/hzparse.exe generated successfully!`，零错误零警告（`EXIT:0`）。
- 正例单文件（`examples/hzparse/target/tmp/p1_pos.hz`，仓库内临时）：
  `fn fact(n: i32) -> i32 { if n <= 1 { return 1 } return n * fact(n - 1) } fn main() -> i32 { let x = fact(5) print(x) return x }`
  → 两侧 `--dump-ast-json -i` 双零；`split_lines` 后单行逐字节一致（裸 `==` 差一 `\r`，经判据归一即等）。
- 超例单文件（`examples/hzparse/target/tmp/p1_neg.hz`）：`struct S { x: i32 }`
  → 两侧同为非零（Rust `ast-json: stmt outside M1-C subset exit 1`；hz `panic hzparse: ast-json outside P1 subset exit 1`）。
- 全量对拍：`python examples/hzparse/parse_ast_diff.py ./huzc/target/debug/huzc.exe ./examples/hzparse/hzparse.exe`
  → `128 passed, 0 failed (total 128)`（`corpus.hz` 126 + 内置正/超 2；`PASS(neg)` 含超集，`FAIL(status)` 0）。
- 格式：`./huzc/target/debug/huzc.exe fmt --check examples/hzparse` → `All 13 file(s) are properly formatted.`。
- 复核：`git status --short` 仅 `M examples/hzparse/src/main.hz` + `?? examples/hzparse/{RFC_P2B_DRAFT.md,parse_ast_diff.py,src/parser/p1*.hz}`；
  `git diff --stat` 仅 `main.hz 33+/1-`（`--dump-ast-json -i` 入口）；无 `check.sh`/`hzast`/`codegen` 改动。
  构建产物与临时文件仅 `examples/hzparse/target/tmp/`（`**/target/` 已忽略）与 `examples/hzparse/*.exe`（`*.exe` 已忽略）。

## 5. 给 `check.sh` 集成的命令草稿（本文件只记录，自己不执行）

```bash
# 前提：复用 check.sh[3/7] 已定位的 HUZC_BIN（含 EXE_SUFFIX 平台自适应）与 PY（python3/python/py 三选一）。
# 构建（固定产物路径，对标 [6/7] hzparse.exe；*.exe 已被 gitignore 覆盖）：
"$HUZC_BIN" build --path examples/hzparse --output "examples/hzparse/hzparse.exe"
HZPARSE_BIN="examples/hzparse/hzparse.exe"

# P2-b 对拍（草稿，暂不入门禁；语料为 corpus.hz 动态交集 + 2 内置正/超）：
"$PY" examples/hzparse/parse_ast_diff.py "$HUZC_BIN" "$HZPARSE_BIN"
```

- 注意：本草稿不接入 `check.sh`（P2-b 明确不入门禁）；正式集成时再补
  `[8/7]` 阶段与 `SKIP_ASTPARSEDIFF` 开关，与 fix-4 的 C2 门错开。
