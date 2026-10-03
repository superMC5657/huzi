#!/usr/bin/env python3
# hzir_diff.py -- huzc 直编译与 hzir JSON-lower 双编译 stdout 对拍（P4-a 草稿，不入门禁）。
#
# 用法:
#   python examples/hzir/hzir_diff.py <huzc-bin> <hzir-bin>
# 例如 (仓库根):
#   python examples/hzir/hzir_diff.py ./huzc/target/debug/huzc.exe ./examples/hzir/target/hzir.exe
#
# 对拍口径 (冻结，见 examples/hzir/RFC.md §4；P4-b 起模板用例见 RFC §5)：
#   语料 = 3 个已放开用例（s1_hello/s2_arith/s5_sum）：
#     1. JSON 新鲜度：huzc --dump-ast-json -i cases/<name>.hz 经 split_lines 后须
#        len==1 且逐字节 == json/<name>.json（P2-b JSON，huzc/hzparse 双零一致）。
#     2. 双编译：huzc -i cases/<name>.hz -o target/direct_<key> 直编译运行 stdout
#        vs hzir 二进制已生成的 target/hzir_s<N>j.exe（JSON→IR→clang 全在 Huzi 内）
#        运行 stdout，双零退出且经 split_lines 后逐字节一致。
#     3. 期望：expected/<key>.stdout 须以 b'\n' 结尾（含末尾换行），经 split_lines
#        后与上述两者逐字节一致（三方一致即 PASS）。
#   模板用例（P4-b 起：s9_for_range/s10_for_sum；P4-c 加 s11_vec；
#   P4-d 加 s12_generic/s13_trait；JSON 在 P1 子集外，dump 非零）：
#     1. JSON 否定：huzc --dump-ast-json -i cases/<name>.hz 须非零退出（P1 外，
#        与 RFC_P2B 超子集双非零同口径；若变零说明该用例已入 P1，应转 JSON-lower）。
#     2. 双编译：直编译运行 stdout vs hzir 二进制已生成的 target/hzir_s<tag>.exe
#        （模板 IR→clang 全在 Huzi 内）运行 stdout，双零退出且三方与 expected
#        经 split_lines 后逐字节一致（无旧.ll==新.ll 项，模板只有一份 IR）。
#   8 个原模板（emit_s*）由 hzir 二进制自检（exit 0 + "hzir all 13 pass"）覆盖，
#   本脚本另验其 3 对应新旧 IR 逐字节一致（target/hzir_s<N>.ll == target/hzir_s<N>j.ll）。
#
# 比较方法(冻结，复用 hzlex/dump_diff.py:10-20 与 hzparse/parse_ast_diff.py:20-25)：
#   两侧统一按 b'\n' 切分（尾空行 pop 保留），二进制安全，不 strip 任何字节。
#   判据：切后除末空外每一行都以 b'\r' 结尾即判 Windows-CRLF 模式，每行去一个尾部
#   b'\r'（内容 \r + \r\n 行尾则留一个 \r，内容无损）；否则判 LF/混合模式原样保留。
#   警告：不要换成 diff --strip-trailing-cr 或 sed。
#   任一侧非零退出即记该用例 FAIL，不比较内容。
#
# 退出码：用法/路径错误返回 2；全部 PASS 返回 0；有 FAIL 返回 1。
#
# 构建产物与临时文件只放仓库树内（examples/hzir/target/，已被 **/target/ 忽略），
# 不用系统 Temp。本草稿不接入 check.sh。

import os
import subprocess
import sys


CASES = [
    ("s1_hello", "s1_hello.hz", "s1_hello.json", "s1_hello.stdout", "1", "1j"),
    ("s2_arith", "s2_arith.hz", "s2_arith.json", "s2_arith.stdout", "2", "2j"),
    ("s5_sum", "s5_sum.hz", "s5_sum.json", "s5_sum.stdout", "5", "5j"),
]

# 模板用例（P4-b 起；tag 为 hzir_s<tag> 后缀，无 json 项）。
TEMPLATE_CASES = [
    ("s9_for_range", "s9_for_range.hz", "s9_for_range.stdout", "9"),
    ("s10_for_sum", "s10_for_sum.hz", "s10_for_sum.stdout", "10"),
    ("s11_vec", "s11_vec.hz", "s11_vec.stdout", "11"),
    ("s12_generic", "s12_generic.hz", "s12_generic.stdout", "12"),
    ("s13_trait", "s13_trait.hz", "s13_trait.stdout", "13"),
]


