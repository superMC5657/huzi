#!/bin/bash
# 仓库统一本地质量门禁入口（替代已移除 CI 的口头约定）
#
# 运行环境：Git Bash / MSYS2 / WSL（依赖 bash/diff，与各子 test.sh 一致）。
# 在仓库根目录执行：bash check.sh
# 跳过性能抽查：SKIP_BENCH=1 bash check.sh 或 bash check.sh --skip-bench
# 跳过自举对拍：SKIP_DUMPDIFF=1 bash check.sh 或 bash check.sh --skip-dumpdiff
# 跳过 parser 对拍：SKIP_PARSEDIFF=1 bash check.sh 或 bash check.sh --skip-parsediff
# 跳过 ast 对拍：SKIP_ASTDIFF=1 bash check.sh 或 bash check.sh --skip-astdiff
# 跳过 parse-ast 对拍：SKIP_PARSEASTDIFF=1 bash check.sh 或 bash check.sh --skip-parseastdiff
# 跳过 P3 文件模式：SKIP_P3FILEMODE=1 bash check.sh 或 bash check.sh --skip-p3filemode
# 跳过 hzir 对拍：SKIP_HZIR=1 bash check.sh 或 bash check.sh --skip-hzir
# 跳过 eval 对拍：SKIP_EVALDIFF=1 bash check.sh 或 bash check.sh --skip-evaldiff
#
# 聚合十项门禁（顺序即执行顺序，失败即停）：
#   1. huzc/test.sh          编译器回归（含负例；交互用例 10_guess_number_game 跳过，
#                            24_pipe_read 喂管道数据；产物隔离在 huzc/test/out）
#   2. huzi-src/test.sh      自举标准库自测（产物隔离在 huzi-src/test/out）
#   3. huzc fmt --check      格式化门禁（检查 huzc/test/cases）
#   4. bench_compare.py      性能回归抽查（huzi release 相对 Rust -O 不超过 2.0x；
#                            历史比值存档在 huzc/test/bench_baseline.txt，仅漂移提示）
#   5. dump_diff.py          自举前端对拍（huzc --dump-tokens vs 自举 hzlex，
#                            对 huzc/test/cases 与 huzi-src 两个语料逐文件逐字节一致；
#                            方法学见 examples/hzlex/dump_diff.py 头注释；
#                            hzlex 经 `huzc build` 构建，产物固定在
#                            examples/hzlex/hzlex.exe（*.exe 已被 gitignore 覆盖））
#   6. parse_diff.py         自举 parser 对拍（huzc --dump-parse-stats vs 自举 hzparse，
#                            九维统计逐字节一致 + 失败首错行列一致，另 4 内置负例；
#                            语料只认 examples/hzparse/src/corpus.hz，首步做 fd/corpus
#                            覆盖率校验防假绿；hzparse 经 `huzc build` 构建，产物固定在
#                            examples/hzparse/hzparse.exe（*.exe 已被 gitignore 覆盖））
#   7. ast_diff.py            自举 ast 对拍 C2（huzc --ast-json-test vs 自举 hzast --dump-json，
#                            16 向量单行紧凑 JSON 逐字节一致 + 未知 ID 双非零，共 17/17；
#                            口径见 examples/hzast/CONTRACT_DRAFT.md 冻结 1-3
#                            （转义 5 种/键序/换行 split_lines）；
#                            hzast 经 `huzc build` 构建，产物固定在
#                            examples/hzast/hzast.exe（*.exe 已被 gitignore 覆盖））
#   8. parse_ast_diff.py      自举 parse-ast 对拍 P2-b（huzc --dump-ast-json -i vs 自举 hzparse --dump-ast-json -i，
#                            仅 P1 子集 + 超集双非零，语料为 examples/hzparse/src/corpus.hz 动态交集 + 2 内置正/超，
#                            双零单行逐字节一致、双非零 PASS(neg)，128 绿；
#                            口径见 examples/hzparse/RFC_P2B_DRAFT.md §3，与 CONTRACT 冻结 1-3 同源；
#                            hzparse 经 `huzc build` 构建复用 examples/hzparse/hzparse.exe（*.exe 已被 gitignore 覆盖））
#   9. p3file_mode         P3 hzast 文件模式（hzast --dump-json -i <json> 读 P2-b 落盘 JSON 重输出；
#                            ast_diff 17/17 回归 + P1 正例双零 split_lines 单行逐字节 + 超集双非零，
#                            不扩子集；口径见 examples/hzast/src/dump.hz dump_json_file；
#                            hzast 经 `huzc build` 构建复用 examples/hzast/hzast.exe（*.exe 已被 gitignore 覆盖）；
#                            内置正/超临时文件只放 examples/hzast/target/tmp/p3file/（**/target/ 已忽略））
#   10. hzir_diff.py       P4-a hzir 自检与对拍（target/hzir.exe 旧 8 模板 + 新 3 JSON-lower 自检全过 +
#                            hzir_diff 3/3：direct==json==expected + 旧.ll==新.ll；
#                            口径见 examples/hzir/RFC.md §4；
#                            hzir 经 `huzc build` 构建，产物固定在
#                            examples/hzir/target/hzir.exe（**/target/ 已忽略））
#   注：[n/10] 编号保持不动，7b 为例外编号（执行顺序紧随阶段 7，即 7→7b→8，不占 1-10 计数）。
#   7b. eval_diff.py         自举 eval 对拍 C1（huzc 即时编译 eval_ref.hz vs 自举 hzast --dump-eval，
#                            6 行 name=value 逐字节一致，6/6；
#                            口径见 examples/hzast/RFC.md §1；
#                            hzast 经 `huzc build` 构建复用 examples/hzast/hzast.exe（*.exe 已被 gitignore 覆盖））
#
# 注意：huzc/test.sh 自带 RUN_BENCH=1/--bench 开关可顺带跑性能门禁；
# 本入口为避免重复耗时，调用它时不传 --bench，性能抽查统一放在第 4 阶段。
set -eu
cd "$(dirname "$0")" || exit 1

