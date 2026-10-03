# hzast Canonical JSON 冻结契约（草稿，供 P1 check.sh[7/7] 集成前冻结）

> 范围：仅冻结“转义 / 键序 / 换行”三项逐字节口径，供 `ast_diff.py`（C2）与
> `huzc --ast-json-test` / `hzast --dump-json` 对拍使用。
> 不动变体子集（Expr7 / Stmt7 / Type0，见 `RFC.md §2.1`），不做 Type / Span / 文件模式。
> 口径唯一来源仍为 `RFC.md`；本文件为三冻结的提炼草稿，落地时合入 `RFC.md §2.2-2.3`
> 或作为行内注释（`json.hz` 头 + `ast_json.rs` 头）均可，不新增语料与变体。

## 冻结 1：转义（5 种，对标 `huzc/crates/huzc/src/ast_json.rs:10-23`）

- 仅转义 5 种（两侧逐字节同串）：
  `"` → `\"`、`\\` → `\\`、` \n ` → `\n`、` \r ` → `\r`、` \t ` → `\t`。
- 其余控制字符（`\b \f \u00XX`）与裸控制字节不在本期语料内；若出现，两侧行为未定义，不对拍。
- 非 ASCII 按 UTF-8 字节透传（Huzi 侧 `substring(s,i,i+1)` 逐字节拼接；
  Rust 侧 `for c in input.chars()` 逐字符匹配同 5 种，其余 `push(c)` 原样透传，不做 `\u` 转义）。
  与 `serde_json` 默认透传一致，刻意不用 `serde_json` 以免其 `\b \f` 多转义导致幽灵 diff。
- `op / name` 字段同样经此转义（本期 `op` 仅 `+ - * / % == != < <= > >= !`，
  `name` 仅 `[A-Za-z0-9_]`，均无转义触发；`expr_str_esc` 向量覆盖 5 种全触发：
  `a"b\c<d>` 形如 `a\"b\\c\nd\te\rf`，见 `vectors.hz:18` 与 `ast_json.rs:341-343`）。

## 冻结 2：键序（紧凑、无空白，键序固定）

- 紧凑、无空白、末尾单 `\n`（`print` 自带换行；对拍按行切分后比较，见冻结 3）。
- 形状（与 `RFC.md §2.2`、`json.hz:76-165`、`ast_json.rs:89-147` 完全同序）：

```
expr := {"kind":"num","value":INT}
      | {"kind":"bool","value":true|false}
      | {"kind":"str","value":STRING}
      | {"kind":"var","name":STRING}
      | {"kind":"bin","op":STRING,"left":expr,"right":expr}
      | {"kind":"un","op":STRING,"operand":expr}
      | {"kind":"call","name":STRING,"args":[expr,...]}
stmt := {"kind":"let","name":STRING,"expr":expr}
      | {"kind":"assign","name":STRING,"expr":expr}
      | {"kind":"if","cond":expr,"then":[stmt,...],"else":[stmt,...]}
      | {"kind":"while","cond":expr,"body":[stmt,...]}
      | {"kind":"return","expr":expr}
      | {"kind":"print","expr":expr}
      | {"kind":"expr","expr":expr}
fn   := {"name":STRING,"params":[STRING,...],"body":[stmt,...]}
prog := {"fns":[fn,...],"main":[stmt,...]}
```

- `num` 为十进制 `i32`（`to_string` 口径，不补零）；`bool` 小写；空数组即 `[]`。
- `if` 的 `else` 空即 `[]`（Huzi `else_body` 空 vec 对应；Rust 无 `else_branch` 即 `[]`，
  见 `ast_json.rs:203-206`）；`call` 空参即 `[]`；`prog_fact` 为唯一程序级向量
 （含 `params/body` 与 `fns/main` 键序，见 `ast_json.rs:448-463` 与 `vectors.hz:76-103`）。
- Box 透明（`bin.left/right`、`un.operand` 直接内联子 JSON，不落盘）；
  Span 丢弃（Rust `Spanned<Stmt>` 只取 `.node`，JSON 不含行列）；
  `null` 右子不落盘，统一为 `operand` 单字段（见 `RFC.md §2.4`）。
- 子集外变体（`match/closure/struct/enum` 等、含 Type 标注、`elif`、泛型调用等）
  两侧约定非零退出，不对拍（Rust 返回 `Err`，Huzi 返回 `""`，见 `ast_json.rs:59-69,105,145`）。

## 冻结 3：换行（`split_lines` 复用 `examples/hzlex/dump_diff.py:10-20`）

