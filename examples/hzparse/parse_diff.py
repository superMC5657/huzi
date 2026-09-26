#!/usr/bin/env python3
# parse_diff.py —— huzc --dump-parse-stats 与自举 hzparse 的九维对拍。
#
# 用法:
#   python examples/hzparse/parse_diff.py <huzc-bin> <hzparse-bin>
# 例如 (仓库根):
#   python examples/hzparse/parse_diff.py ./huzc/target/debug/huzc ./examples/hzparse/hzparse
#
# 对拍口径 (冻结):
#   成功文件: Rust 侧 stdout 单行 `fns=.. ... depth=..` 须与 hzparse 侧
#     `ok <path> fns=.. ... depth=..` 的统计段逐字节一致。
#   失败文件: 两侧须同为非零退出, 且首错行列 `L:C` 一致 (消息可不同)。
#     Rust 侧从 stdout+stderr 提 `parse-error L:C`/`lex-error L:C` 或
#     `line L, column C`; hzparse 侧提行尾 `L:C`。
#   语料清单只认 examples/hzparse/src/corpus.hz (仓库根相对路径),
#   另加 4 个内置负例 (残缺输入) 验首错行列。
#
# 退出码: 用法/路径错误返回 2; 全部 PASS 返回 0; 有 FAIL 返回 1.

import os
import re
import subprocess
import sys
import tempfile


def repo_root_of(script_path):
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(script_path))))


def load_corpus(repo):
    path = os.path.join(repo, "examples", "hzparse", "src", "corpus.hz")
    with open(path, encoding="utf-8") as f:
        txt = f.read()
    return re.findall(r'"([^"]+\.hz)"', txt)


def extract_stats_rust(out):
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("fns="):
            return line
    return ""


def extract_stats_hz(out):
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("ok "):
            parts = line.split(" ", 2)
            if len(parts) == 3:
                return parts[2]
    return ""


def extract_loc(text):
    m = re.search(r"(?:parse-error|lex-error)\s+(\d+):(\d+)", text)
    if m:
        return (int(m.group(1)), int(m.group(2)))
    m = re.search(r"at line (\d+), column (\d+)", text)
    if m:
        return (int(m.group(1)), int(m.group(2)))
    m = re.search(r"(\d+):(\d+)\s*$", text.strip())
    if m:
        return (int(m.group(1)), int(m.group(2)))
    m = re.search(r"(\d+):(\d+)", text)
    if m:
        return (int(m.group(1)), int(m.group(2)))
    return None


def run_one(huzc, hzparse, path):
    pa = subprocess.run(
        [huzc, "--dump-parse-stats", "-i", path], capture_output=True, text=True
    )
    pb = subprocess.run([hzparse, path], capture_output=True, text=True)
    ra_out = (pa.stdout or "") + (pa.stderr or "")
    rb_out = (pb.stdout or "") + (pb.stderr or "")
    if pa.returncode == 0 and pb.returncode == 0:
        sa = extract_stats_rust(pa.stdout or "")
        sb = extract_stats_hz(pb.stdout or "")
        if sa and sa == sb:
            return (True, "PASS: %s" % path)
        return (False, "FAIL(diff): %s rust=%r hz=%r" % (path, sa[:80], sb[:80]))
    if pa.returncode != 0 and pb.returncode != 0:
        la = extract_loc(ra_out)
        lb = extract_loc(rb_out)
        if la is not None and la == lb:
            return (True, "PASS(neg): %s %d:%d" % (path, la[0], la[1]))
        return (False, "FAIL(negloc): %s rust=%r hz=%r" % (path, la, lb))
    return (False, "FAIL(status): %s rust_rc=%d hz_rc=%d" % (path, pa.returncode, pb.returncode))


def run_negatives(huzc, hzparse):
    cases = [
        "fn f() -> i32 { return (1 + 2 }",
        "fn f(a: i32 { return a }",
        "fn f() -> i32 { return 1 + }",
        "fn broken( { ",
    ]
    ok = fail = 0
    for i, src in enumerate(cases):
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".hz", delete=False, encoding="utf-8"
        ) as tf:
            tf.write(src)
            name = tf.name
        try:
            passed, msg = run_one(huzc, hzparse, name)
        finally:
            os.unlink(name)
        print("%s [neg%d]" % (msg, i))
        if passed:
            ok += 1
        else:
            fail += 1
    return (ok, fail)


def main(argv):
    if len(argv) != 3:
        print("usage: parse_diff.py <huzc-bin> <hzparse-bin>", file=sys.stderr)
        return 2
    huzc, hzparse = argv[1], argv[2]
    for p in (huzc, hzparse):
        if not (os.path.isfile(p) and os.access(p, os.X_OK)):
            print("FAIL: binary not found/executable: %s" % p, file=sys.stderr)
            return 2
    repo = repo_root_of(argv[0])
    files = load_corpus(repo)
    passed = failed = 0
    for rel in files:
        full = os.path.join(repo, rel)
        if not os.path.isfile(full):
            print("FAIL(missing): %s" % rel)
            failed += 1
            continue
        ok, msg = run_one(huzc, hzparse, full)
        print(msg)
        if ok:
            passed += 1
        else:
            failed += 1
    nok, nfail = run_negatives(huzc, hzparse)
    passed += nok
    failed += nfail
    print("-----------------------------")
    print("%d passed, %d failed (total %d)" % (passed, failed, passed + failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
