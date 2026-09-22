// Rust 参考版本：与 test/bench_perf.hz 完全相同的三个计算内核
// 用于对比 huzi 编译产物与原生 Rust 的执行速度
//
// 编译（release 优化）:
//   rustc -O test/bench_perf.rs -o test/out/bench_perf_rs
// 运行:
//   ./test/out/bench_perf_rs

fn int_loop(n: i32) -> i32 {
    let mut sum = 0;
    for i in 0..n {
        sum = sum + i % 7;
    }
    sum
}

fn fib(n: i32) -> i32 {
    if n <= 1 {
        return n;
    }
    fib(n - 1) + fib(n - 2)
}

fn float_loop(n: i32) -> f64 {
    let mut f = 1.0;
    for _ in 0..n {
        f = f * 1.0000001 + 0.0000001;
    }
    f
}

fn main() {
    let n_int: i32 = 10_000_000;
    let n_fib: i32 = 30;
    let n_float: i32 = 5_000_000;

    let r1 = int_loop(n_int);
    let r2 = fib(n_fib);
    let r3 = float_loop(n_float);

    println!("int_loop result:{}", r1);
    println!("fib result:{}", r2);
    println!("float_loop result:{:.6}", r3);
}
