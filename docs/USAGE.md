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
cargo run --release --bin huzc -- --input test/examples/01_variables_ops.hz -o 01_variables_ops

# 示例 - 编译到子目录
cargo run --release --bin huzc -- --input test/examples/01_variables_ops.hz -o test/out/01_variables_ops
```

### 3. 运行程序

```bash
# Windows
./01_variables_ops.exe

# Linux/macOS
./01_variables_ops
```

## 编译器选项

| 选项 | 说明 | 示例 |
|------|------|------|
| `--input <file>` | 输入的 .hz 源文件 | `--input test/examples/01_variables_ops.hz` |
| `-o <name>` | 输出文件基础名，自动添加平台扩展名；缺省使用输入文件名（去除后缀） | `-o demo` → `demo.exe` (Windows) |
| `--release` (`-r`) | Release 模式：生成代码前先用 `opt -O2` 优化 LLVM IR，运行速度显著更快；编译过程不打印任何日志（错误仍输出到 stderr）。默认 dev 模式不做 IR 优化并打印编译进度 | `huzc --input main.hz -o main --release` |
| `--opt-level <0-3>` | LLVM 优化级别,覆盖 `--release` 的默认级别 2;`--opt-level 0` 等价于 dev 模式 | `huzc --input main.hz -o main --opt-level 3` |
| `--debug` (`-g`) | 调试模式：在可执行文件中嵌入 DWARF 调试信息(编译单元、行号表、变量),可用 GDB/LLDB 断点单步;隐含 `--opt-level 0`(优化会打乱行号对应),链接器自动加调试参数 | `huzc --input main.hz -o main -g` |

### 输出文件说明

- **Windows**: 输出 `hello.exe`，中间文件 `hello.ll`、`hello.obj`
- **Linux/macOS**: 输出 `hello`，中间文件 `hello.ll`、`hello.o`
- 中间文件在编译完成后自动清理

## 代码格式化 (huzc fmt)

Huzc 内置源码格式化工具，基于抽象语法树 (AST) pretty-print 实现幂等格式化：

```bash
# 格式化单个文件
huzc fmt path/to/file.hz

# 递归格式化整个目录下的所有 .hz 文件
huzc fmt test/examples

# 仅检查是否已符合格式（不修改文件，有未格式化文件时退出码为 1）
huzc fmt --check test/examples
```

- **缩进规范**：统一使用 4 个空格缩进。
- **幂等性保障**：格式化后的代码再次格式化保持零 diff。
- **注释说明**：首版格式化基于 AST 重构输出，注释会被归一化，保持源码语义与控制流完全一致。

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

# 无返回值：可省略 `->` 类型与 `return`，调用时作语句使用
fn greet(name: str) {
    print("Hello,", name)
}

fn main() -> i32 {
    greet("huzi")
    return 0
}
```

无返回值函数的调用值不可赋给变量（`let x = greet()` 编译报错）；
`fn main` 仍须显式声明 `-> i32`。

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

### 3.1 延迟执行 (defer)

`defer <stmt>` 用于在当前函数退出（无论是通过显式 `return` 还是正常执行落出）前执行语句，常用于资源清理与善后：

- **LIFO 逆序执行**：同一个函数内注册的多条 `defer` 语句，按照“后注册、先执行”（LIFO 栈）的顺序逆序调用。
- **作用域限制**：`defer` 仅允许在函数体内使用，顶层使用将在编译期报错。循环体内部声明的 `defer` 同样统一延迟至包含它的整个函数退出时执行。
- **不捕获返回值**：`defer` 在计算完 `return` 表达式后执行，无法篡改已求值的返回值。
- **与 exit/panic 的交互**：调用内置函数 `exit()` 或因越界/除零发生 `panic` 运行时错误时，进程直接终止退出，**不会**触发 `defer` 栈的执行。