# 解析跳过开关（环境变量与命令行参数二选一）
SKIP_BENCH="${SKIP_BENCH:-0}"
if [ "${1:-}" = "--skip-bench" ]; then
  SKIP_BENCH=1
fi
SKIP_DUMPDIFF="${SKIP_DUMPDIFF:-0}"
if [ "${1:-}" = "--skip-dumpdiff" ] || [ "${2:-}" = "--skip-dumpdiff" ]; then
  SKIP_DUMPDIFF=1
fi
SKIP_PARSEDIFF="${SKIP_PARSEDIFF:-0}"
if [ "${1:-}" = "--skip-parsediff" ] || [ "${2:-}" = "--skip-parsediff" ] || [ "${3:-}" = "--skip-parsediff" ]; then
  SKIP_PARSEDIFF=1
fi
SKIP_ASTDIFF="${SKIP_ASTDIFF:-0}"
if [ "${1:-}" = "--skip-astdiff" ] || [ "${2:-}" = "--skip-astdiff" ] || [ "${3:-}" = "--skip-astdiff" ] || [ "${4:-}" = "--skip-astdiff" ]; then
  SKIP_ASTDIFF=1
fi
SKIP_PARSEASTDIFF="${SKIP_PARSEASTDIFF:-0}"
if [ "${1:-}" = "--skip-parseastdiff" ] || [ "${2:-}" = "--skip-parseastdiff" ] || [ "${3:-}" = "--skip-parseastdiff" ] || [ "${4:-}" = "--skip-parseastdiff" ] || [ "${5:-}" = "--skip-parseastdiff" ]; then
  SKIP_PARSEASTDIFF=1
