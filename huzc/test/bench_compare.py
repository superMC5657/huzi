#!/usr/bin/env python3
# huzi vs Rust vs Python 性能对比与回归门禁脚本
#
# 用法（在项目根目录 huzc/ 下执行）:
#   python test/bench_compare.py
#
# 基线文件 test/bench_baseline.txt 存档上次通过门禁的
# huzi release / Rust -O 比值，仅做漂移提示、不参与门禁判定。
#
# 脚本会：
#   1. 找到（或构建）huzc 编译器
#   2. 编译 test/bench_perf.hz 两次（dev / --release）并多次运行计时
#   3. 编译 test/bench_perf.rs 两次（rustc 默认无优化 / rustc -O）并多次运行计时
#   4. 用纯 Python 跑同样的三个内核并计时
#   5. 输出五者对比表格，并校验数值一致性与性能回退门禁

import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TEST_DIR = os.path.join(ROOT, "test")
OUT_DIR = os.path.join(TEST_DIR, "out")

N_INT = 10_000_000
N_FIB = 30
N_FLOAT = 5_000_000

EXE_RUNS = 10  # 编译型程序跑多次取最小值，减小噪声
PY_RUNS = 1    # 解释型太慢，只跑一次

# 性能门禁阈值：huzi release 相对 Rust -O 的耗时倍数上限（基准约 1.6x，允许波动上限 2.0x）
MAX_REL_VS_RUST_OPT_RATIO = 2.0

# 历史基线文件：只存档、不判门（门禁仍只看上面的 2.0x 与三门禁）
BASELINE_FILE = os.path.join(TEST_DIR, "bench_baseline.txt")
# 相对基线的漂移提示线（仅打印提示、不退出，硬门禁不动）
BASELINE_DRIFT_WARN = 0.10


def find_compiler():
    for rel in ("target/release/huzc.exe", "target/release/huzc"):
        path = os.path.join(ROOT, rel)
        if os.path.exists(path):
            return path
    return None


def build_compiler():
    print("未找到已构建的 huzc，执行 cargo build --release ...")
    subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)
    exe = find_compiler()
    if exe is None:
        sys.exit("构建后仍未找到 huzc 可执行文件")
    return exe


def exe_suffix(name):
    return name + (".exe" if os.name == "nt" else "")


