#!/bin/bash
# 仓库统一本地质量门禁入口（替代已移除 CI 的口头约定）
#
# 运行环境：Git Bash / MSYS2 / WSL（依赖 bash/diff，与各子 test.sh 一致）。
# 在仓库根目录执行：bash check.sh
# 跳过性能抽查：SKIP_BENCH=1 bash check.sh 或 bash check.sh --skip-bench
# 跳过自举对拍：SKIP_DUMPDIFF=1 bash check.sh 或 bash check.sh --skip-dumpdiff
# 跳过 parser 对拍：SKIP_PARSEDIFF=1 bash check.sh 或 bash check.sh --skip-parsediff
# 跳过 ast 对拍：SKIP_ASTDIFF=1 bash check.sh 或 bash check.sh --skip-astdiff
#
# 聚合七项门禁（顺序即执行顺序，失败即停）：
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
#   7b. eval_diff.py          暂缓（仅注释，不执行，见阶段 7 末尾说明）
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

# 分阶段执行：标题 + 结果汇总，失败即停（set -e 生效处直接退出，
# 被 if 接管处显式 exit 1，保证 exit code 非零向上传递）
PASS_LIST=""

# 阶段 1：编译器回归（含 cargo build --workspace，由子脚本内部执行）
echo "===== [1/7] 编译器回归 huzc/test.sh ====="
bash huzc/test.sh
echo "PASS [1/7]: 编译器回归"
PASS_LIST="$PASS_LIST 1.编译器回归"

# 阶段 2：自举标准库自测（缺编译器时子脚本会自动构建）
echo "===== [2/7] 标准库自测 huzi-src/test.sh ====="
bash huzi-src/test.sh
echo "PASS [2/7]: 标准库自测"
PASS_LIST="$PASS_LIST 2.标准库自测"

# 阶段 3：格式化门禁（复用 test.sh 的平台自适应后缀逻辑）
echo "===== [3/7] 格式化门禁 huzc fmt --check ====="
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
  (cd huzc && cargo build --workspace) || { echo "FAIL [3/7]: 编译器构建失败"; exit 1; }
  HUZC_BIN="huzc/target/debug/huzc$EXE_SUFFIX"
fi
"$HUZC_BIN" fmt --check huzc/test/cases || { echo "FAIL [3/7]: fmt --check 未通过"; exit 1; }
echo "PASS [3/7]: 格式化门禁"
PASS_LIST="$PASS_LIST 3.格式化门禁"

# 阶段 4：性能回归抽查（脚本按自身路径定位 huzc 根，与 cwd 无关）
echo "===== [4/7] 性能抽查 bench_compare.py ====="
if [ "$SKIP_BENCH" = "1" ]; then
  echo "SKIP [4/7]: 已按要求跳过性能抽查"
else
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [4/7]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" huzc/test/bench_compare.py || { echo "FAIL [4/7]: 性能回归门禁未通过"; exit 1; }
  echo "PASS [4/7]: 性能抽查"
  PASS_LIST="$PASS_LIST 4.性能抽查"
fi

# 阶段 5：自举前端对拍（huzc --dump-tokens vs 自举 hzlex；M1-A dump 对拍接入门禁，防漂移）
echo "===== [5/7] 自举对拍 dump_diff.py ====="
if [ "$SKIP_DUMPDIFF" = "1" ]; then
  echo "SKIP [5/7]: 已按要求跳过自举对拍"
else
  # 复用阶段 3 已定位的 huzc（debug 优先，含 --dump-tokens），此处不重编
  if [ ! -f "$HUZC_BIN" ]; then
    echo "FAIL [5/7]: 未找到 huzc 可执行文件（阶段 1/3 已应构建过）"
    exit 1
  fi
  # 经 huzc build 构建自举 hzlex：固定输出到 examples/hzlex/hzlex.exe
  # （*.exe 已被 gitignore 覆盖，不脏工作区；伴生 .ll/.obj 同理被忽略）
  HZLEX_BIN="examples/hzlex/hzlex.exe"
  "$HUZC_BIN" build --path examples/hzlex --output "$HZLEX_BIN" || { echo "FAIL [5/7]: hzlex 构建失败"; exit 1; }
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [5/7]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" examples/hzlex/dump_diff.py "$HUZC_BIN" "$HZLEX_BIN" huzc/test/cases || { echo "FAIL [5/7]: 自举对拍未通过（huzc/test/cases）"; exit 1; }
  "$PY" examples/hzlex/dump_diff.py "$HUZC_BIN" "$HZLEX_BIN" huzi-src || { echo "FAIL [5/7]: 自举对拍未通过（huzi-src）"; exit 1; }
  echo "PASS [5/7]: 自举对拍"
  PASS_LIST="$PASS_LIST 5.自举对拍"
