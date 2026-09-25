#!/usr/bin/env python3
# dump_diff.py —— hzlex 与 huzc --dump-tokens 的文本对拍(只做文本比较,不解析语义)。
#
# 用法:
#   python examples/hzlex/dump_diff.py <huzc-bin> <hzlex-bin> <corpus-root>
# 例如:
#   python examples/hzlex/dump_diff.py ./huzc/target/debug/huzc ./examples/hzlex/hzlex huzc/test/cases
#   python examples/hzlex/dump_diff.py ./huzc/target/debug/huzc ./examples/hzlex/hzlex huzi-src
#
# 比较方法(冻结):huzc 输出 LF 行尾,hzlex 经 C 运行时输出 CRLF 行尾,
#   故 huzc 侧按 b'\\n' 切分、hzlex 侧按 b'\\r\\n' 切分后逐行逐字节比较,
#   不 strip 任何字节。
#   警告:不要换成 `diff --strip-trailing-cr` 或 `sed 's/\\r$//'` —
#   实测前者在部分 diff 构建下对含 CR 内容行比较不可靠(幽灵 FAIL),
#   后者会吃掉文本尾部真正的 content-\\r,两者都曾把全绿误报成 40 FAIL。
#   任一侧 dump 非零退出即记该文件 FAIL,不比较内容。
#   dump 只读源文件、不执行、不读 stdin,故交互式用例(如 10_guess_number_game)
#   无需跳过,照常对拍。
#
# 退出码:用法/路径错误返回 2;全部 PASS 返回 0;有 FAIL 返回 1。

import os
import subprocess
import sys


def main(argv):
    if len(argv) != 4:
        print('usage: dump_diff.py <huzc-bin> <hzlex-bin> <corpus-root>', file=sys.stderr)
        return 2
    huzc, hzlex, corpus = argv[1], argv[2], argv[3]
    if not (os.path.isfile(huzc) and os.access(huzc, os.X_OK)):
        print('FAIL: huzc binary not found/executable: %s' % huzc, file=sys.stderr)
        return 2
    if not (os.path.isfile(hzlex) and os.access(hzlex, os.X_OK)):
        print('FAIL: hzlex binary not found/executable: %s' % hzlex, file=sys.stderr)
        return 2
    if not os.path.isdir(corpus):
        print('FAIL: corpus root not a directory: %s' % corpus, file=sys.stderr)
        return 2

    files = []
    for dp, _, fns in os.walk(corpus):
        for f in sorted(fns):
            if f.endswith('.hz'):
                files.append(os.path.join(dp, f))

    passed = failed = 0
    for p in files:
        pa = subprocess.run([huzc, '--dump-tokens', '-i', p], capture_output=True)
        if pa.returncode != 0:
            print('FAIL(dump-huzc): %s' % p)
            failed += 1
            continue
        pb = subprocess.run([hzlex, p], capture_output=True)
        if pb.returncode != 0:
            print('FAIL(dump-hzlex): %s' % p)
            failed += 1
            continue
        la = pa.stdout.split(b'\n')
        if la and la[-1] == b'':
            la.pop()
        lb = pb.stdout.split(b'\r\n')
        if lb and lb[-1] == b'':
            lb.pop()
        if la == lb:
            print('PASS: %s' % p)
            passed += 1
        else:
            detail = ''
            for i, (x, y) in enumerate(zip(la, lb)):
                if x != y:
                    detail = 'line%d rust=%r hzlex=%r' % (i + 1, x[:70], y[:70])
                    break
            else:
                detail = 'len %d vs %d' % (len(la), len(lb))
            print('FAIL(diff): %s %s' % (p, detail))
            failed += 1

    print('-----------------------------')
    print('%d passed, %d failed (total %d)' % (passed, failed, passed + failed))
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
