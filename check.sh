#!/bin/bash
# 仓库统一本地质量门禁入口（替代已移除 CI 的口头约定）
#
# 运行环境：Git Bash / MSYS2 / WSL（依赖 bash/diff，与各子 test.sh 一致）。
# 在仓库根目录执行：bash check.sh
# 跳过性能抽查：SKIP_BENCH=1 bash check.sh 或 bash check.sh --skip-bench
#
# 聚合四项门禁（顺序即执行顺序，失败即停）：
#   1. huzc/test.sh          编译器回归（含负例；交互用例 10_guess_number_game 跳过，
#                            24_pipe_read 喂管道数据；产物隔离在 huzc/test/out）
#   2. huzi-src/test.sh      自举标准库自测（产物隔离在 huzi-src/test/out）
#   3. huzc fmt --check      格式化门禁（检查 huzc/test/cases）
#   4. bench_compare.py      性能回归抽查（huzi release 相对 Rust -O 不超过 2.0x）
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

# 分阶段执行：标题 + 结果汇总，失败即停（set -e 生效处直接退出，
# 被 if 接管处显式 exit 1，保证 exit code 非零向上传递）
PASS_LIST=""

# 阶段 1：编译器回归（含 cargo build --workspace，由子脚本内部执行）
echo "===== [1/4] 编译器回归 huzc/test.sh ====="
bash huzc/test.sh
echo "PASS [1/4]: 编译器回归"
PASS_LIST="$PASS_LIST 1.编译器回归"

# 阶段 2：自举标准库自测（缺编译器时子脚本会自动构建）
echo "===== [2/4] 标准库自测 huzi-src/test.sh ====="
bash huzi-src/test.sh
echo "PASS [2/4]: 标准库自测"
PASS_LIST="$PASS_LIST 2.标准库自测"

# 阶段 3：格式化门禁（复用 test.sh 的平台自适应后缀逻辑）
echo "===== [3/4] 格式化门禁 huzc fmt --check ====="
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
  (cd huzc && cargo build --workspace) || { echo "FAIL [3/4]: 编译器构建失败"; exit 1; }
  HUZC_BIN="huzc/target/debug/huzc$EXE_SUFFIX"
fi
"$HUZC_BIN" fmt --check huzc/test/cases || { echo "FAIL [3/4]: fmt --check 未通过"; exit 1; }
echo "PASS [3/4]: 格式化门禁"
PASS_LIST="$PASS_LIST 3.格式化门禁"

# 阶段 4：性能回归抽查（脚本按自身路径定位 huzc 根，与 cwd 无关）
echo "===== [4/4] 性能抽查 bench_compare.py ====="
if [ "$SKIP_BENCH" = "1" ]; then
  echo "SKIP [4/4]: 已按要求跳过性能抽查"
else
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL [4/4]: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" huzc/test/bench_compare.py || { echo "FAIL [4/4]: 性能回归门禁未通过"; exit 1; }
  echo "PASS [4/4]: 性能抽查"
  PASS_LIST="$PASS_LIST 4.性能抽查"
fi

# 汇总：能执行到此即前序阶段全部通过（任一失败已提前非零退出）
echo "-----------------------------"
echo "门禁汇总:$PASS_LIST"
echo "全部通过"