```python
fn process_file(path: str) -> i32 {
    let content = read_file(path)
    defer print("process_file cleanup")
    if len(content) == 0 {
        return 0
    }
    # 处理文件内容...
    return 1
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
print(v)                 # vec: [1, 2, 3]
print(p)                 # 结构体: Point { x: 3, y: 4 }
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

结构体之间不可形成直接的值循环嵌套；如需构建自引用或递归数据结构（如链表、树），需通过 `Box<T>` 间接引用（详见「Box 与自引用结构体」）。

`print` 支持直接打印结构体，按 `Point { x: 3, y: 4 }` 递归展开字段输出；含 `Box` 字段的结构体同样整体打印，`Box` 字段判空后递归展开，空指针打印为 `null`。

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

# match 表达式支持模式匹配并产出值
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
print(r == Shape::Rect(3.0, 4.0))  # true
```

枚举支持 `==` 和 `!=` 运算符：先比对变体判别码，再逐一比对 payload 字段（字符串按内容比对，结构体递归比对）。比较两个不同枚举类型会在编译期报错。

match 表达式要求分支穷尽，必须覆盖全部变体或包含 `_` 兜底分支；`print` 打印无 payload 的枚举变体时输出其判别码整数。

### 7. Box 与自引用结构体

```python
# 自引用结构体：通过 Box<T> 构建
struct Node {
    val: i32,
    next: Box<Node>,
}

fn main() -> i32 {
    # box(Node { ... }) 在堆上分配并返回 Box<Node>；null 表示空指针
    let mut head: Box<Node> = box(Node { val: 1, next: null })
    head.next = box(Node { val: 2, next: null })

    # Box 字段访问自动解引用
    print(head.val, head.next.val)   # 1 2

    # Box 整体打印：递归展开字段，空指针打印为 null
    print(head)   # Node {val: 1, next: Node {val: 2, next: null}}

    # 支持判空与指针比较
    if head.next == null {
        print("empty")
    }
    return 0
}
```

规则：

- **类型**：`Box<T>` 为堆分配泛型指针，`T` 须为具名结构体；在 LLVM 层面表示为指针类型。
- **嵌套**：支持 `Box<Box<Node>>`（任意层数，每层仍是指针，堆单元逐层持有下一层指针，最内层持有结构体值）；`box(box(Node { ... }))` 逐层分配，`null` 可赋给任意层数的 `Box` 槽。
- **构造**：`box(expr)` 在堆上分配内存并存入求值结果，返回 `Box<T>`。
- **null**：表示空指针，适用于变量声明、字段赋值、参数传递与返回值。
- **赋值**：`Box` 变量及字段赋值受 `let mut` 约束；`box(..)` 与槽类型的层数 + 最内层结构名须一致，否则报 `Box type mismatch`（如 `box(box(..))` 不能进 `Box<Node>` 槽）。
- **比较**：支持与 `null` 进行判空（`x == null` / `x != null`）以及同类型 `Box` 之间的指针比较（嵌套同样是指针比较）。
- **字段访问**：通过点号访问 `Box` 字段时自动解引用（如 `head.next.val`）；嵌套逐层解引用直达最内层（如 `outer.val` 穿透 `Box<Box<Node>>`）。
- **打印**：`Box` 支持整体打印（`print(b)` 递归展开字段，空指针打印为 `null`）；嵌套逐层判空后打印最内层结构体；逐字段打印（如 `print(b.val)`）仍然可用。
- **含 str 字段的结构体**：可整体装箱（`Box<Msg>`，`Msg { tag: str, val: i32 }`），打印与字段访问行为一致。
- **内存管理**：无 GC；手动释放(`free_box`，详见「内存管理」)，未 free 的内存在进程退出时由 OS 统一回收。

边界（暂不支持，编译期明确报错）：

