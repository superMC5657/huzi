# Huzi 编程语言 & Huzc 编译器用户指南

## 简介

Huzi 是一种简洁的编译型编程语言，语法类似 Python，编译后生成高效的可执行文件。Huzc 是 Huzi 语言的编译器，使用 Rust 开发，LLVM-18 作为后端。

## 环境要求

- Rust 1.70+
- LLVM 18
- Windows 10/11 (x86_64) / Linux / macOS
- clang 或 lld-link (用于链接)

## 快速开始

### 1. 构建编译器

```bash
cargo build --release
```

### 2. 编译 Huzi 程序

```bash
# 基本用法
cargo run --release --bin huzc -- --input <源文件.hz> -o <输出名称>

# 示例 - 编译到当前目录
cargo run --release --bin huzc -- --input examples/hello.hz -o hello

# 示例 - 编译到子目录
cargo run --release --bin huzc -- --input examples/hello.hz -o out/hello
```

### 3. 运行程序

```bash
# Windows
./hello.exe

# Linux/macOS
./hello
```

## 编译器选项

| 选项 | 说明 | 示例 |
|------|------|------|
| `--input <file>` | 输入的 .hz 源文件 | `--input hello.hz` |
| `-o <name>` | 输出文件基础名 (自动添加平台扩展名) | `-o hello` → `hello.exe` (Windows) |
| `--release` (`-r`) | Release 模式：生成代码前先用 `opt -O2` 优化 LLVM IR，运行速度显著更快；编译过程不打印任何日志（错误仍输出到 stderr）。默认 dev 模式不做 IR 优化并打印编译进度 | `huzc --input main.hz -o main --release` |
| `--opt-level <0-3>` | LLVM 优化级别,覆盖 `--release` 的默认级别 2;`--opt-level 0` 等价于 dev 模式 | `huzc --input main.hz -o main --opt-level 3` |
| `--debug` (`-g`) | 调试模式：在可执行文件中嵌入 DWARF 调试信息(编译单元、行号表、变量),可用 GDB/LLDB 断点单步;隐含 `--opt-level 0`(优化会打乱行号对应),链接器自动加调试参数 | `huzc --input main.hz -o main -g` |

### 输出文件说明

- **Windows**: 输出 `hello.exe`，中间文件 `hello.ll`、`hello.obj`
- **Linux/macOS**: 输出 `hello`，中间文件 `hello.ll`、`hello.o`
- 中间文件在编译完成后自动清理

## 调试

`-g`/`--debug` 生成带 DWARF 调试信息的可执行文件(强制 `-O0`):

```bash
huzc -g -i main.hz -o main
```

之后即可用 GDB/LLDB 调试,断点、单步、打印变量均按 Huzi 源码行号工作:

```bash
gdb ./main
(gdb) break main.hz:10     # 按源码行设置断点
(gdb) run
(gdb) print x              # 打印变量(gdb 会显示 let 声明的名字)
(gdb) next                 # 按语句单步
```

说明:

- **文件模块**:被 `import` 的 `.hz` 文件的函数同样携带调试信息,按各自文件归属。
- **复合类型**:结构体/枚举/元组以结构体形式描述;数组在内部按指针传递,gdb 中显示为地址。
- **Windows + GDB**:GDB 的 COFF 读取器对 lld-link 输出的符号表支持有限,建议配合 `-l mingw` 使用
  (`huzc -g -l mingw -i main.hz -o main`);LLDB 对两种链接方式都兼容。

## 语言语法

### 1. 变量声明

```python
# 不可变变量
let x = 10

# 可变变量
let mut y = 20

# 带类型注解
let z: i32 = 30
```

### 2. 函数定义

```python
# 有返回值
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

# 无返回值
fn greet() -> i32 {
    print("Hello!")
    return 0
}
```

### 3. 控制流

