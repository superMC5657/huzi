#!/bin/bash
# Regression test: compile and run every example, assert exit code AND stdout.
# Negative tests must fail: *.compile_fail.hz at compile time,
# *.runtime_fail.hz at run time.
set -u
cd "$(dirname "$0")" || exit 1

mkdir -p out

cargo build 2>&1 | grep -E "^error" && { echo "BUILD FAILED"; exit 1; }

pass=0
fail=0

# 24_pipe_read 从 stdin 逐行读取,无重定向会阻塞等键盘输入,需要喂管道数据。
printf 'a\nb\n' > out/pipe_input.txt

# 10_guess_number_game 是交互式示例(等待键盘猜数),回归中跳过。
for f in examples/*.hz; do
  name=$(basename "$f" .hz)
  if [ "$name" = "10_guess_number_game" ]; then
    echo "SKIP(interactive): $name"
    continue
  fi
  if ! ./target/debug/huzc.exe -i "$f" -o "out/$name" > /tmp/huzc_build.log 2>&1; then
    echo "FAIL(compile): $name"
    grep -E "error" /tmp/huzc_build.log | head -1
    fail=$((fail+1))
    continue
  fi
  stdin_file="/dev/null"
  [ "$name" = "24_pipe_read" ] && stdin_file="out/pipe_input.txt"
  if ! timeout 10 "./out/$name.exe" < "$stdin_file" > /tmp/huzc_run.log 2>&1; then
    code=$?
    echo "FAIL(run/$code): $name"
    fail=$((fail+1))
    continue
  fi
  if ! diff -q "test/expected/$name.stdout" /tmp/huzc_run.log > /dev/null 2>&1; then
    echo "FAIL(stdout): $name"
    diff "test/expected/$name.stdout" /tmp/huzc_run.log | head -5
    fail=$((fail+1))
    continue
  fi
  echo "PASS: $name"
  pass=$((pass+1))
done

for f in test/neg/*.compile_fail.hz; do
  name=$(basename "$f" .hz)
  if ./target/debug/huzc.exe -i "$f" -o "out/neg_$name" > /tmp/huzc_neg.log 2>&1; then
    echo "FAIL(neg-compile-should-fail): $name"
    fail=$((fail+1))
  else
    echo "PASS(neg-compile): $name"
    pass=$((pass+1))
  fi
done

for f in test/neg/*.runtime_fail.hz; do
  name=$(basename "$f" .hz)
  if ! ./target/debug/huzc.exe -i "$f" -o "out/neg_$name" > /tmp/huzc_neg.log 2>&1; then
    echo "FAIL(neg-runtime-compile): $name"
    fail=$((fail+1))
    continue
  fi
  if timeout 10 "./out/neg_$name.exe" < /dev/null > /tmp/huzc_neg_run.log 2>&1; then
    echo "FAIL(neg-runtime-should-fail): $name"
    fail=$((fail+1))
  else
    echo "PASS(neg-runtime): $name"
    pass=$((pass+1))
  fi
done

echo "-----------------------------"
echo "$pass passed, $fail failed"
[ $fail -eq 0 ]