- `Box<i32>` / `Box<str>` 等非结构体直接包装：报 `Box<T> requires a named struct type`。
- 含 `vec` 字段的结构体：`vec` 只有局部变量形态（无字段类型语法，写 `v: vec` 报 `Unsupported type: vec`），因此这类结构体无法定义，更无法装箱；`str`/数组/元组字段不受影响。
- 嵌套中间层不可具名取出：没有显式解引用语法，`Box<Box<Node>>` 只能整体判空/打印/直达最内层字段，无法单独命名中间的 `Box<Node>` 值。

完整示例见 `test/examples/32_box_linked_list.hz`（单层）与 `test/examples/37_box_nest.hz`（嵌套 + 含 `str` 字段装箱）；层数错配的负例见 `test/neg/box_nest_mismatch.compile_fail.hz`。

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
    # 构造：非空由首元素推导，空 vec 使用 vec<T>() 明确类型
    let mut v = vec(1, 2, 3)
    let mut e = vec<i32>()

    # 下标读写（越界触发运行时错误）
    print(v[0])
    v[1] = 20

    # 追加元素
    push(v, 4)
    push(e, 1)
    print("len =", len(v))

    # 增删元素(均需 let mut)
    let x = pop(v)      # 弹出末尾并返回(空 vec 报运行时错误)
    remove(v, 0)        # 删除下标处元素并左移填补(越界报运行时错误)
    insert(v, 0, 99)    # 在下标处右移腾位并写入(允许 idx == len,等价尾插)
    clear(v)            # 长度置 0(保留容量,后续 push 复用)

    # 整体打印输出 [1, 20, 3, 4]
    print(v)

    # for-in 遍历
    for x in v {
        print(x)
    }
    return 0
}
```

约束：非空 `vec(...)` 元素类型由首元素推导；空向量须使用 `vec<T>()` 明确类型。`print(v)` 输出 `[e1, e2, ...]` 格式。`for x in v` 在进入循环时确定遍历长度。`remove`/`insert` 越界与空 `pop` 触发运行时错误；`insert` 满时自动翻倍扩容；`clear` 后 `len(v) == 0` 且可继续 `push`。

### HashMap（str → i32 特化）

```python
fn main() -> i32 {
    let mut m = map_new()
    map_put(m, "apple", 10)        # 插入(需 let mut)；同键覆盖
    map_put(m, "banana", 20)
    print(map_len(m))              # 2

    let r = map_get(m, "apple")    # 返回元组 (found: bool, val: i32)
    print(r)                       # (true, 10)
    print(r.0)                     # true
    print(r.1)                     # 10

    print(map_has(m, "banana"))    # true
    print(map_remove(m, "banana")) # true(命中删除；缺键返回 false)
    print(map_len(m))              # 1
    return 0
}
```

约束：仅 `str → i32` 特化，无泛型/方法/impl/trait；键须为 `str`、值须为 `i32`，其它类型在编译期报错。开放寻址线性探测，哈希为自实现 FNV-1a，键比较按内容（`strcmp`）；负载超过 3/4 时翻倍扩容并重哈希，删除使用墓碑标记不断链。缺键 `map_get` 返回 `(false, 0)`，不 abort；`map_remove` 缺键返回 `false`。`map` 只有局部变量形态（无字段/参数类型语法），不支持整体 `print` 与函数传参；`len(m)` 请改用 `map_len(m)`。完整示例见 `test/examples/38_hashmap.hz`。

### HashMap 最小可用特化（str -> i32）

```python
fn main() -> i32 {
    # 构造：无参数，总是 str->i32
    let mut m = map_new()

    # 写入（需 let mut；已存在则覆盖）
    map_put(m, "apple", 10)
    map_put(m, "banana", 20)
    print("len = ", map_len(m))   # len = 2

    # 读取：返回 (found: bool, val: i32) 元组；缺键返回 (false, 0)，不 abort
    let r1 = map_get(m, "apple")
    print(r1)           # (true, 10)
    print(r1.0, r1.1)   # true10（print 多参数直接拼接）
    let r2 = map_get(m, "missing")
    print(r2)           # (false, 0)

    # 存在性与删除（删除需 let mut，返回是否命中）
    print(map_has(m, "banana"))     # true
    print(map_remove(m, "banana"))  # true
    print(map_has(m, "banana"))     # false
    print("len = ", map_len(m))     # len = 1
    return 0
}
```

实现：开放寻址线性探测，槽复用 vec 三元组 `{ ptr, len, cap }`，条目为
`{ hash, occupied, key_ptr, key_len, val }`；哈希为自实现 FNV-1a（免 libc 依赖），
键相等借用 `strcmp`；负载超过 0.7 时翻倍扩容并重哈希，删除用墓碑标记保证探测链不断裂。

边界（诚实声明，编译期明确报错）：

- 仅 `str -> i32` 特化：键须为 `str`，值须为 `i32`，无泛型、无方法、无 `impl`/`trait`；
  传其它类型报 `HashMap is specialized to str->i32 only`。
- 仅局部变量形态：无字段/参数类型语法，不可作函数参数、结构体字段，不可 `print`；
  `len(m)` 请改用 `map_len(m)`。
- `map_put`/`map_remove` 需 `let mut` 变量；未释放内存在进程退出时由 OS 统一回收（与 vec 一致）。

完整示例见 `test/examples/38_hashmap.hz`（含循环 put 100 个再全读回的扩容验证）。

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

    # 字符串按字节索引
    let s = "hello"
    print(s[0], s[len(s) - 1])

    return 0
}
```

