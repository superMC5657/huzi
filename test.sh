#!/bin/bash
# Regression test: compile and run every example, assert exit code AND stdout.
# Negative tests must fail: *.compile_fail.hz at compile time,
# *.runtime_fail.hz at run time.
set -u
cd "$(dirname "$0")" || exit 1

mkdir -p test/out

# 平台自适应:Windows 可执行文件带 .exe 后缀,类 Unix 无后缀;
# macOS 缺 GNU timeout,退化为直接运行。
EXE_SUFFIX=""
case "$(uname -s 2>/dev/null)" in
  MINGW*|MSYS*|CYGWIN*|Windows*) EXE_SUFFIX=".exe" ;;
esac
HUZC="./target/debug/huzc$EXE_SUFFIX"
DIFF_CMD="diff"
if diff --strip-trailing-cr /dev/null /dev/null > /dev/null 2>&1; then
  DIFF_CMD="diff --strip-trailing-cr"
fi
run_limited() {
  if command -v timeout > /dev/null 2>&1; then timeout 10 "$@"; else "$@"; fi
}

cargo build --workspace || { echo "BUILD FAILED"; exit 1; }

pass=0
fail=0

# 24_pipe_read 从 stdin 逐行读取,无重定向会阻塞等键盘输入,需要喂管道数据。
printf 'a\nb\n' > test/out/pipe_input.txt

# 10_guess_number_game 是交互式示例(等待键盘猜数),回归中跳过。
for f in test/examples/*.hz; do
  name=$(basename "$f" .hz)
  if [ "$name" = "10_guess_number_game" ]; then
    echo "SKIP(interactive): $name"
    continue
  fi
  if ! "$HUZC" -i "$f" -o "test/out/$name" > /tmp/huzc_build.log 2>&1; then
    echo "FAIL(compile): $name"
    grep -E "error" /tmp/huzc_build.log | head -1
    fail=$((fail+1))
    continue
  fi
  stdin_file="/dev/null"
  [ "$name" = "24_pipe_read" ] && stdin_file="test/out/pipe_input.txt"
  if ! run_limited "./test/out/$name$EXE_SUFFIX" < "$stdin_file" > /tmp/huzc_run.log 2>&1; then
    code=$?
    echo "FAIL(run/$code): $name"
    fail=$((fail+1))
    continue
  fi
  if ! $DIFF_CMD -q "test/expected/$name.stdout" /tmp/huzc_run.log > /dev/null 2>&1; then
    echo "FAIL(stdout): $name"
    $DIFF_CMD "test/expected/$name.stdout" /tmp/huzc_run.log | head -5
    fail=$((fail+1))
    continue
  fi
  echo "PASS: $name"
  pass=$((pass+1))
done

for f in test/neg/*.compile_fail.hz; do
  name=$(basename "$f" .hz)
  if "$HUZC" -i "$f" -o "test/out/neg_$name" > /tmp/huzc_neg.log 2>&1; then
    echo "FAIL(neg-compile-should-fail): $name"
    fail=$((fail+1))
  else
    echo "PASS(neg-compile): $name"
    pass=$((pass+1))
  fi
done

for f in test/neg/*.runtime_fail.hz; do
  name=$(basename "$f" .hz)
  if ! "$HUZC" -i "$f" -o "test/out/neg_$name" > /tmp/huzc_neg.log 2>&1; then
    echo "FAIL(neg-runtime-compile): $name"
    fail=$((fail+1))
    continue
  fi
  if run_limited "./test/out/neg_$name$EXE_SUFFIX" < /dev/null > /tmp/huzc_neg_run.log 2>&1; then
    echo "FAIL(neg-runtime-should-fail): $name"
    fail=$((fail+1))
  else
    echo "PASS(neg-runtime): $name"
    pass=$((pass+1))
  fi
done

echo "-----------------------------"
echo "$pass passed, $fail failed"
[ $fail -eq 0 ] || exit 1

if [ "${RUN_BENCH:-0}" = "1" ] || [ "${1:-}" = "--bench" ]; then
  echo "==> 运行性能回归门禁 (bench_compare.py)..."
  PY=""
  for cand in python3 python py; do
    if "$cand" -c "import sys; exit(0 if sys.version_info[0] >= 3 else 1)" >/dev/null 2>&1; then
      PY="$cand"
      break
    fi
  done
  [ -n "$PY" ] || { echo "FAIL: 未检测到有效的 Python 3 环境"; exit 1; }
  "$PY" test/bench_compare.py || { echo "FAIL: 性能回归门禁未通过"; exit 1; }
fi