```python
# 条件判断
if x > 10 {
    print("big")
} elif x > 5 {
    print("medium")
} else {
    print("small")
}

# 循环 (范围：start..end)
for i in 0..10 {
    print(i)
}

# 循环 (数组遍历)
for x in arr {
    print(x)
}

# 条件循环
while x > 0 {
    x = x - 1
}
```

### 4. 内置函数

```python
# 打印 - 支持所有基本类型、vec 与结构体
print("Hello")           # 字符串
print(42)                # 整数
print(3.14)              # 浮点数
print(true)              # 布尔值
print("x =", x)          # 多参数
print(v)                 # vec:[1, 2, 3](空 vec 为 [])
print(p)                 # 结构体:Point {x: 3, y: 4}(嵌套递归)
```

### 5. 结构体

```python
# 定义结构体
struct Point {
    x: i32,
    y: i32,
}

# 实例化（必须提供全部字段）
let p = Point { x: 3, y: 4 }

# 字段访问
print(p.x, p.y)

# 字段赋值（变量需要 let mut 声明）
let mut q = Point { x: 0, y: 0 }
q.x = 10

# 嵌套字段访问
struct Rect {
    origin: Point,
    w: i32,
    h: i32,
}
let rect = Rect { origin: p, w: 100, h: 50 }
print(rect.origin.x)

# 结构体作为函数参数/返回值（按值传递，赋值时逐字段拷贝）
fn sum_points(a: Point, b: Point) -> i32 {
    return a.x + b.x + a.y + b.y
}
fn make_point(v: i32) -> Point {
    return Point { x: v, y: v * 2 }
}

# 数组中的结构体、结构体的数组字段
let points = [p, q]
points[0].x = 99      # points 需要 let mut
struct Data {
    nums: [i32; 3],
    total: i32,
}
let data = Data { nums: [1, 2, 3], total: 6 }
print(data.nums[2], len(data.nums))
```

限制：结构体不支持自引用/相互嵌套的值循环（`struct A { b: B }` + `struct B { a: A }` 会报编译错误；环边经过 `Box` 的自引用是合法的，见下节「Box 与自引用结构体」）；`print` 支持整个结构体，按 `Point {x: 3, y: 4}` 格式输出（字段编译期展开，嵌套结构体/数组字段递归打印；含 `Box` 字段的结构体整体打印会报错，请逐字段打印）。

### 6. 枚举与 match

```python
# 简单枚举（值为判别码，可 == 比较，print 打印为整数）
enum Color {
    Red,
    Green,
    Blue,
}

# 带数据的枚举（每个变体可带零个、一个或多个 payload）
enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Label(str, i32),
    Point2D,
}

# 构造变体：Enum::Variant 或 Enum::Variant(v1, v2, ...)
let c = Color::Green
let s = Shape::Circle(2.0)
let r = Shape::Rect(3.0, 4.0)

# match 作为表达式，每个分支产出值（全覆盖时可省略 `_`）
fn area(s: Shape) -> f64 {
    return match s {
        Shape::Circle(r) => 3.14159 * r * r,   # r 绑定 payload
        Shape::Rect(w, h) => w * h,            # 多字段按序绑定多个变量
        Shape::Label(name, n) => n * 1.0,
        Shape::Point2D => 0.0,
    }
}

print(area(s))                # 12.56636
print(c == Color::Red)        # false
print(r == Shape::Rect(3.0, 4.0))  # true（带数据枚举支持 ==/!=）
```

带数据枚举的 `==` 先比判别码再逐字段比：整数/浮点/bool/char 按值比，`str` 按内容比（`strcmp`），嵌套结构体按字段递归比；变体不同则直接不等，`!=` 取反。比较两个不同枚举类型是编译错误。

限制：match 做穷尽性检查（覆盖全变体即可省略 `_`，缺变体且无 `_` 时编译报错并列出缺失变体名；`_` 兜底仍兼容）；`print` 简单枚举输出的是判别码整数。