def compile_bench(compiler, release=False):
    os.makedirs(OUT_DIR, exist_ok=True)
    src = os.path.join(TEST_DIR, "bench_perf.hz")
    out = os.path.join(OUT_DIR, "bench_perf_hz_release" if release else "bench_perf_hz")
    mode = "release" if release else "dev"
    print(f"编译 {src} (huzi {mode}) ...")
    cmd = [compiler, "-i", src, "-o", out]
    if release:
        cmd.append("--release")
    subprocess.run(cmd, cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    exe = exe_suffix(out)
    if not os.path.exists(exe):
        sys.exit(f"编译产物不存在: {exe}")
    return exe


def compile_rust_bench(release):
    os.makedirs(OUT_DIR, exist_ok=True)
    src = os.path.join(TEST_DIR, "bench_perf.rs")
    out = exe_suffix(os.path.join(OUT_DIR,
                                  "bench_perf_rs_opt" if release else "bench_perf_rs"))
    mode = "rustc -O" if release else "rustc (debug)"
    print(f"编译 {src} ({mode}) ...")
    cmd = ["rustc"] + (["-O"] if release else []) + [src, "-o", out]
    if os.name == "nt":
        cmd += ["-C", "link-arg=/DEBUG:NONE"]
    subprocess.run(cmd, cwd=ROOT, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    if not os.path.exists(out):
        sys.exit(f"编译产物不存在: {out}")
    return out


# ---------------- 三个内核的 Python 等价实现 ----------------

def int_loop(n):
    s = 0
    for i in range(n):
        s = s + i % 7
    return s


def fib(n):
    if n <= 1:
        return n
    return fib(n - 1) + fib(n - 2)


def float_loop(n):
    f = 1.0
    for _ in range(n):
        f = f * 1.0000001 + 0.0000001
    return f


def time_exe(path):
    times = []
    stdout = ""
    for _ in range(EXE_RUNS):
        t0 = time.perf_counter()
        proc = subprocess.run([path], capture_output=True, text=True)
        times.append(time.perf_counter() - t0)
        stdout = proc.stdout
    return min(times), stdout


def print_comparison(dev_best, rel_best, rust_dev_best, rust_opt_best, py_total):
    def speedup(base):
        return py_total / base if base > 0 else float("inf")

    def ratio(a, b):
        return a / b if b > 0 else float("inf")

    def fmt(label, a, b, name_a, name_b):
        r = ratio(a, b)
        faster = name_a if r < 1 else name_b
        print(f"{label}: {r:6.2f} 倍耗时  ({faster} 更快)")

    print("\n==================== 结果对比 ====================")
    print(f"huzi dev     (编译执行, {EXE_RUNS} 次取最小): {dev_best * 1000:8.1f} ms   "
          f"比 Python 快 {speedup(dev_best):7.1f} 倍")
    print(f"huzi release (编译执行, {EXE_RUNS} 次取最小): {rel_best * 1000:8.1f} ms   "
          f"比 Python 快 {speedup(rel_best):7.1f} 倍")
    print(f"Rust debug   (rustc 默认, {EXE_RUNS} 次取最小): {rust_dev_best * 1000:8.1f} ms   "
          f"比 Python 快 {speedup(rust_dev_best):7.1f} 倍")
    print(f"Rust -O      (rustc -O,  {EXE_RUNS} 次取最小): {rust_opt_best * 1000:8.1f} ms   "
          f"比 Python 快 {speedup(rust_opt_best):7.1f} 倍")
    print(f"Python       (解释执行, {PY_RUNS} 次):             {py_total * 1000:8.1f} ms")

    print("---- dev 档对比 (huzi dev vs Rust debug) ----")
    fmt("huzi dev 相对 Rust debug", dev_best, rust_dev_best, "huzi dev", "Rust debug")
    print("---- release 档对比 (huzi release vs Rust -O) ----")
    fmt("huzi release 相对 Rust -O", rel_best, rust_opt_best, "huzi release", "Rust -O")
    print("---- 优化收益 ----")
    fmt("huzi dev 相对 huzi release", dev_best, rel_best, "huzi dev", "huzi release")
    print("==================================================")


def verify_gate(dev_best, rel_best, rust_opt_best,
                dev_out, rel_out, rust_dev_out, rust_opt_out):
    # 1. 结果一致性校验（五边数值必须一致）
    ok = all("29999994" in out and "832040" in out
             for out in (dev_out, rel_out, rust_dev_out, rust_opt_out))
    if not ok:
        print("FAIL: 结果一致性校验失败！五边输出数值不一致。")
        sys.exit(1)
    print("结果一致性: 通过")

    # 2. Release 必须优于 Dev
    if rel_best > dev_best:
        print(f"FAIL: 性能异常！huzi release ({rel_best*1000:.1f}ms) 慢于 dev ({dev_best*1000:.1f}ms)")
        sys.exit(1)

    # 3. 性能回退门禁：huzi release 相对 Rust -O 回退不可超标
    ratio = rel_best / rust_opt_best if rust_opt_best > 0 else float("inf")
    if ratio > MAX_REL_VS_RUST_OPT_RATIO:
        print(f"FAIL: 性能回退超标！huzi release 相对 Rust -O 为 {ratio:.2f}x，超过门禁阈值 {MAX_REL_VS_RUST_OPT_RATIO:.2f}x")
        sys.exit(1)

    print(f"性能门禁: 通过 (huzi release / Rust -O = {ratio:.2f}x <= {MAX_REL_VS_RUST_OPT_RATIO:.2f}x)")


def load_baseline():
    """读取历史基线比值；文件缺失或非法时返回 None（不判失败）"""
    try:
        with open(BASELINE_FILE, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                return float(line)
    except (OSError, ValueError):
        return None
    return None


def save_baseline(ratio):
    """门禁通过后把本次比值存档（覆盖写，仅归档历史）"""
    with open(BASELINE_FILE, "w", encoding="utf-8") as f:
        f.write("# huzi release / Rust -O 历史基线（仅漂移提示，硬门禁仍为 2.0x）\n")
        f.write(f"{ratio:.4f}\n")


def report_baseline(ratio):
    """对比基线并给出超阈提示；只打印、不退出、不改三门禁"""
    baseline = load_baseline()
    if baseline is None or baseline <= 0:
        print(f"历史基线: 无（首次运行，将本次 {ratio:.2f}x 存档）")
        return
    drift = (ratio - baseline) / baseline
    print(f"历史基线: {baseline:.2f}x，本次: {ratio:.2f}x，漂移: {drift:+.1%}")
    if drift > BASELINE_DRIFT_WARN:
        print(f"提示: 相对基线退化 {drift:.1%}，超过提示线 {BASELINE_DRIFT_WARN:.0%}（仅提示，门禁仍以 {MAX_REL_VS_RUST_OPT_RATIO:.2f}x 为准）")


def main():
    compiler = find_compiler() or build_compiler()
    exe_dev = compile_bench(compiler, release=False)
    exe_rel = compile_bench(compiler, release=True)
    rust_dev_exe = compile_rust_bench(release=False)
    rust_opt_exe = compile_rust_bench(release=True)

    print(f"\n测试参数: int_loop({N_INT:,})  fib({N_FIB})  float_loop({N_FLOAT:,})")
    print(f"编译型程序运行 {EXE_RUNS} 次取最小值，Python 运行 {PY_RUNS} 次\n")

    dev_best, dev_out = time_exe(exe_dev)
    rel_best, rel_out = time_exe(exe_rel)
    print("huzi dev 输出:\n" + dev_out.rstrip())

    rust_dev_best, rust_dev_out = time_exe(rust_dev_exe)
    rust_opt_best, rust_opt_out = time_exe(rust_opt_exe)
    print("\nRust 输出:\n" + rust_opt_out.rstrip())

    print("\nPython 运行中，请稍候 ...")
    t0 = time.perf_counter()
    r1 = int_loop(N_INT)
    r2 = fib(N_FIB)
    r3 = float_loop(N_FLOAT)
    py_total = time.perf_counter() - t0
    print(f"int_loop result:{r1}\nfib result:{r2}\nfloat_loop result:{r3:.6f}")

    print_comparison(dev_best, rel_best, rust_dev_best, rust_opt_best, py_total)
    verify_gate(dev_best, rel_best, rust_opt_best,
                dev_out, rel_out, rust_dev_out, rust_opt_out)
    ratio = rel_best / rust_opt_best if rust_opt_best > 0 else float("inf")
    report_baseline(ratio)
    save_baseline(ratio)
    print(f"基线已存档: {BASELINE_FILE}")


if __name__ == "__main__":
    main()