def repo_root_of(script_path):
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(script_path))))


def split_lines(raw):
    parts = raw.split(b'\n')
    if parts and parts[-1] == b'':
        parts.pop()
    if parts and all(p.endswith(b'\r') for p in parts):
        parts = [p[:-1] for p in parts]
    return parts


def run_capture(argv):
    try:
        pa = subprocess.run(argv, capture_output=True)
    except OSError as e:
        return (None, b'', 'OSError: %s' % e)
    return (pa.returncode, pa.stdout or b'', pa.stderr or b'')


def check_json_fresh(huzc, repo, case, hz, js):
    full_hz = os.path.join(repo, 'examples', 'hzir', 'cases', hz)
    full_js = os.path.join(repo, 'examples', 'hzir', 'json', js)
    if not os.path.isfile(full_js):
        return (False, 'FAIL(json-missing): %s' % js)
    rc, out, _ = run_capture([huzc, '--dump-ast-json', '-i', full_hz])
    if rc != 0:
        return (False, 'FAIL(json-huzc-status): %s rc=%s' % (case, rc))
    try:
        with open(full_js, 'rb') as f:
            exp = f.read()
    except OSError as e:
        return (False, 'FAIL(json-read): %s %s' % (js, e))
    la, lb = split_lines(out), split_lines(exp)
    if len(la) == 1 and len(lb) == 1 and la[0] == lb[0]:
        return (True, 'PASS(json): %s' % case)
    return (False, 'FAIL(json-diff): %s rust=%d json=%d' % (case, len(la), len(lb)))


def build_direct(huzc, repo, hz, key):
    full_hz = os.path.join(repo, 'examples', 'hzir', 'cases', hz)
    out_base = os.path.join(repo, 'examples', 'hzir', 'target', 'direct_' + key)
    rc, out, err = run_capture([huzc, '-i', full_hz, '-o', out_base])
    if rc != 0:
        return (False, None, 'FAIL(direct-build): %s rc=%d err=%r' % (key, rc, (err or b'')[:120]))
    for cand in (out_base + '.exe', out_base):
        if os.path.isfile(cand):
            return (True, cand, '')
    return (False, None, 'FAIL(direct-missing): %s' % key)


def check_case(huzc, repo, case, hz, js, exp_name, old_tag, json_tag):
    ok, msg = check_json_fresh(huzc, repo, case, hz, js)
    print(msg)
    if not ok:
        return False
    okb, direct_exe, emsg = build_direct(huzc, repo, hz, case)
    if not okb:
        print(emsg)
        return False
    rc1, out1, _ = run_capture([direct_exe])
    if rc1 != 0:
        print('FAIL(direct-run): %s rc=%s' % (case, rc1))
        return False
    json_exe = os.path.join(repo, 'examples', 'hzir', 'target', 'hzir_s' + json_tag + '.exe')
    if not os.path.isfile(json_exe):
        # 兼容无后缀（Linux 直出无 .exe）
        alt = os.path.join(repo, 'examples', 'hzir', 'target', 'hzir_s' + json_tag)
        if os.path.isfile(alt):
            json_exe = alt
        else:
            print('FAIL(json-exe-missing): %s' % case)
            return False
    rc2, out2, _ = run_capture([json_exe])
    if rc2 != 0:
        print('FAIL(json-run): %s rc=%s' % (case, rc2))
        return False
    exp_path = os.path.join(repo, 'examples', 'hzir', 'expected', exp_name)
    try:
        with open(exp_path, 'rb') as f:
            exp_raw = f.read()
    except OSError as e:
        print('FAIL(expected-read): %s %s' % (case, e))
        return False
    if not exp_raw.endswith(b'\n'):
        print('FAIL(expected-nonl): %s missing trailing newline' % case)
        return False
    l1, l2, le = split_lines(out1), split_lines(out2), split_lines(exp_raw)
    if l1 == l2 == le:
        print('PASS: %s direct==json==expected (%d lines)' % (case, len(le)))
    else:
        print('FAIL(diff): %s direct=%r json=%r expected=%r' % (case, l1[:3], l2[:3], le[:3]))
        return False
    # 新旧 IR 一致（模板 vs JSON-lower 逐字节）
    old_ll = os.path.join(repo, 'examples', 'hzir', 'target', 'hzir_s' + old_tag + '.ll')
    new_ll = os.path.join(repo, 'examples', 'hzir', 'target', 'hzir_s' + json_tag + '.ll')
    try:
        with open(old_ll, 'rb') as f:
            a = f.read()
        with open(new_ll, 'rb') as f:
            b = f.read()
    except OSError as e:
        print('FAIL(ll-read): %s %s' % (case, e))
        return False
    if a == b:
        print('PASS(ll): %s old==json (%d bytes)' % (case, len(a)))
        return True
    print('FAIL(ll-diff): %s old=%d json=%d' % (case, len(a), len(b)))
    return False