### 7. Box 与自引用结构体

```python
# 自引用结构体:环边经过 Box<T> 即合法(直接值循环仍报编译错误)
struct Node {
    val: i32,
    next: Box<Node>,
}

fn main() -> i32 {
    # box(Node { ... }) 在堆上分配并返回 Box<Node>;null 表空位
    let mut head: Box<Node> = box(Node { val: 1, next: null })
    head.next = box(Node { val: 2, next: null })

    # Box 字段读自动解引用(head.next.val 逐层解)
    print(head.val, head.next.val)   # 12

    # ==/!= 支持 Box vs null(判空)与 Box vs Box(比指针)
    if head.next == null {
        print("empty")
    }
    return 0
}
```

规则：

- **类型**：`Box<T>` 是全语言唯一的尖括号泛型，`T` 须为具名结构体；嵌套 `Box<Box<..>>` 暂不支持（编译报错）；`Box<T>` 在 LLVM 层面降为指针，含 Box 字段的结构体定长。
- **构造**：`box(expr)` 先求值再 `malloc` 存入，`expr` 须为 `T` 的值（如 `box(Node { val: 1, next: null })`）；`box(null)` 无意义，直接写 `null`。
- **null**：只能出现在 `Box` 期望位置（`let` 标注、字段赋值、函数参数、`return`）；裸 `let x = null` 无法推导类型，须写 `let x: Box<Node> = null`；`null` 赋给非 Box（如 `let x: i32 = null`）编译报错；`Box<T>` 与 `T` 之间不隐式转换。
- **赋值**：Box 整体赋值（`head.next = box(..)` / `head.next = null`）受 `let mut` 约束，与现有字段赋值规则一致。
- **条件**：`if`/`while` 条件直接写 `x == null` / `x != null` 表达式即可（复用逻辑运算符）。
- **print**：`print(boxVal)` 暂不支持（会报友好错误），请打印其字段（如 `print(b.val)`）；含 Box 字段的结构体整体打印同样报错。
- **内存管理**：暂无 GC，也不提供 `free`，`box` 分配的内存在程序结束前不释放（泄漏可接受）；如需长期运行的堆管理，请自行设计 arena/复用池。

完整示例见 `examples/32_box_linked_list.hz`。

## 示例程序

### Hello World

```python
fn main() -> i32 {
    print("Hello, World!")
    return 0
}
```

### 数组使用

```python
fn main() -> i32 {
    # 数组字面量
    let arr = [1, 2, 3, 4, 5]
    
    # 访问数组元素
    print("First:", arr[0])
    print("Third:", arr[2])
    
    # 数组求和
    let sum = 0
    for i in 0..5 {
        sum = sum + arr[i]
    }
    print("Sum =", sum)
    
    return 0
}
```

### Vec 动态数组使用

```python
fn main() -> i32 {
    # 构造:非空时元素类型由首元素推导,至少 1 个元素;
    # 空 vec 用 vec<T>() 显式指定元素类型(支持 i32/i64/f32/f64/bool/str/char/结构体)
    let mut v = vec(1, 2, 3)
    let mut e = vec<i32>()

    # 下标读写(越界报运行时错误)
    print(v[0])
    v[1] = 20

    # 追加:满时容量自动翻倍(空 vec 首 push 分配初始容量,需 let mut)
    push(v, 4)
    push(e, 1)
    print("len =", len(v))

    # 整体打印:[1, 20, 3, 4](空 vec 为 [];字符串元素无引号,与 print(str) 一致)
    print(v)

    # for-in 遍历
    for x in v {
        print(x)
    }
    return 0
}
```

约束:裸 `vec()` 无元素可推导类型，编译报错并引导写 `vec<T>()`；`print(v)` 按 `[e1, e2, ...]` 输出（运行时按 len 循环，元素复用自身打印逻辑，嵌套结构体可用）；
`for x in v` 进入前一次性读取长度(循环内 push 新增的元素不保证被遍历)。