```python
fn main() -> i32 {
    # split 返回 vec<str>,与下标/len/print/for-in 互通
    let parts = split("foo,bar,baz", ",")
    print(len(parts))     # 3
    print(parts[0])       # foo
    print(parts)          # [foo, bar, baz]
    for p in parts {
        print(p)
    }
    let q = split("a::b::c", "::")
    print(q[1])           # b

    # substring 按字节区间,越界报运行时错误
    print(substring("hello", 1, 4))   # ell

    # trim 去两端空白,contains 返回布尔
    print(trim("   hi  "))            # hi
    print(contains("hello", "ell"))   # true
    return 0
}
```

说明：下标(`s[i]`)、`split`/`contains` 的分隔与匹配、`substring`
区间、`trim` 空白判定均为**字节语义**，UTF-8 多字节字符不按字符切分；
空分隔符 `split(s, "")` 将整体作为唯一一段；空子串 `contains(s, "")` 恒为 true。
完整示例见 `test/examples/33_string_ops2.hz`。

### 内存管理（手动释放，无 GC）

| 函数 | 说明 | 安全态 |
|------|------|--------|
| `free_str(s)` | 释放堆字符串(`concat`/`substring`/`trim`/`to_string`/`read_line`/`read_file` 等返回)，变量指向空串 | `len(s) == 0` |
| `free_vec(v)` | 释放 vec 的 `data` 数组，长度与容量清零(`{ null, 0, 0 }`)，后续 `push` 经 `realloc(null)` 复用 | `len(v) == 0`，可继续 `push` |
| `free_box(b)` | 释放 `Box` 的堆槽，变量置 `null` | `b == null` 为 true |

```python
let mut s = concat("a", "b")
free_str(s)        # len(s) == 0
free_str(s)        # 二次 free 为 no-op，不崩
let mut v = vec(1, 2, 3)
free_vec(v)        # len(v) == 0
push(v, 42)        # free 后可继续 push 复用
```

规则：