fi
SKIP_P3FILEMODE="${SKIP_P3FILEMODE:-0}"
if [ "${1:-}" = "--skip-p3filemode" ] || [ "${2:-}" = "--skip-p3filemode" ] || [ "${3:-}" = "--skip-p3filemode" ] || [ "${4:-}" = "--skip-p3filemode" ] || [ "${5:-}" = "--skip-p3filemode" ] || [ "${6:-}" = "--skip-p3filemode" ]; then
  SKIP_P3FILEMODE=1
fi
SKIP_HZIR="${SKIP_HZIR:-0}"
if [ "${1:-}" = "--skip-hzir" ] || [ "${2:-}" = "--skip-hzir" ] || [ "${3:-}" = "--skip-hzir" ] || [ "${4:-}" = "--skip-hzir" ] || [ "${5:-}" = "--skip-hzir" ] || [ "${6:-}" = "--skip-hzir" ] || [ "${7:-}" = "--skip-hzir" ]; then
  SKIP_HZIR=1
fi
SKIP_EVALDIFF="${SKIP_EVALDIFF:-0}"
if [ "${1:-}" = "--skip-evaldiff" ] || [ "${2:-}" = "--skip-evaldiff" ] || [ "${3:-}" = "--skip-evaldiff" ] || [ "${4:-}" = "--skip-evaldiff" ] || [ "${5:-}" = "--skip-evaldiff" ] || [ "${6:-}" = "--skip-evaldiff" ] || [ "${7:-}" = "--skip-evaldiff" ] || [ "${8:-}" = "--skip-evaldiff" ]; then
  SKIP_EVALDIFF=1
fi

# 分阶段执行：标题 + 结果汇总，失败即停（set -e 生效处直接退出，
# 被 if 接管处显式 exit 1，保证 exit code 非零向上传递）
PASS_LIST=""

# 阶段 1：编译器回归（含 cargo build --workspace，由子脚本内部执行）
echo "===== [1/10] 编译器回归 huzc/test.sh ====="
bash huzc/test.sh
echo "PASS [1/10]: 编译器回归"
PASS_LIST="$PASS_LIST 1.编译器回归"

# 阶段 2：自举标准库自测（缺编译器时子脚本会自动构建）
echo "===== [2/10] 标准库自测 huzi-src/test.sh ====="
bash huzi-src/test.sh
echo "PASS [2/10]: 标准库自测"
PASS_LIST="$PASS_LIST 2.标准库自测"

# 阶段 3：格式化门禁（复用 test.sh 的平台自适应后缀逻辑）
echo "===== [3/10] 格式化门禁 huzc fmt --check ====="
EXE_SUFFIX=""
case "$(uname -s 2>/dev/null)" in
  MINGW*|MSYS*|CYGWIN*|Windows*) EXE_SUFFIX=".exe" ;;
esac
HUZC_BIN="huzc/target/debug/huzc$EXE_SUFFIX"
if [ ! -f "$HUZC_BIN" ]; then
  HUZC_BIN="huzc/target/release/huzc$EXE_SUFFIX"
fi
if [ ! -f "$HUZC_BIN" ]; then
  echo "未找到 huzc 可执行文件，先执行 cargo build（阶段 1 已应构建过）..."
  (cd huzc && cargo build --workspace) || { echo "FAIL [3/10]: 编译器构建失败"; exit 1; }
  HUZC_BIN="huzc/target/debug/huzc$EXE_SUFFIX"
fi
"$HUZC_BIN" fmt --check huzc/test/cases || { echo "FAIL [3/10]: fmt --check 未通过"; exit 1; }
echo "PASS [3/10]: 格式化门禁"
PASS_LIST="$PASS_LIST 3.格式化门禁"

# 阶段 4：性能回归抽查（脚本按自身路径定位 huzc 根，与 cwd 无关）
echo "===== [4/10] 性能抽查 bench_compare.py ====="
if [ "$SKIP_BENCH" = "1" ]; then
  echo "SKIP [4/10]: 已按要求跳过性能抽查"
