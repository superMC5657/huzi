#!/usr/bin/env python3
# ast_diff.py —— huzc --ast-json-test 与 hzast --dump-json 的 30 向量对拍（C2+P3+P3b）。
#
# 用法:
#   python examples/hzast/ast_diff.py <huzc-bin> <hzast-bin>
# 例如 (仓库根):
#   huzc build --path examples/hzast --output /tmp/hzast
#   python examples/hzast/ast_diff.py ./huzc/target/debug/huzc /tmp/hzast
#
# 对拍口径 (P3 扩展，见 examples/hzast/RFC.md §2 + P3 增补):
#   语料为 30 个固定向量 ID（非 .hz 文件；Huzi 侧无通用解析器）：
#     expr_num/expr_bool/expr_str/expr_str_esc/expr_var/expr_bin/expr_un/expr_call/
#     expr_tuple/expr_array/expr_index/expr_enum_some/expr_enum_none/expr_enum_ok/expr_enum_err/expr_try/
#     expr_struct/expr_field/expr_method/expr_fstring/type_atom/type_applied/
#     stmt_let/stmt_assign/stmt_if/stmt_while/stmt_return/stmt_print/stmt_expr/prog_fact
#   Rust 侧 `huzc --ast-json-test <id>` 与 Huzi 侧 `hzast --dump-json <id>`
#   各打印单行紧凑 JSON；按 b'\n' 切分、二进制安全比较，不 strip。
#   任一侧非零退出即记该向量 FAIL；未知 ID 两侧须同为非零退出（另测）。
#
# 退出码: 用法/路径错误返回 2; 全部 PASS 返回 0; 有 FAIL 返回 1.

import os
import subprocess
import sys

IDS = [
    "expr_num",
    "expr_bool",
    "expr_str",
    "expr_str_esc",
    "expr_var",
    "expr_bin",
    "expr_un",
    "expr_call",
    "expr_tuple",
    "expr_array",
    "expr_index",
    "expr_enum_some",
    "expr_enum_none",
    "expr_enum_ok",
    "expr_enum_err",
    "expr_try",
    "expr_struct",
    "expr_field",
    "expr_method",
    "expr_fstring",
    "type_atom",
    "type_applied",
    "stmt_let",
    "stmt_assign",
    "stmt_if",
    "stmt_while",
    "stmt_return",
    "stmt_print",
    "stmt_expr",
    "prog_fact",
]


def split_lines(raw):
    parts = raw.split(b'\n')
    if parts and parts[-1] == b'':
        parts.pop()
    if parts and all(p.endswith(b'\r') for p in parts):
        parts = [p[:-1] for p in parts]
    return parts


def run_one(huzc, hzast, vid):
    pa = subprocess.run([huzc, "--ast-json-test", vid], capture_output=True)
    pb = subprocess.run([hzast, "--dump-json", vid], capture_output=True)
    if pa.returncode != 0 or pb.returncode != 0:
        return (False, "FAIL(status): %s rust_rc=%d hz_rc=%d" % (vid, pa.returncode, pb.returncode))
    la = split_lines(pa.stdout)
    lb = split_lines(pb.stdout)
    if len(la) != 1 or len(lb) != 1:
        return (False, "FAIL(len): %s rust=%d hz=%d" % (vid, len(la), len(lb)))
    if la[0] == lb[0]:
        return (True, "PASS: %s" % vid)
    n = min(len(la[0]), len(lb[0]))
    pos = next((i for i in range(n) if la[0][i] != lb[0][i]), n)
    return (
        False,
        "FAIL(diff): %s at=%d rust=%r hz=%r"
        % (vid, pos, la[0][max(0, pos - 20):pos + 50], lb[0][max(0, pos - 20):pos + 50]),
    )


def main(argv):
    if len(argv) != 3:
        print("usage: ast_diff.py <huzc-bin> <hzast-bin>", file=sys.stderr)
        return 2
    huzc, hzast = argv[1], argv[2]
    for p in (huzc, hzast):
        if not (os.path.isfile(p) and os.access(p, os.X_OK)):
            print("FAIL: binary not found/executable: %s" % p, file=sys.stderr)
            return 2
    passed = failed = 0
    for vid in IDS:
        ok, msg = run_one(huzc, hzast, vid)
        print(msg)
        if ok:
            passed += 1
        else:
            failed += 1
    pa = subprocess.run([huzc, "--ast-json-test", "no_such_id"], capture_output=True)
    pb = subprocess.run([hzast, "--dump-json", "no_such_id"], capture_output=True)
    if pa.returncode != 0 and pb.returncode != 0:
        print("PASS(neg): unknown id both non-zero")
        passed += 1
    else:
        print("FAIL(neg): unknown id rust_rc=%d hz_rc=%d" % (pa.returncode, pb.returncode))
        failed += 1
    print("-----------------------------")
    print("%d passed, %d failed (total %d)" % (passed, failed, passed + failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
