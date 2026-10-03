#!/usr/bin/env python3
# parse_ast_diff.py -- huzc --dump-ast-json -i 与自举 hzparse --dump-ast-json -i 的 P1 对拍（P2-b 草稿，不入门禁）。
#
# 用法:
#   python examples/hzparse/parse_ast_diff.py <huzc-bin> <hzparse-bin>
# 例如 (仓库根):
#   python examples/hzparse/parse_ast_diff.py ./huzc/target/debug/huzc.exe ./examples/hzparse/hzparse.exe
#
# 对拍口径 (冻结，见 RFC_P2B_DRAFT.md §3，与 CONTRACT_DRAFT.md 冻结 1-3 同源):
#   语料 = src/corpus.hz 中 P1 子集交集（动态判定，无需另建清单）:
#     对 corpus.hz 逐文件跑 huzc --dump-ast-json -i <file> vs hzparse --dump-ast-json -i <file>。
#     双零退出 -> 按 split_lines 切分后须 len==1 且逐字节 == 即 PASS；
#       任一长度!=1 或内容 diff 即 FAIL（复用 examples/hzlex/dump_diff.py:10-20 判据，
#       二进制安全，不 strip；警告：不要换成 diff --strip-trailing-cr 或 sed）。
#     双非零退出 -> PASS(neg)（超子集，不比较内容）；一零一非零 -> FAIL(status)。
#   另加 2 内置用例（正/超各一，与本地验证同文件）:
#     正例 P1 最小 fact+main（if/return/call/let/print 全覆盖）须双零且逐字节一致。
#     超例 struct S 须双非零。
#
# 比较方法(冻结):两侧统一按 b'\n' 切分(尾空行 pop 保留),二进制安全,不 strip 任何字节。
#   判据:切后除末空外每一行都以 b'\r' 结尾即判 Windows-CRLF 模式,每行去一个尾部 b'\r'
#   (内容 \r + \r\n 行尾则留一个 \r,内容无损);否则判 LF/混合模式,原样保留(含内容 \r)。
#   此判据替代旧的 b'\r\n' in stdout 检测:后者在 dump 内容含 \r 时误触发致幽灵 FAIL。
#   警告:不要换成 `diff --strip-trailing-cr` 或 `sed 's/\r$//'`。
#   任一侧 dump 非零退出即按双非零/单零判，不比较内容。
#
# 退出码: 用法/路径错误返回 2; 全部 PASS 返回 0; 有 FAIL 返回 1.
# 覆盖率: 复用 parse_diff.py 的 fd==corpus 校验思路，但本草稿暂不做强制覆盖率门，
#   只打印 passed/failed（避免 P2-b 未定 P1 清单前误红；正式入门禁时再收紧）。
#
# 内置用例临时文件只放仓库树内（examples/hzparse/target/tmp/，已被 **/target/ 忽略），不用系统 Temp。

import os
import re
import subprocess
import sys


POS_SRC = "fn fact(n: i32) -> i32 { if n <= 1 { return 1 } return n * fact(n - 1) } fn main() -> i32 { let x = fact(5) print(x) return x }"
NEG_SRC = "struct S { x: i32 }"


def repo_root_of(script_path):
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(script_path))))


def load_corpus(repo):
    path = os.path.join(repo, "examples", "hzparse", "src", "corpus.hz")
    with open(path, encoding="utf-8") as f:
        txt = f.read()
    return re.findall(r'"([^"]+\.hz)"', txt)


def split_lines(raw):
    parts = raw.split(b'\n')
    if parts and parts[-1] == b'':
        parts.pop()
    if parts and all(p.endswith(b'\r') for p in parts):
        parts = [p[:-1] for p in parts]
    return parts


def run_one(huzc, hzparse, path):
    pa = subprocess.run([huzc, "--dump-ast-json", "-i", path], capture_output=True)
    pb = subprocess.run([hzparse, "--dump-ast-json", "-i", path], capture_output=True)
    if pa.returncode == 0 and pb.returncode == 0:
        la = split_lines(pa.stdout or b"")
        lb = split_lines(pb.stdout or b"")
        if len(la) == 1 and len(lb) == 1 and la[0] == lb[0]:
            return (True, "PASS: %s" % path)
        if len(la) != 1 or len(lb) != 1:
            return (False, "FAIL(len): %s rust=%d hz=%d" % (path, len(la), len(lb)))
        n = min(len(la[0]), len(lb[0]))
        pos = next((i for i in range(n) if la[0][i] != lb[0][i]), n)
        return (False, "FAIL(diff): %s at=%d rust=%r hz=%r" % (path, pos, la[0][max(0, pos - 20):pos + 50], lb[0][max(0, pos - 20):pos + 50]))
    if pa.returncode != 0 and pb.returncode != 0:
        return (True, "PASS(neg): %s rust_rc=%d hz_rc=%d" % (path, pa.returncode, pb.returncode))
    return (False, "FAIL(status): %s rust_rc=%d hz_rc=%d" % (path, pa.returncode, pb.returncode))


def run_builtin(huzc, hzparse, repo):
    tmpdir = os.path.join(repo, "examples", "hzparse", "target", "tmp", "parse_ast_builtin")
    os.makedirs(tmpdir, exist_ok=True)
    pos_path = os.path.join(tmpdir, "p1_pos.hz")
    neg_path = os.path.join(tmpdir, "p1_neg.hz")
    with open(pos_path, "w", encoding="utf-8", newline="\n") as f:
        f.write(POS_SRC)
    with open(neg_path, "w", encoding="utf-8", newline="\n") as f:
        f.write(NEG_SRC)
    try:
        ok1, msg1 = run_one(huzc, hzparse, pos_path)
        print("%s [builtin-pos]" % msg1)
        ok2, msg2 = run_one(huzc, hzparse, neg_path)
        print("%s [builtin-neg]" % msg2)
        # 内置正例须双零逐字节一致，超例须双非零；run_one 已按此判，额外收紧语义：
        # 若正例走了 PASS(neg)（双非零）亦视为 FAIL，避免超集误判掩盖。
        if ok1 and "(neg)" in msg1:
            print("FAIL(builtin-pos-neg): positive case must be double-zero, got double-nonzero")
            ok1 = False
        return ((1 if ok1 else 0), (1 if not ok1 else 0), (1 if ok2 else 0), (1 if not ok2 else 0))
    finally:
        try:
            os.unlink(pos_path)
        except OSError:
            pass
        try:
            os.unlink(neg_path)
        except OSError:
            pass


def main(argv):
    if len(argv) != 3:
        print("usage: parse_ast_diff.py <huzc-bin> <hzparse-bin>", file=sys.stderr)
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
            # 语料缺失按超集外处理：只提示，不计 FAIL（草稿阶段不收紧覆盖率）。
            print("SKIP(missing): %s" % rel)
            continue
        ok, msg = run_one(huzc, hzparse, full)
        print(msg)
        if ok:
            passed += 1
        else:
            failed += 1
    bpass, bfail, npass, nfail = run_builtin(huzc, hzparse, repo)
    passed += bpass + npass
    failed += bfail + nfail
    print("-----------------------------")
    print("%d passed, %d failed (total %d)" % (passed, failed, passed + failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