else
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [4/10]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" huzc/test/bench_compare.py || { echo "FAIL [4/10]: 性能回归门禁未通过"; exit 1; }
  echo "PASS [4/10]: 性能抽查"
  PASS_LIST="$PASS_LIST 4.性能抽查"
fi

# 阶段 5：自举前端对拍（huzc --dump-tokens vs 自举 hzlex；M1-A dump 对拍接入门禁，防漂移）
echo "===== [5/10] 自举对拍 dump_diff.py ====="
if [ "$SKIP_DUMPDIFF" = "1" ]; then
  echo "SKIP [5/10]: 已按要求跳过自举对拍"
else
  # 复用阶段 3 已定位的 huzc（debug 优先，含 --dump-tokens），此处不重编
  if [ ! -f "$HUZC_BIN" ]; then
    echo "FAIL [5/10]: 未找到 huzc 可执行文件（阶段 1/3 已应构建过）"
    exit 1
  fi
  # 经 huzc build 构建自举 hzlex：固定输出到 examples/hzlex/hzlex.exe
  # （*.exe 已被 gitignore 覆盖，不脏工作区；伴生 .ll/.obj 同理被忽略）
  HZLEX_BIN="examples/hzlex/hzlex.exe"
  "$HUZC_BIN" build --path examples/hzlex --output "$HZLEX_BIN" || { echo "FAIL [5/10]: hzlex 构建失败"; exit 1; }
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [5/10]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" examples/hzlex/dump_diff.py "$HUZC_BIN" "$HZLEX_BIN" huzc/test/cases || { echo "FAIL [5/10]: 自举对拍未通过（huzc/test/cases）"; exit 1; }
  "$PY" examples/hzlex/dump_diff.py "$HUZC_BIN" "$HZLEX_BIN" huzi-src || { echo "FAIL [5/10]: 自举对拍未通过（huzi-src）"; exit 1; }
  echo "PASS [5/10]: 自举对拍"
  PASS_LIST="$PASS_LIST 5.自举对拍"
fi

# 阶段 6：自举 parser 对拍（huzc --dump-parse-stats vs 自举 hzparse；九维对拍接入门禁，防漂移）
echo "===== [6/10] 自举 parser 对拍 parse_diff.py ====="
if [ "$SKIP_PARSEDIFF" = "1" ]; then
  echo "SKIP [6/10]: 已按要求跳过自举 parser 对拍"
else
  # 复用阶段 3 已定位的 huzc（debug 优先，含 --dump-parse-stats），此处不重编
  if [ ! -f "$HUZC_BIN" ]; then
    echo "FAIL [6/10]: 未找到 huzc 可执行文件（阶段 1/3 已应构建过）"
    exit 1
  fi
  # 经 huzc build 构建自举 hzparse：固定输出到 examples/hzparse/hzparse.exe
  # （*.exe 已被 gitignore 覆盖，不脏工作区；伴生 .ll/.obj 同理被忽略）
  HZPARSE_BIN="examples/hzparse/hzparse.exe"
  "$HUZC_BIN" build --path examples/hzparse --output "$HZPARSE_BIN" || { echo "FAIL [6/10]: hzparse 构建失败"; exit 1; }
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [6/10]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" examples/hzparse/parse_diff.py "$HUZC_BIN" "$HZPARSE_BIN" || { echo "FAIL [6/10]: 自举 parser 对拍未通过"; exit 1; }
  echo "PASS [6/10]: 自举 parser 对拍"
  PASS_LIST="$PASS_LIST 6.自举parser对拍"
fi

# 阶段 7：自举 ast 对拍 C2（huzc --ast-json-test vs 自举 hzast --dump-json；C2 已验 17/17 绿）
echo "===== [7/10] 自举 ast 对拍 ast_diff.py ====="
if [ "$SKIP_ASTDIFF" = "1" ]; then
  echo "SKIP [7/10]: 已按要求跳过自举 ast 对拍"