### 阶乘计算

```python
fn factorial(n: i32) -> i32 {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}

fn main() -> i32 {
    let result = factorial(5)
    print("Factorial(5) =", result)
    return 0
}
```

### 数学函数

```python
fn main() -> i32 {
    # 平方根
    let r = sqrt(16.0)
    print("sqrt(16) =", r)
    
    # 幂运算
    let p = pow(2.0, 10.0)
    print("2^10 =", p)
    
    # 三角函数
    let s = sin(3.14159 / 2)
    print("sin(π/2) =", s)
    
    # 绝对值
    let a = abs(-42)
    print("abs(-42) =", a)
    
    return 0
}
```

### 字符串操作

```python
fn main() -> i32 {
    # 字符串长度
    let s = "hello"
    print("len:", len(s))
    
    # 字符串拼接
    let a = "Hello, "
    let b = "World!"
    let c = concat(a, b)
    print(c)
    
    # 数值转字符串
    let num = 42
    let str = to_string(num)
    print("num as string:", str)

    # 下标:按字节索引,s[i] 越界(含负下标)报运行时错误
    let s = "hello"
    print(s[0], s[len(s) - 1])

    return 0
}
```

### 输入函数

```python
fn main() -> i32 {
    # 读取字符串
    let name = read_line()
    print("Hello,", name)
    
    # 读取整数
    let age = read_int()
    print("Age:", age)
    
    # 读取浮点数
    let height = read_float()
    print("Height:", height)
    
    return 0
}
```

## 支持的类型

| 类型 | 说明 | 示例 |
|------|------|------|
| `i32` | 32 位整数 | `let x: i32 = 42` |
| `i64` | 64 位整数 | `let x: i64 = 42` |
| `f32` | 32 位浮点数 | `let x: f32 = 3.14` |
| `f64` | 64 位浮点数 | `let x: f64 = 3.14` |
| `bool` | 布尔值 | `let x: bool = true` |
| `str` | 字符串 | `let x: str = "hello"` |
| `char` | 字符 | `let x: char = 'a'` |
| `[T; N]` | 数组 | `let arr: [i32; 5] = [1, 2, 3, 4, 5]` |
| `vec(T)` | 动态数组(非空由首元素推导,空 vec 用 `vec<T>()`) | `let mut v = vec(1, 2, 3)` / `let mut e = vec<i32>()` + `push(v, 4)` + `print(v)` → `[1, 2, 3, 4]` |
| `Box<T>` | 堆指针(`T` 为具名结构体,支持自引用,无 GC) | `let mut h: Box<Node> = box(Node { val: 1, next: null })` + `h.next.val` + `h.next == null` |

## 运算符

### 算术运算符
```python
+   # 加法
-   # 减法
*   # 乘法
/   # 除法
%   # 取模
```

### 比较运算符
```python
==  # 等于
!=  # 不等于
<   # 小于
>   # 大于
<=  # 小于等于
>=  # 大于等于
```

字符串使用全部六个比较运算符(按字典序,基于 `strcmp`):

```python
if s == "quit" {
    print("bye!")
}
```

数组不能直接比较(编译报错),请逐元素比较。

### 逻辑运算符
```python
&&  # 逻辑与
||  # 逻辑或
!   # 逻辑非
```

## 标准库函数

### 输入输出
| 函数 | 说明 | 示例 |
|------|------|------|
| `print(...)` | 打印输出 | `print("x =", x)` |
| `read_line()` | 读取一行字符串 | `let s = read_line()` |
| `read_int()` | 读取整数 | `let n = read_int()` |
| `read_float()` | 读取浮点数 | `let f = read_float()` |
| `is_eof()` | stdin 是否已读到末尾 | `if is_eof() { break }` |

