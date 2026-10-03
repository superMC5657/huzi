#!/usr/bin/env python3
# eval_diff.py —— hzast --dump-eval 与原生 eval_ref.hz 的 20 行对拍（C1）。
#
# 用法:
#   python examples/hzast/eval_diff.py <huzc-bin> <hzast-bin>
# 例如 (仓库根):
#   huzc build --path examples/hzast --output /tmp/hzast
#   python examples/hzast/eval_diff.py ./huzc/target/debug/huzc /tmp/hzast
#
# 对拍口径 (P3 扩展，见 examples/hzast/RFC.md §1 + P3 增补):
#   hzast 侧 `hzast --dump-eval` 打印 20 行 `name=value`（顺序固定）；
#   原生侧由 huzc 即时编译 `examples/hzast/eval_ref.hz`（直写算式，不经 eval）
#   并执行得同样 20 行；两侧按 b'\n' 切分、二进制安全比较，不 strip。
#   任一侧非零退出即记 FAIL，不比较内容。
#
# 退出码: 用法/路径错误返回 2; 20/20 PASS 返回 0; 有 FAIL 返回 1.

import os
import subprocess
import sys
import tempfile


def repo_root_of(script_path):
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(script_path))))


def split_lines(raw):
    parts = raw.split(b'\n')
    if parts and parts[-1] == b'':
        parts.pop()
    if parts and all(p.endswith(b'\r') for p in parts):
        parts = [p[:-1] for p in parts]
    return parts


def build_ref(huzc, repo):
    src = os.path.join(repo, "examples", "hzast", "eval_ref.hz")
    # Windows: mkstemp 产物加 .exe 后缀，否则 huzc 输出 <path>.exe 而脚本
    # 起 subprocess.run([path]) 会 WinError 193；POSIX 保持无后缀不变。
    suffix = ".exe" if os.name == "nt" else ""
    fd, path = tempfile.mkstemp(prefix="eval_ref_", suffix=suffix)
    os.close(fd)
    pa = subprocess.run(
        [huzc, "--input", src, "--output", path],
        capture_output=True,
    )
    if pa.returncode != 0:
        for cand in (path, path + ".exe"):
            try:
                os.unlink(cand)
            except OSError:
                pass
        return (None, pa.stderr.decode("utf-8", "replace")[-500:])
    # 回退：若 huzc 在 Windows 对裸 path 另行追加 .exe，取实际落盘者；
    # POSIX 下首候选即中，行为不变。
    for cand in (path, path + ".exe"):
        if os.path.isfile(cand) and os.path.getsize(cand) > 0:
            actual = cand
            break
    else:
        for cand in (path, path + ".exe"):
            try:
                os.unlink(cand)
            except OSError:
                pass
        return (None, "ref binary missing: %s[.exe]" % path)
    if actual != path:
        try:
            os.unlink(path)
        except OSError:
            pass
    return (actual, "")


def main(argv):
    if len(argv) != 3:
        print("usage: eval_diff.py <huzc-bin> <hzast-bin>", file=sys.stderr)
        return 2
    huzc, hzast = argv[1], argv[2]
    for p in (huzc, hzast):
        if not (os.path.isfile(p) and os.access(p, os.X_OK)):
            print("FAIL: binary not found/executable: %s" % p, file=sys.stderr)
            return 2
    repo = repo_root_of(argv[0])
    ref_bin, err = build_ref(huzc, repo)
    if ref_bin is None:
        print("FAIL(build-ref): %s" % err)
        return 1
    try:
        pa = subprocess.run([hzast, "--dump-eval"], capture_output=True)
        try:
            pb = subprocess.run([ref_bin], capture_output=True)
        except OSError as e:
            # 曾见 WinError 193（无 .exe 后缀空占位）；修后不应再触发，
            # 兜底记 FAIL 而非 traceback 崩。
            print("FAIL(run-ref): OSError: %s" % e)
            return 1
        if pa.returncode != 0:
            print("FAIL(dump-hzast): rc=%d" % pa.returncode)
            return 1
        if pb.returncode != 0:
            print("FAIL(run-ref): rc=%d" % pb.returncode)
            return 1
        la = split_lines(pa.stdout)
        lb = split_lines(pb.stdout)
        names = [b"arithmetic=", b"variables=", b"conditional=", b"while=", b"fact=", b"str=", b"tuple=", b"array=", b"index=", b"option_some=", b"option_none=", b"result_ok=", b"result_err=", b"result_try=", b"struct=", b"field=", b"method=", b"fstring=", b"type_atom=", b"type_applied="]
        if len(la) != 20 or len(lb) != 20:
            print("FAIL(len): hzast=%d ref=%d" % (len(la), len(lb)))
            return 1
        passed = failed = 0
        for i, (x, y) in enumerate(zip(la, lb)):
            if x == y and x.startswith(names[i]):
                print("PASS: %s" % x.decode("utf-8", "replace"))
                passed += 1
            else:
                print("FAIL(diff): hzast=%r ref=%r" % (x[:70], y[:70]))
                failed += 1
        print("-----------------------------")
        print("%d passed, %d failed (total %d)" % (passed, failed, passed + failed))
        return 1 if failed else 0
    finally:
        try:
            os.unlink(ref_bin)
        except OSError:
            pass


if __name__ == "__main__":
    sys.exit(main(sys.argv))