else
  # 复用阶段 3 已定位的 huzc（debug 优先，含 --ast-json-test），此处不重编
  if [ ! -f "$HUZC_BIN" ]; then
    echo "FAIL [7/10]: 未找到 huzc 可执行文件（阶段 1/3 已应构建过）"
    exit 1
  fi
  # 经 huzc build 构建自举 hzast：固定输出到 examples/hzast/hzast.exe
  # （*.exe 已被 gitignore 覆盖，不脏工作区；伴生 .ll/.obj 同理被忽略）
  HZAST_BIN="examples/hzast/hzast.exe"
  "$HUZC_BIN" build --path examples/hzast --output "$HZAST_BIN" || { echo "FAIL [7/10]: hzast 构建失败"; exit 1; }
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [7/10]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" examples/hzast/ast_diff.py "$HUZC_BIN" "$HZAST_BIN" || { echo "FAIL [7/10]: 自举 ast 对拍未通过"; exit 1; }
  echo "PASS [7/10]: 自举 ast 对拍"
  PASS_LIST="$PASS_LIST 7.自举ast对拍"
fi

# 阶段 7b：自举 eval 对拍 C1（huzc 即时编译 eval_ref.hz vs 自举 hzast --dump-eval；6/6）
echo "===== [7b/10] 自举 eval 对拍 eval_diff.py ====="
if [ "$SKIP_EVALDIFF" = "1" ]; then
  echo "SKIP [7b/10]: 已按要求跳过自举 eval 对拍"
else
  # 复用阶段 3 已定位的 huzc（debug 优先），此处不重编编译器
  if [ ! -f "$HUZC_BIN" ]; then
    echo "FAIL [7b/10]: 未找到 huzc 可执行文件（阶段 1/3 已应构建过）"
    exit 1
  fi
  # 经 huzc build 构建自举 hzast：复用阶段 7 产物路径 examples/hzast/hzast.exe
  # （*.exe 已被 gitignore 覆盖，不脏工作区；伴生 .ll/.obj 同理被忽略）
  HZAST_BIN="examples/hzast/hzast.exe"
  "$HUZC_BIN" build --path examples/hzast --output "$HZAST_BIN" || { echo "FAIL [7b/10]: hzast 构建失败"; exit 1; }
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [7b/10]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" examples/hzast/eval_diff.py "$HUZC_BIN" "$HZAST_BIN" || { echo "FAIL [7b/10]: 自举 eval 对拍未通过"; exit 1; }
  echo "PASS [7b/10]: 自举 eval 对拍"
  PASS_LIST="$PASS_LIST 7b.自举eval对拍"
fi

# 阶段 8：自举 parse-ast 对拍 P2-b（huzc --dump-ast-json -i vs 自举 hzparse --dump-ast-json -i；仅 P1 子集 + 超集双非零）
echo "===== [8/10] 自举 parse-ast 对拍 parse_ast_diff.py ====="
if [ "$SKIP_PARSEASTDIFF" = "1" ]; then
  echo "SKIP [8/10]: 已按要求跳过自举 parse-ast 对拍"
else
  # 复用阶段 3 已定位的 huzc（debug 优先，含 --dump-ast-json），此处不重编
  if [ ! -f "$HUZC_BIN" ]; then
    echo "FAIL [8/10]: 未找到 huzc 可执行文件（阶段 1/3 已应构建过）"
    exit 1
  fi
  # 经 huzc build 构建自举 hzparse：固定输出到 examples/hzparse/hzparse.exe
  # （*.exe 已被 gitignore 覆盖，不脏工作区；伴生 .ll/.obj 同理被忽略）
  HZPARSE_BIN="examples/hzparse/hzparse.exe"
  "$HUZC_BIN" build --path examples/hzparse --output "$HZPARSE_BIN" || { echo "FAIL [8/10]: hzparse 构建失败"; exit 1; }
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [8/10]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" examples/hzparse/parse_ast_diff.py "$HUZC_BIN" "$HZPARSE_BIN" || { echo "FAIL [8/10]: 自举 parse-ast 对拍未通过"; exit 1; }
  echo "PASS [8/10]: 自举 parse-ast 对拍"
  PASS_LIST="$PASS_LIST 8.自举parse-ast对拍"