- 两侧统一按 `b'\n'` 切分（尾空行 `pop` 保留），二进制安全，不 `strip` 任何字节。
- 判据：切后除末空外每一行都以 `b'\r'` 结尾即判 Windows-CRLF 模式，每行去一个尾部 `b'\r`
 （内容 `\r` + `\r\n` 行尾则留一个 `\r`，内容无损）；否则判 LF/混合模式，原样保留（含内容 `\r`）。
- 此判据替代旧的 `b'\r\n' in stdout` 检测（后者在 dump 内容含 `\r` 时误触发，致幽灵 FAIL；
  警告：不要换成 `diff --strip-trailing-cr` 或 `sed 's/\r$//'`，两者曾把全绿误报成 40 FAIL）。
- C1（`eval_diff.py:28-34`）：`--dump-eval` 6 行 `name=value`
 （`arithmetic/variables/conditional/while/fact/str`，顺序固定，见 `eval_diff.py:79`），
  与 `eval_ref.hz` 原生 6 行逐行 `==` 且前缀匹配，6/6 即 PASS；任一侧非零退出即 FAIL。
- C2（`ast_diff.py:44-50`）：16 向量各单行紧凑 JSON（`len==1`），逐字节 `==` 即 PASS；
  16/16 + 未知 ID 双非零（`ast_diff.py:90-97`：`no_such_id` 两侧均非零退出，Rust `eprintln+exit1`，
  Huzi `panic`）共 17/17 即 PASS；任一向量长度≠1 或内容 diff 即 FAIL。
- `eval.hz` 仅 `i32/bool/str`（`val_to_int/val_to_bool/val_to_string` 口径），不涉及换行转义；
  `str` 含 `\n/\r/\t` 时经冻结 1 转义后落盘，换行判据只切 `print` 行尾 `\n`，内容 `\r` 无损。

## 刻意不做（非缺口，是冻结边界，与 `RFC.md §3` 同口径）

- 无 Type JSON（Type 0/16，本期不做）、无 Span、无 `match/closure/struct/enum` 全量变体。
- 无错误恢复：未知 ID 两侧均非零退出，不比较内容。
- 无 `serde_json` 依赖：两侧手写转义，保持零新增依赖。
- 不碰 `codegen/llc/lsp/huzi-src/log`；文件模式（`--dump-ast-json -i <file>` 整程序
  `{"fns":...,"main":...}`）本期仅供人工排查，不进对拍（Huzi 侧无解析器，无法对等）。

## 给 check.sh[7/7] 集成的精确命令（本文件只记录，不改 `check.sh`）

```bash
# 前提：复用 check.sh[3/5] 已定位的 HUZC_BIN（含 EXE_SUFFIX 平台自适应）与 PY（python3/python/py 三选一）。
# 构建（固定产物路径，对标 [5/5] hzlex.exe；*.exe 已被 gitignore 覆盖）：
"$HUZC_BIN" build --path examples/hzast --output "examples/hzast/hzast.exe"
HZAST_BIN="examples/hzast/hzast.exe"

# C1 对拍（6/6 PASS 即返回 0，任一 FAIL 返回 1；eval_ref 由脚本即时编译，Windows 需 .exe 后缀适配见备注）：
"$PY" examples/hzast/eval_diff.py "$HUZC_BIN" "$HZAST_BIN"

# C2 对拍（16 向量 + 1 未知 ID 双非零，共 17/17 PASS 即返回 0）：
"$PY" examples/hzast/ast_diff.py "$HUZC_BIN" "$HZAST_BIN"
```

- 本地验证实测（Windows pwsh，仓库根）：
  `./huzc/target/debug/huzc.exe build --path examples/hzast --output C:/Users/supermc/AppData/Local/Temp/opencode/hzast_p1.exe`
  `python examples/hzast/ast_diff.py ./huzc/target/debug/huzc.exe C:/.../hzast_p1.exe` → 17 passed（16+1neg）。
  `python examples/hzast/eval_diff.py ...` 暂 FAIL（见下备注，需编译器侧修复后复验）。
- 备注（Windows 适配，留给 check.sh 集成时处理，不在本草稿改动）：
  `eval_diff.py:37-51` 的 `build_ref` 用 `tempfile.mkstemp` 无 `.exe` 后缀，
  `huzc --input <src> --output <path>` 在 Windows 生成 `<path>.exe`，
  脚本随后 `subprocess.run([ref_bin])` 会 `WinError 193`；Git Bash 下 `check.sh` 需显式加
  `EXE_SUFFIX=".exe"` 适配（同 [3/5] 的 `uname -s` 分支），或脚本侧 `path + ".exe"` 回退。
  另 `hzast --dump-eval` 当前堆损坏（`0xC0000374`，见验证报告），需先修编译器结构体移动语义。
