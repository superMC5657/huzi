#!/usr/bin/env python3
# dump_diff.py —— hzlex 与 huzc --dump-tokens 的文本对拍(只做文本比较,不解析语义)。
#
# 用法:
#   python examples/hzlex/dump_diff.py <huzc-bin> <hzlex-bin> <corpus-root>
# 例如:
#   python examples/hzlex/dump_diff.py ./huzc/target/debug/huzc ./examples/hzlex/hzlex huzc/test/cases
#   python examples/hzlex/dump_diff.py ./huzc/target/debug/huzc ./examples/hzlex/hzlex huzi-src
#
# 比较方法(冻结):两侧统一按 b'\n' 切分(尾空行 pop 保留),二进制安全,不 strip 任何字节。
#   判据:切后除末空外每一行都以 b'\r' 结尾即判 Windows-CRLF 模式,每行去一个尾部 b'\r'
#   (内容 \r + \r\n 行尾则留一个 \r,内容无损);否则判 LF/混合模式,原样保留(含内容 \r)。
#   此判据替代旧的 b'\r\n' in stdout 检测:后者在 dump 内容含 \r(如 csv/framing/http 协议 \r\n
#   与 stringx_test 202 行 line1\r)+LF 行尾时误触发,把整文件按 \r\n 切成单行致幽灵 FAIL。
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


def split_lines(raw):
    parts = raw.split(b'\n')
    if parts and parts[-1] == b'':
        parts.pop()
    if parts and all(p.endswith(b'\r') for p in parts):
        parts = [p[:-1] for p in parts]
    return parts


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
        la = split_lines(pa.stdout)
        lb = split_lines(pb.stdout)
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