fi

# 阶段 9：P3 hzast 文件模式（hzast --dump-json -i <json> 读 P2-b 落盘 JSON 重输出；
#          ast_diff 17/17 回归 + P1 正例双零 split_lines 单行逐字节 + 超集双非零，不扩子集）
echo "===== [9/10] P3 文件模式 hzast --dump-json -i ====="
if [ "$SKIP_P3FILEMODE" = "1" ]; then
  echo "SKIP [9/10]: 已按要求跳过 P3 文件模式"
else
  # 复用阶段 3 已定位的 huzc（debug 优先），此处不重编编译器
  if [ ! -f "$HUZC_BIN" ]; then
    echo "FAIL [9/10]: 未找到 huzc 可执行文件（阶段 1/3 已应构建过）"
    exit 1
  fi
  # 经 huzc build 构建自举 hzast：复用阶段 7 产物路径 examples/hzast/hzast.exe
  # （*.exe 已被 gitignore 覆盖，不脏工作区；伴生 .ll/.obj 同理被忽略）
  HZAST_BIN="examples/hzast/hzast.exe"
  "$HUZC_BIN" build --path examples/hzast --output "$HZAST_BIN" || { echo "FAIL [9/10]: hzast 构建失败"; exit 1; }
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [9/10]: 未检测到有效的 Python 3 环境"; exit 1; }
  # C2 回归：16 向量 + 未知 ID 双非零，共 17/17（P3 未扩变体，不断言 eval）
  "$PY" examples/hzast/ast_diff.py "$HUZC_BIN" "$HZAST_BIN" || { echo "FAIL [9/10]: 自举 ast 对拍未通过（17/17 回归）"; exit 1; }
  # P3 文件模式两例（内置正/超各一；临时文件只放仓库树内
  # examples/hzast/target/tmp/p3file/，已被 **/target/ 忽略，不用系统 Temp）：
  #   正例 P1 最小 fact+main：huzc 落盘 JSON 与 hzast 重输出须双零且经 split_lines
  #   后 len==1 逐字节一致（CRLF 归一，口径同 ast_diff.py split_lines）；
  #   超例 struct S（源）+ match-kind JSON（超子集变体）：两侧须双非零。
  HUZC_BIN="$HUZC_BIN" HZAST_BIN="$HZAST_BIN" "$PY" - <<'P3EOF' || { echo "FAIL [9/10]: P3 文件模式未通过"; exit 1; }
import os
import subprocess
import sys


def split_lines(raw):
    parts = raw.split(b'\n')
    if parts and parts[-1] == b'':
        parts.pop()
    if parts and all(p.endswith(b'\r') for p in parts):
        parts = [p[:-1] for p in parts]
    return parts


POS_SRC = "fn fact(n: i32) -> i32 { if n <= 1 { return 1 } return n * fact(n - 1) } fn main() -> i32 { let x = fact(5) print(x) return x }"
NEG_SRC = "struct S { x: i32 }"
NEG_JSON = '{"fns":[],"main":[{"kind":"match","expr":{"kind":"num","value":1}}]}'

huzc = os.environ["HUZC_BIN"]
hzast = os.environ["HZAST_BIN"]
tmpdir = os.path.join("examples", "hzast", "target", "tmp", "p3file")
os.makedirs(tmpdir, exist_ok=True)
pos_hz = os.path.join(tmpdir, "p3_pos.hz")
with open(pos_hz, "w", encoding="utf-8", newline="\n") as f:
    f.write(POS_SRC)
pa = subprocess.run([huzc, "--dump-ast-json", "-i", pos_hz], capture_output=True)
if pa.returncode != 0:
    print("FAIL(p3-pos-rust-status): rc=%d" % pa.returncode)
    sys.exit(1)