### 命令行参数
| 函数 | 说明 | 示例 |
|------|------|------|
| `arg_count()` | 参数个数(含程序名) | `let n = arg_count()` |
| `arg(i)` | 第 i 个参数;越界/负数返回空串 | `arg(1)` → 第一个用户参数 |

```huzi
fn main() -> i32 {
    let mut i = 0
    while i < arg_count() {
        print(arg(i))
        i = i + 1
    }
    0
}
```

- `arg(0)` 是程序自身路径;`arg(1)` 起是用户参数。
- `arg(i)` 返回的字符串直接指向 argv 存储(零拷贝),不要改写其内容。
- Windows 下程序启动时从 Unicode 命令行(`GetCommandLineW`)转码为 UTF-8
  重建 argv,中文等非 ASCII 参数不再乱码;Linux/macOS 直接使用系统 argv。

### 管道输入
`read_line()`/`read_int()`/`read_float()` 均可读管道或重定向的 stdin,配合
`is_eof()` 判断输入结束:

```huzi
# echo "你好" | ./prog.exe
fn main() -> i32 {
    let mut done = false
    while !done {
        let line = read_line()
        if is_eof() {
            done = true
        } else {
            print(concat("<< ", line))
        }
    }
    0
}
```

惯用法是**先读取再检查 `is_eof()`**:读到末尾时 `read_line()` 返回空串且
`is_eof()` 变为 true,此时停止处理即可。参考 `examples/23_cli_args.hz`
与 `examples/24_pipe_read.hz`。

### 字符串
| 函数 | 说明 | 示例 |
|------|------|------|
| `len(s)` | 获取字符串长度 | `len("hello")` → 5 |
| `concat(a, b)` | 字符串拼接 | `concat("a", "b")` → "ab" |
| `to_string(x)` | 数值转字符串 | `to_string(42)` → "42" |

### 随机数 / 时间 / 进程
| 函数 | 说明 | 示例 |
|------|------|------|
| `srand(seed)` | 设置伪随机数序列起点 | `srand(42)` |
| `rand()` | 伪随机数,`0..=RAND_MAX`(Windows 为 32767);同一 seed 序列可复现 | `rand() % 100` |
| `time()` | 当前 Unix 时间戳(秒,i64) | `let t = time()` |
| `exit(n)` | 立即终止进程,退出码 n | `exit(1)` |
| `sleep_ms(ms)` | 毫秒级睡眠(负值按 0) | `sleep_ms(100)` |

```huzi
fn main() -> i32 {
    srand(time())      # 用时钟播种,每次运行序列不同
    print(rand() % 6 + 1)   # 掷骰子
    0
}
```

### 文件读写
| 函数 | 说明 | 示例 |
|------|------|------|
| `read_file(path)` | 一次性读入整个文件(≤2GB);失败返回空串 | `let s = read_file("data.txt")` |
| `write_file(path, content)` | 整体写入(覆盖);返回是否成功 | `write_file("out.txt", s)` |

```huzi
fn main() -> i32 {
    if write_file("out/demo.txt", "hello file\n") {
        print(read_file("out/demo.txt"))    # hello file
    }
    0
}
```

### 运行时错误
以下错误在运行时立即终止程序(打印一行错误后以退出码 1 退出):

- 整数除零 / 取模零:`Runtime error: division by zero`(浮点除法遵循 IEEE 语义,不检查)
- 数组下标越界:`Runtime error: array index out of bounds (length N)`(负下标同样报错)
- vec 下标越界:`Runtime error: vec index out of bounds`(负下标同样报错)
- 字符串下标越界:`Runtime error: string index out of bounds`(负下标同样报错;按字节语义,UTF-8 多字节暂不做字符语义)

### 数学
| 函数 | 说明 | 示例 |
|------|------|------|
| `abs(x)` | 绝对值 | `abs(-5)` → 5.0 |
| `sqrt(x)` | 平方根 | `sqrt(16.0)` → 4.0 |
| `pow(x, n)` | 幂运算 | `pow(2.0, 3.0)` → 8.0 |
| `sin(x)` | 正弦 | `sin(0)` → 0.0 |
| `cos(x)` | 余弦 | `cos(0)` → 1.0 |