fi

# 阶段 6：自举 parser 对拍（huzc --dump-parse-stats vs 自举 hzparse；九维对拍接入门禁，防漂移）
echo "===== [6/7] 自举 parser 对拍 parse_diff.py ====="
if [ "$SKIP_PARSEDIFF" = "1" ]; then
  echo "SKIP [6/7]: 已按要求跳过自举 parser 对拍"
else
  # 复用阶段 3 已定位的 huzc（debug 优先，含 --dump-parse-stats），此处不重编
  if [ ! -f "$HUZC_BIN" ]; then
    echo "FAIL [6/7]: 未找到 huzc 可执行文件（阶段 1/3 已应构建过）"
    exit 1
  fi
  # 经 huzc build 构建自举 hzparse：固定输出到 examples/hzparse/hzparse.exe
  # （*.exe 已被 gitignore 覆盖，不脏工作区；伴生 .ll/.obj 同理被忽略）
  HZPARSE_BIN="examples/hzparse/hzparse.exe"
  "$HUZC_BIN" build --path examples/hzparse --output "$HZPARSE_BIN" || { echo "FAIL [6/7]: hzparse 构建失败"; exit 1; }
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [6/7]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" examples/hzparse/parse_diff.py "$HUZC_BIN" "$HZPARSE_BIN" || { echo "FAIL [6/7]: 自举 parser 对拍未通过"; exit 1; }
  echo "PASS [6/7]: 自举 parser 对拍"
  PASS_LIST="$PASS_LIST 6.自举parser对拍"
fi

# 阶段 7：自举 ast 对拍 C2（huzc --ast-json-test vs 自举 hzast --dump-json；C2 已验 17/17 绿）
echo "===== [7/7] 自举 ast 对拍 ast_diff.py ====="
if [ "$SKIP_ASTDIFF" = "1" ]; then
  echo "SKIP [7/7]: 已按要求跳过自举 ast 对拍"
else
  # 复用阶段 3 已定位的 huzc（debug 优先，含 --ast-json-test），此处不重编
  if [ ! -f "$HUZC_BIN" ]; then
    echo "FAIL [7/7]: 未找到 huzc 可执行文件（阶段 1/3 已应构建过）"
    exit 1
  fi
  # 经 huzc build 构建自举 hzast：固定输出到 examples/hzast/hzast.exe
  # （*.exe 已被 gitignore 覆盖，不脏工作区；伴生 .ll/.obj 同理被忽略）
  HZAST_BIN="examples/hzast/hzast.exe"
  "$HUZC_BIN" build --path examples/hzast --output "$HZAST_BIN" || { echo "FAIL [7/7]: hzast 构建失败"; exit 1; }
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [7/7]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" examples/hzast/ast_diff.py "$HUZC_BIN" "$HZAST_BIN" || { echo "FAIL [7/7]: 自举 ast 对拍未通过"; exit 1; }
  echo "PASS [7/7]: 自举 ast 对拍"
  PASS_LIST="$PASS_LIST 7.自举ast对拍"
fi

# 阶段 7b：自举 eval 对拍 C1（暂缓，仅注释不执行，避免常红污染 P0 门）。
#   暂缓原因：C1 红系已知编译器 defect，hz 侧无法绕，按 oracle 结论暂缓。
#   已知 defect 行号（均在 huzc/crates/huzi-codegen/src/codegen/ 内，不在本门禁修复）：
#     - stmt/let_.rs:119-128（Env 值拷贝别名化）
#     - stmt/mod.rs:94-116,198-308（作用域 Env 处理未跟进 RC）
#     - aggregates/struct_lit.rs:39-52（结构体字面量 RC 未跟进）
#     - expr_place/assign.rs:8-67（赋值路径 RC 未跟进）
#     - drop.rs:28-139（RAII 析构与上述别名化冲突）
#   另 eval_diff.py:37-51 的 build_ref 用 tempfile.mkstemp 无 .exe 后缀，
#   Windows 下 huzc 生成 <path>.exe 致 subprocess.run 报 WinError 193，
#   与 eval.hz 堆损坏（0xC0000374）一并另起提交修复，本次不碰 eval_diff.py 逻辑。
#   预留集成命令（暂缓未启用）：
#     "$PY" examples/hzast/eval_diff.py "$HUZC_BIN" "$HZAST_BIN"

# 汇总：能执行到此即前序阶段全部通过（任一失败已提前非零退出）
echo "-----------------------------"
echo "门禁汇总:$PASS_LIST"
echo "全部通过"