pos_json = os.path.join(tmpdir, "p3_pos.json")
with open(pos_json, "wb") as f:
    f.write(pa.stdout or b"")
pb = subprocess.run([hzast, "--dump-json", "-i", pos_json], capture_output=True)
if pb.returncode != 0:
    print("FAIL(p3-pos-hz-status): rc=%d" % pb.returncode)
    sys.exit(1)
la, lb = split_lines(pa.stdout or b""), split_lines(pb.stdout or b"")
if len(la) == 1 and len(lb) == 1 and la[0] == lb[0]:
    print("PASS(p3-pos): double-zero single-line byte-equal")
else:
    print("FAIL(p3-pos-diff): rust=%d hz=%d" % (len(la), len(lb)))
    sys.exit(1)
neg_hz = os.path.join(tmpdir, "p3_neg.hz")
with open(neg_hz, "w", encoding="utf-8", newline="\n") as f:
    f.write(NEG_SRC)
neg_json = os.path.join(tmpdir, "p3_neg.json")
with open(neg_json, "w", encoding="utf-8", newline="\n") as f:
    f.write(NEG_JSON)
na = subprocess.run([huzc, "--dump-ast-json", "-i", neg_hz], capture_output=True)
nb = subprocess.run([hzast, "--dump-json", "-i", neg_json], capture_output=True)
if na.returncode != 0 and nb.returncode != 0:
    print("PASS(p3-neg): superset double-nonzero rust_rc=%d hz_rc=%d" % (na.returncode, nb.returncode))
else:
    print("FAIL(p3-neg-status): rust_rc=%d hz_rc=%d" % (na.returncode, nb.returncode))
    sys.exit(1)
P3EOF
  echo "PASS [9/10]: P3 文件模式"
  PASS_LIST="$PASS_LIST 9.P3文件模式"
fi

# 阶段 10：P4-a hzir 自检与对拍（target/hzir.exe 旧 8 模板 + 新 3 JSON-lower 自检全过 +
#           hzir_diff.py 3/3：direct==json==expected + 旧.ll==新.ll）
echo "===== [10/10] P4-a hzir 自检与对拍 hzir_diff.py ====="
if [ "$SKIP_HZIR" = "1" ]; then
  echo "SKIP [10/10]: 已按要求跳过 P4-a hzir 对拍"
else
  # 复用阶段 3 已定位的 huzc（debug 优先），此处不重编编译器
  if [ ! -f "$HUZC_BIN" ]; then
    echo "FAIL [10/10]: 未找到 huzc 可执行文件（阶段 1/3 已应构建过）"
    exit 1
  fi
  # 经 huzc build 构建自举 hzir：固定输出到 examples/hzir/target/hzir.exe
  # （**/target/ 已被 gitignore 覆盖，不脏工作区；伴生 .ll/.obj 同理被忽略）
  HZIR_BIN="examples/hzir/target/hzir.exe"
  "$HUZC_BIN" build --path examples/hzir --output "$HZIR_BIN" || { echo "FAIL [10/10]: hzir 构建失败"; exit 1; }
  # hzir 自检（8 模板 + 3 JSON-lower 全在 Huzi 内 write+clang+run，exit 0 + 双 pass 行）
  "$HZIR_BIN" || { echo "FAIL [10/10]: hzir 自检未通过（旧 8 + 新 3）"; exit 1; }
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [10/10]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" examples/hzir/hzir_diff.py "$HUZC_BIN" "$HZIR_BIN" || { echo "FAIL [10/10]: P4-a 对拍未通过"; exit 1; }
  echo "PASS [10/10]: P4-a hzir 对拍"
  PASS_LIST="$PASS_LIST 10.P4a对拍"
fi

# 汇总：能执行到此即前序阶段全部通过（任一失败已提前非零退出）
echo "-----------------------------"
echo "门禁汇总:$PASS_LIST"
echo "全部通过"