- **手动释放、无 GC**：不调用 free 也能正常运行，未 free 的内存在进程退出时由 OS 统一回收。
- **均需 `let mut`**（回写变量槽），返回整数 0（与 `push`/`clear` 一致，仅为兼容表达式位置）。
- **二次 free 为 no-op**：`free_str` 对空串跳过，`free_vec` 对 `free(null)`（libc 语义即 no-op），`free_box` 对 `null` 跳过。
- **浅释放**：`free_vec` 只释放 vec 自身的 `data`，不释放元素内部的堆内存；`free_box` 只释放 Box 自身的槽，不递归释放字段中的堆内存。
- **字面量非堆分配**：`free_str` 仅用于堆字符串，对字符串字面量调用属于未定义行为；空串调用为安全的 no-op。

完整示例见 `test/examples/35_memory_free.hz`。

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
| `[T; N]` | 固定长度数组 | `let arr: [i32; 5] = [1, 2, 3, 4, 5]` |
| `(T1, T2, ...)` | 元组 | `let t = (1, "hello", true)` |
| `vec` | 动态数组 | `let mut v = vec(1, 2, 3)` / `let mut e = vec<i32>()` |
| `HashMap`(str→i32 特化) | 哈希表(开放寻址) | `let mut m = map_new()` + `map_put`/`map_get`/`map_has`/`map_remove`/`map_len` |
| `struct` | 结构体 | `struct Point { x: i32, y: i32 }` |
| `enum` | 枚举 | `enum Color { Red, Green }` / `enum Shape { Circle(f64) }` |
| `Box<T>` | 堆指针结构体 | `let mut h: Box<Node> = box(Node { val: 1, next: null })` |

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
| `arg_ok(i)` | 下标是否有效(`0 <= i < arg_count()`);先查再取,不用猜空串 | `arg_ok(99)` → false |

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

- `arg(0)` 是程序自身路径；`arg(1)` 起是用户参数。
- `arg(i)` 返回的字符串直接指向 argv 存储，不可改写其内容。
- 命令行参数统一按 UTF-8 处理（Windows 与 Unix 平台行为一致）。

### 管道输入
`read_line()`/`read_int()`/`read_float()` 均可读管道或重定向的 stdin，配合
`is_eof()` 判断输入结束：

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

惯用法是**先读取再检查 `is_eof()`**：读到末尾时 `read_line()` 返回空串且
`is_eof()` 变为 true，此时停止处理即可。参考 `test/examples/23_cli_args.hz`
与 `test/examples/24_pipe_read.hz`。

### 字符串
| 函数 | 说明 | 示例 |
|------|------|------|
| `len(s)` | 获取字符串长度 | `len("hello")` → 5 |
| `concat(a, b)` | 字符串拼接 | `concat("a", "b")` → "ab" |
| `to_string(x)` | 数值转字符串 | `to_string(42)` → "42" |
| `split(s, d)` | 按分隔符切分,返回 vec\<str\> | `split("a,b", ",")` → ["a", "b"] |
| `substring(s, l, r)` | 字节区间 `[l, r)` 拷贝 | `substring("hello", 1, 4)` → "ell" |
| `trim(s)` | 去两端 ASCII 空白 | `trim("  hi  ")` → "hi" |
| `contains(s, sub)` | 是否包含子串 | `contains("hello", "ell")` → true |

### 随机数 / 时间 / 进程
| 函数 | 说明 | 示例 |
|------|------|------|
| `srand(seed)` | 设置伪随机数序列起点 | `srand(42)` |
| `rand()` | 伪随机数，`0..=RAND_MAX`（Windows 为 32767）；同一 seed 序列可复现 | `rand() % 100` |
| `time()` | 当前 Unix 时间戳（秒，i64） | `let t = time()` |
| `exit(n)` | 立即终止进程，退出码 n | `exit(1)` |
| `panic(msg)` | 打印 `Runtime error: <msg>` 并以退出码 1 终止 | `panic("boom")` |
| `sleep_ms(ms)` | 毫秒级睡眠（负值按 0 处理） | `sleep_ms(100)` |

```huzi
fn main() -> i32 {
    srand(time())      # 用时钟播种，每次运行序列不同
    print(rand() % 6 + 1)   # 掷骰子
    0
}
```