## 模块系统

### import 语句

```huzi
import math              # 内置模块
import mods.helpers      # 文件模块:解析为 mods/helpers.hz
```

- **文件模块**:相对导入文件所在目录查找 `<路径>.hz`(其次当前工作目录);点分名对应子路径。
- 模块文件只允许 `fn`/`struct`/`enum` 定义与 `import`,不允许顶层级语句。
- 导入后通过**限定名**使用:`helpers::add(1, 2)`、`math::sqrt(4.0)`;文件模块的
  `struct`/`enum` 注册为全局名,直接按原名使用。
- 同一模块只编译一次(按解析路径去重),循环导入会在首次访问处截断。
- 模块内函数可调用同模块其它函数与内置函数,不能调用主程序里的函数。

### 内置模块

| 模块 | 说明 |
|------|------|
| `math` | 数学函数的限定形式:`math::sqrt(x)`、`math::pow(x, n)`、`math::sin(x)` 等,与内置同名函数等价 |

## 关键字

| 关键字 | 说明 |
|--------|------|
| `fn` | 函数定义 |
| `let` | 变量声明 |
| `mut` | 可变变量 |
| `if` | 条件判断 |
| `elif` | 多条件分支 |
| `else` | 否则分支 |
| `for` | 循环 |
| `while` | 条件循环 |
| `return` | 返回值 |
| `import` | 导入模块 |
| `in` | 循环范围 |

## 编译流程

```
.hz 源文件
    ↓
[1/5] 词法分析 (Lexer)
    ↓
[2/5] 语法分析 (Parser)
    ↓
[3/5] 代码生成 (CodeGen) → LLVM IR
    ↓
[4/5] 验证 (Verify)
    ↓
[可选] IR 优化 (仅 --release 模式: opt -O2)
    ↓
[5/5] 链接 (Linker) → 可执行文件
```

> 说明：`cargo build` 的 debug/release 只影响 huzc 编译器自身的编译速度，
> 不影响它生成的程序。想让生成的程序更快，请使用 `--release` 编译选项。

## 项目结构

```
huzc/
├── examples/           # 示例程序
│   ├── hello.hz
│   ├── array.hz        # 数组示例
│   ├── fact.hz         # 阶乘示例
│   └── ...
├── crates/
│   ├── huzc/           # 编译器入口
│   ├── huzi-lexer/     # 词法分析器
│   ├── huzi-parser/    # 语法分析器
│   ├── huzi-codegen/   # LLVM 代码生成
│   ├── huzi-ast/       # AST 定义
│   └── huzi-error/     # 错误处理
└── docs/
    ├── USAGE.md        # 用户指南
    └── TECHNICAL.md    # 技术文档
```

## 常见问题

### Q: 编译报错 "Verification failed"
A: 这是 LLVM 验证警告，编译器会继续生成可执行文件。如果生成的程序无法运行，请检查代码逻辑。

### Q: 如何查看生成的 LLVM IR
A: 中间文件 `.ll` 在编译完成后会自动清理。如需查看，可临时修改代码保留中间文件。

### Q: 支持递归函数吗
A: 支持。详见示例 `fact.hz`。

### Q: 输出文件在哪里
A: 中间文件 (`.ll` 和 `.obj/.o`) 与输出文件在同一目录，编译完成后自动清理。

### Q: 如何指定输出目录
A: 使用 `-o` 指定路径即可：
```bash
huzc --input src/main.hz -o build/myapp
# 生成 build/myapp.exe (Windows) 或 build/myapp (Linux/macOS)
```

## 后续计划

TODO 全部已完成，暂无待开发功能（P5-16 发布需求已移除）。

---

**祝您使用愉快！**
