#!/bin/bash
# huzi-src 标准库回归自测脚本
set -u
cd "$(dirname "$0")" || exit 1

mkdir -p test/out
export HUZI_TEST_ENV="huzi_env_ok"

EXE_SUFFIX=""
case "$(uname -s 2>/dev/null)" in
  MINGW*|MSYS*|CYGWIN*|Windows*) EXE_SUFFIX=".exe" ;;
esac

DIFF_CMD="diff"
if diff --strip-trailing-cr /dev/null /dev/null > /dev/null 2>&1; then
  DIFF_CMD="diff --strip-trailing-cr"
fi

run_limited() {
  if command -v timeout > /dev/null 2>&1; then timeout 10 "$@"; else "$@"; fi
}

HUZC="../huzc/target/debug/huzc$EXE_SUFFIX"
if [ ! -f "$HUZC" ]; then
  HUZC="../huzc/target/release/huzc$EXE_SUFFIX"
fi
if [ ! -f "$HUZC" ]; then
  (cd ../huzc && cargo build --workspace) || { echo "HUZC BUILD FAILED"; exit 1; }
  HUZC="../huzc/target/debug/huzc$EXE_SUFFIX"
fi

pass=0
fail=0

for f in test/*_test.hz; do
  [ -e "$f" ] || continue
  name=$(basename "$f" .hz)

  if ! "$HUZC" -i "$f" -o "test/out/$name" > test/out/build_$name.log 2>&1; then
    echo "FAIL(compile): $name"
    head -5 test/out/build_$name.log
    fail=$((fail+1))
    continue
  fi

  if ! run_limited "./test/out/$name$EXE_SUFFIX" > "test/out/run_$name.log" 2>&1; then
    code=$?
    echo "FAIL(run/$code): $name"
    cat "test/out/run_$name.log"
    fail=$((fail+1))
    continue
  fi

  if [ -f "test/expected/$name.stdout" ]; then
    if ! $DIFF_CMD -q "test/expected/$name.stdout" "test/out/run_$name.log" > /dev/null 2>&1; then
      echo "FAIL(stdout): $name"
      $DIFF_CMD "test/expected/$name.stdout" "test/out/run_$name.log" | head -5
      fail=$((fail+1))
      continue
    fi
  fi

  echo "PASS: $name"
  pass=$((pass+1))
done

echo "-----------------------------"
echo "$pass passed, $fail failed"
[ $fail -eq 0 ] || exit 1
