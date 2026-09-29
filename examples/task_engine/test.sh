#!/bin/bash
# task_engine 自举链自测：huzc -i → -o → 运行 → diff --strip-trailing-cr
# 三段式仿 huzi-src/test.sh（编译→运行→对拍）；src/main.hz 内嵌 assert
# 保持不动，程序非零退出即 FAIL。
#
# 待跑标注：当前工具链在 [5/5] llc 阶段报预存债
#   "mul constexprs are no longer supported"（见 examples/hzast/RFC.md §3），
# 暂无实跑输出；test/expected/task_engine.stdout 现以 README
# “运行输出效果预览”逐字节转录为准（含示例时间戳），待工具链修复后重跑
# 对拍并刷新期望文件。时间戳与总耗时每次运行都变，对拍前用 norm 归一化。
set -u
cd "$(dirname "$0")" || exit 1

mkdir -p test/out

EXE_SUFFIX=""
case "$(uname -s 2>/dev/null)" in
  MINGW*|MSYS*|CYGWIN*|Windows*) EXE_SUFFIX=".exe" ;;
esac

DIFF_CMD="diff"
if diff --strip-trailing-cr /dev/null /dev/null > /dev/null 2>&1; then
  DIFF_CMD="diff --strip-trailing-cr"
fi

run_limited() {
  if command -v timeout > /dev/null 2>&1; then timeout 60 "$@"; else "$@"; fi
}

HUZC="../../huzc/target/debug/huzc$EXE_SUFFIX"
if [ ! -f "$HUZC" ]; then
  HUZC="../../huzc/target/release/huzc$EXE_SUFFIX"
fi
if [ ! -f "$HUZC" ]; then
  (cd ../../huzc && cargo build --workspace) || { echo "HUZC BUILD FAILED"; exit 1; }
  HUZC="../../huzc/target/debug/huzc$EXE_SUFFIX"
fi

# 时间戳/耗时归一化：[INFO] 行首时间、启动行内嵌时间、总耗时毫秒每次运行都变
norm() {
  sed -E -e 's/^\[INFO\] [0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2} - /[INFO] <TS> - /' \
         -e 's/\[[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}\]/[<TS>]/' \
         -e 's/总耗时: [0-9]+ ms/总耗时: <ELAPSED> ms/' "$1"
}

rm -f task_report.json
if ! "$HUZC" -i "src/main.hz" -o "test/out/task_engine" > test/out/build_task_engine.log 2>&1; then
  echo "FAIL(compile): task_engine"
  head -5 test/out/build_task_engine.log
  exit 1
fi

if ! run_limited "./test/out/task_engine$EXE_SUFFIX" > test/out/run_task_engine.log 2>&1; then
  code=$?
  echo "FAIL(run/$code): task_engine"
  cat test/out/run_task_engine.log
  exit 1
fi
rm -f task_report.json

norm test/expected/task_engine.stdout > test/out/expected_task_engine.norm
norm test/out/run_task_engine.log > test/out/run_task_engine.norm
if ! $DIFF_CMD -u test/out/expected_task_engine.norm test/out/run_task_engine.norm; then
  echo "FAIL(stdout): task_engine"
  exit 1
fi

echo "PASS: task_engine"