### 文件读写
| 函数 | 说明 | 示例 |
|------|------|------|
| `read_file(path)` | 一次性读入整个文件（≤2GB）；失败返回空串 | `let s = read_file("data.txt")` |
| `read_file_ok(path)` | 文件是否可读；先分支再读,不用猜空串 | `if read_file_ok(p) { read_file(p) }` |
| `read_file_err(path)` | 成功返回空串,失败返回诊断文本 | `read_file_err("no.txt")` → "cannot open file" |
| `write_file(path, content)` | 整体写入（覆盖）；返回是否成功 | `write_file("test/out/demo.txt", s)` |

```huzi
fn main() -> i32 {
    if write_file("test/out/demo.txt", "hello file\n") {
        print(read_file("test/out/demo.txt"))    # hello file
    }
    0
}
```

哨兵消除(不用猜空串)：

```huzi
fn main() -> i32 {
    let p = "test/out/no_such_file.txt"
    if read_file_ok(p) {
        print(read_file(p))
    } else {
        print(read_file_err(p))    # cannot open file
    }
    if arg_ok(1) {
        print(arg(1))
    }
    0
}
```

### 运行时错误
以下错误在运行时立即终止程序（以退出码 1 退出）：

- 主动 abort：`panic("boom")` → `Runtime error: boom`（与下述检查共用同一报错/退出路径）
- 整数除零 / 取模零：`Runtime error: division by zero`（浮点除法遵循 IEEE 754 语义）
- 数组下标越界：`Runtime error: array index out of bounds (length N)`
- vec 下标越界：`Runtime error: vec index out of bounds`
- 空 vec 执行 `pop`：`Runtime error: vec pop from empty vec`
- 字符串下标越界：`Runtime error: string index out of bounds`（按字节下标检查）
- 子串区间越界：`Runtime error: substring out of bounds`（`start > end` 或 `end > len(s)`，含负数）

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
├── crates/
│   ├── huzc/           # 编译器 CLI 入口与链接编排
│   ├── huzi-ast/       # 抽象语法树与符号定义
│   ├── huzi-lexer/     # 词法分析器
│   ├── huzi-parser/    # 语法分析器
│   ├── huzi-codegen/   # LLVM 代码生成与内置函数
│   ├── huzi-error/     # 错误渲染与建议
│   └── huzi-lsp/       # 语言服务器 (LSP)
├── test/
│   ├── examples/       # 示例程序与模块
│   ├── expected/       # 示例标准输出快照
│   ├── neg/            # 编译期与运行时负例测试
│   └── out/            # 测试构建输出目录
└── docs/
    ├── USAGE.md        # 用户使用指南
    └── 开发文档.md      # 技术架构与开发文档
```

## 常见问题

### Q: 编译报错 "Verification failed"
A: 这是 LLVM 模块验证错误，说明生成的 IR 存在非法指令，属于编译器内部错误（bug），编译流程将直接终止并报错。

### Q: 如何查看生成的 LLVM IR
A: 中间文件 `.ll` 在编译完成后会自动清理。如需查看，可临时修改代码保留中间文件。

### Q: 支持递归函数吗
A: 支持。详见示例 `test/examples/05_recursion.hz`。

### Q: 输出文件在哪里
A: 中间文件 (`.ll` 和 `.obj/.o`) 与输出文件在同一目录，编译完成后自动清理。

### Q: 如何指定输出目录
A: 使用 `-o` 指定路径即可：
```bash
huzc --input test/examples/20_huzi_demo.hz -o build/myapp
# 生成 build/myapp.exe (Windows) 或 build/myapp (Linux/macOS)
```

## 项目状态

编译器核心功能、标准内置函数、复合数据类型（数组、元组、结构体、枚举、Vec、Box）、模块系统、调试支持与语言服务（LSP）均已完整实现并通过全量回归测试套件验证。

---

**祝您使用愉快！**
