# Huzi 语言进阶与开发教程

本文档为 Huzi 语言的核心学习教程，按照由浅入深的顺序介绍语言语法、面向对象抽象、泛型与内存管理模式。

查阅编译器 CLI 工具链选项请参考 [`USAGE.md`](../USAGE.md)；查阅全量标准库 API 与类型规范请参考 [`reference.md`](reference.md)。

---

## 1. 变量与基础类型

Huzi 采用强类型系统，支持局部变量类型自动推导与显式类型标注。变量默认不可变，需使用 `mut` 关键字声明可变性。

```huzi
# 不可变变量（类型自动推导为 i32）
let x = 10

# 可变变量
let mut y = 20
y = y + 5

# 显式类型注解
let z: i64 = 100
let flag: bool = true
let price: f64 = 19.99
let name: str = "Huzi"
```

---

## 2. 函数与控制流

### 函数定义
函数使用 `fn` 关键字定义。有返回值的函数须使用 `-> Type` 标注返回类型并通过 `return` 返回结果；无返回值的过程函数可省略返回类型标注。

```huzi
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn greet(who: str) {
    print("Hello,", who)
}
```

### 条件分支与循环
```huzi
# 条件分支
if x > 10 {
    print("big")
} elif x > 5 {
    print("medium")
} else {
    print("small")
}

# 范围循环 (0 到 9)
for i in 0..10 {
    print(i)
}

# 数组与集合遍历
let nums = [1, 2, 3]
for n in nums {
    print(n)
}

# 条件循环
let mut count = 3
while count > 0 {
    count = count - 1
}
```

### 延迟执行 (defer)
`defer <stmt>` 用于在函数退出（无论正常结束或中途 `return`）前逆序（LIFO）执行善后工作：

```huzi
fn process_resource() -> i32 {
    print("acquire")
    defer print("cleanup 1")
    defer print("cleanup 2")
    return 42
}
# 执行输出顺序：acquire -> cleanup 2 -> cleanup 1
```

---

## 3. 复合数据结构与结构体

### 数组与元组
- 固定长度数组 `[T; N]`：栈上分配，按指针寻址，具备运行时越界保护。
- 匿名元组 `(T1, T2, ...)`：轻量级连续布局，支持 `.0`, `.1` 索引解构。

```huzi
let arr: [i32; 3] = [10, 20, 30]
print(arr[0], len(arr))

let pair = (1, "apple")
print(pair.0, pair.1)
```

### 动态数组 (vec)
`vec` 是堆分配的动态增长数组，支持作为函数形参和结构体字段：

```huzi
let mut v = vec(1, 2, 3)
v.push(4)
print(v[0], len(v)) # 1 4
print(v) # [1, 2, 3, 4]
```

### 结构体 (struct)
结构体按值传递，赋值与传参时逐字段拷贝：

```huzi
struct Point {
    x: i32,
    y: i32,
}

let mut p = Point { x: 3, y: 4 }
p.x = 10
print(p) # Point { x: 10, y: 4 }
```

---

## 4. 枚举与 match 模式匹配

Huzi 的枚举支持无参枚举与带关联值（payload）的代数数据类型，配合穷尽性 `match` 模式匹配：

```huzi
enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Point,
}

fn area(s: Shape) -> f64 {
    return match s {
        Shape::Circle(r) => 3.14159 * r * r,
        Shape::Rect(w, h) => w * h,
        Shape::Point => 0.0,
    }
}
```

---

## 5. 堆指针 Box 与引用计数 (RC)

### 自引用与链表
递归或动态图结构使用堆指针 `Box<T>` 间接包装：

```huzi
struct Node {
    val: i32,
    next: Box<Node>,
}

let mut head: Box<Node> = box(Node { val: 1, next: null })
head.next = box(Node { val: 2, next: null })
print(head.val, head.next.val) # 1 2
```

### 基础类型装箱与解引用
`Box<T>` 的 `T` 也可为基础类型（`i32`/`i64`/`f64`/`bool`/`str`），读写经前缀 `*` 直达最内层（嵌套 `Box<Box<T>>` 逐层独立堆单元，不退化）：

```huzi
let mut b = box(42)
print(*b) # 42
*b = 100
print(*b, b) # 100 100（直接 print(b) 即打印内容）
let mut n: Box<Box<i32>> = box(box(10))
print(*n) # 10
let s: Box<str> = box("hi")
print(*s) # hi
```

空 `Box` 解引用运行时报错退出；`*null` 与对非 Box 值的 `*` 在编译期直接报错。

### 循环引用诊断与打破
Huzi 对 `Box<T>` 采用引用计数 (RC) 内存管理机制，不做精确 GC（无 tracing 收集器）。
当发生相互引用（A → B → A）时，引用计数永不归零，需手动解除闭环。诊断时用
`ref_count` 做快照：环未打破时双方计数均为 2（自身 1 + 对方引用 1），残留即泄漏：

```huzi
struct Node {
    val: i32,
    next: Box<Node>,
}

fn check_leak(n1: Box<Node>, n2: Box<Node>) {
    # 快照泄漏报告：环内双方 rc 均为 2，未打破即告警退出（负例 rc_cycle_leak）
    print("leak snapshot n1 rc: ", ref_count(n1))
    print("leak snapshot n2 rc: ", ref_count(n2))
    if ref_count(n1) > 1 {
        panic("RC leak detected: cycle not broken (n1)")
    }
    if ref_count(n2) > 1 {
        panic("RC leak detected: cycle not broken (n2)")
    }
}

fn test_cycle() {
    let mut n1 = box(Node { val: 100, next: null })
    let mut n2 = box(Node { val: 200, next: null })
    n1.next = n2
    n2.next = n1
    # 官方打破模式：先登记 defer free，再清空一边引用打破环路
    defer free_box(n2)
    n1.next = null # 打破环路，使析构链路正常执行
}
```

要点：`free_box` 为浅释放（只释放 Box 自身槽，不递归释放其字段中的堆内存），
二次 `free` 为 no-op；`defer` 按 LIFO 在函数退出前执行，保证打破后的释放兜底。
完整可运行示例见 `test/cases/40_rc.hz`（环 2-2 → 打破 2-1）。

---

## 6. 泛型与实参推导

Huzi 支持泛型函数与泛型结构体，编译期单态化保证零运行时开销。函数调用点支持实参类型自动推导：

```huzi
fn id<T>(val: T) -> T {
    return val
}

struct Pair<K, V> {
    key: K,
    val: V,
}

fn main() -> i32 {
    let a = id(42) # 自动推导为 id<i32>
    let b = id("hello") # 自动推导为 id<str>
    let p = Pair<i32, str> { key: 1, val: "one" }
    print(a, b, p.val)
    return 0
}
```

---

## 7. Trait 与静态分发

Trait 提供零运行时开销的接口抽象，所有方法调用在编译期脱糖为静态直接函数调用：

```huzi
trait Printable {
    fn show(self) -> str
}

struct User {
    name: str,
}

impl Printable for User {
    fn show(self) -> str {
        return concat("User: ", self.name)
    }
}

fn main() -> i32 {
    let u = User { name: "Alice" }
    print(u.show())
    return 0
}
```

---

## 8. 模块系统与多文件组织

```huzi
# 导入内置标准模块
import math

# 导入子路径文件模块 (解析为 mods/helper.hz)
import mods.helper

fn main() -> i32 {
    let root = math::sqrt(16.0)
    let res = helper::calc(10)
    print(root, res)
    return 0
}
```