def check_template_case(huzc, repo, case, hz, exp_name, tag):
    # 1. JSON 否定：P1 子集外须非零（模板-only 契约）。
    full_hz = os.path.join(repo, 'examples', 'hzir', 'cases', hz)
    rc0, _, _ = run_capture([huzc, '--dump-ast-json', '-i', full_hz])
    if rc0 == 0:
        print('FAIL(json-neg): %s dump unexpectedly zero (should stay template-only)' % case)
        return False
    print('PASS(json-neg): %s dump nonzero (template-only)' % case)
    # 2. 双编译：direct vs 模板 exe vs expected 三方一致。
    okb, direct_exe, emsg = build_direct(huzc, repo, hz, case)
    if not okb:
        print(emsg)
        return False
    rc1, out1, _ = run_capture([direct_exe])
    if rc1 != 0:
        print('FAIL(direct-run): %s rc=%s' % (case, rc1))
        return False
    tpl_exe = os.path.join(repo, 'examples', 'hzir', 'target', 'hzir_s' + tag + '.exe')
    if not os.path.isfile(tpl_exe):
        alt = os.path.join(repo, 'examples', 'hzir', 'target', 'hzir_s' + tag)
        if os.path.isfile(alt):
            tpl_exe = alt
        else:
            print('FAIL(tpl-exe-missing): %s' % case)
            return False
    rc2, out2, _ = run_capture([tpl_exe])
    if rc2 != 0:
        print('FAIL(tpl-run): %s rc=%s' % (case, rc2))
        return False
    exp_path = os.path.join(repo, 'examples', 'hzir', 'expected', exp_name)
    try:
        with open(exp_path, 'rb') as f:
            exp_raw = f.read()
    except OSError as e:
        print('FAIL(expected-read): %s %s' % (case, e))
        return False
    if not exp_raw.endswith(b'\n'):
        print('FAIL(expected-nonl): %s missing trailing newline' % case)
        return False
    l1, l2, le = split_lines(out1), split_lines(out2), split_lines(exp_raw)
    if l1 == l2 == le:
        print('PASS: %s direct==template==expected (%d lines)' % (case, len(le)))
        return True
    print('FAIL(diff): %s direct=%r template=%r expected=%r' % (case, l1[:3], l2[:3], le[:3]))
    return False


def main(argv):
    if len(argv) != 3:
        print('usage: hzir_diff.py <huzc-bin> <hzir-bin>', file=sys.stderr)
        return 2
    huzc, hzir = argv[1], argv[2]
    for p in (huzc, hzir):
        if not (os.path.isfile(p) and os.access(p, os.X_OK)):
            print('FAIL: binary not found/executable: %s' % p, file=sys.stderr)
            return 2
    repo = repo_root_of(argv[0])
    # 先跑 hzir 二进制生成 .ll/.exe（13 模板 + 3 JSON 全在 Huzi 内 write+clang+run）
    rc, out, _ = run_capture([hzir])
    if rc != 0:
        print('FAIL(hzir-run): rc=%d out=%r' % (rc, (out or b'')[-300:]))
        return 1
    if b'hzir all 13 pass' not in (out or b'') or b'hzir json 3 pass' not in (out or b''):
        print('FAIL(hzir-passmark): missing all-13/json-3 marks')
        return 1
    print('PASS(hzir-bin): old 13 + json 3 selfcheck')
    passed = failed = 0
    for case, hz, js, exp_name, old_tag, json_tag in CASES:
        if check_case(huzc, repo, case, hz, js, exp_name, old_tag, json_tag):
            passed += 1
        else:
            failed += 1
    for case, hz, exp_name, tag in TEMPLATE_CASES:
        if check_template_case(huzc, repo, case, hz, exp_name, tag):
            passed += 1
        else:
            failed += 1
    print('-----------------------------')
    print('%d passed, %d failed (total %d)' % (passed, failed, passed + failed))
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
