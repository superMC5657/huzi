# Huzi 语言与标准库参考手册 (Reference Manual)

本文档为 Huzi 编程语言的规范与内置标准库函数速查参考。

快速入门与编译器选项请参阅 [`USAGE.md`](USAGE.md)；循序渐进的语言教程请参阅 [`tutorial.md`](tutorial.md)。

---

## 1. 关键字与语法规范

| 关键字 | 说明 |
|--------|------|
| `fn` | 函数定义 |
| `let` | 变量声明 |
| `mut` | 可变变量修饰符 |
| `if` / `elif` / `else` | 条件分支控制 |
| `for` / `in` | 范围或集合循环遍历 |
| `while` | 条件循环 |
| `break` / `continue` | 循环控制跳转 |
| `return` | 函数退出并返回值 |
| `defer` | 函数退出前延迟执行（LIFO） |
| `struct` | 命名结构体声明 |
| `enum` | 枚举（代数数据类型）声明 |
| `match` | 模式匹配表达式 |
| `trait` / `impl` | 接口抽象与静态分发实现 |
| `import` | 模块导入 |
| `box` / `null` | 堆指针构造与空值 |

---

## 2. 类型系统与内存语义

| 类型 | 说明 | LLVM IR 表达 | 内存语义 |
|------|------|-------------|---------|
| `i32` / `i64` | 32/64 位有符号整数 | `i32` / `i64` | 值传递 |
| `f32` / `f64` | 32/64 位单/双精度浮点数 | `float` / `double` | 值传递 |
| `bool` | 布尔类型 (`true` / `false`) | `i1` | 值传递 |
| `char` | 8 位字符 | `i8` | 值传递 |
| `str` | 字符串指针（`\0` 结尾） | `ptr` | 指针传递，不可直接原地修改 |
| `[T; N]` | 固定长度数组 | `[N x T]` | 栈分配，按指针寻址传递 |
| `(T1, T2, ...)` | 匿名元组 | `{ T1, T2, ... }` | 栈分配紧凑连续布局 |
| `struct Name` | 命名结构体 | `%struct.Name` | 栈上连续布局，赋值与传参逐字段拷贝 |
| `enum Name` | 枚举变体与 payload | `%enum.Name = { i32, [M x i8] }` | 判别码与 payload 联合存储区 |
| `vec<T>` | 动态数组 | `{ ptr, i32, i32 }` | 栈上元数据（指针、长度、容量），支持参数与字段 |
| `Map` / `HashMap` | 键值映射表 | `{ ptr, i32, i32 }` | 堆上哈希槽与元数据，支持参数与字段 |
| `Box<T>` | 堆指针泛型 | `ptr` | 堆分配，点号自动解引用，引用计数 (RC) 追踪 |

---

## 3. 运算符与优先级

| 类别 | 运算符 | 语义说明 |
|------|--------|---------|
| 算术运算符 | `+`, `-`, `*`, `/`, `%` | 支持整数与浮点数；整数除以零触发 panic |
| 比较运算符 | `==`, `!=`, `<`, `<=`, `>`, `>=` | 数值、布尔、枚举、指针比较；字符串按字典序对比 |
| 逻辑运算符 | `&&`, `\|\|`, `!` | 布尔逻辑与、或、非 |
| 位运算符 | `&`, `\|`, `^`, `<<`, `>>` | 整数按位运算与移位 |

---

## 4. 标准库内置函数全览

### 4.1 输入输出 (I/O)
- `print(...)`: 打印任意基本类型、`vec`、结构体与 `Box`（自动换行）。
- `read_line() -> str`: 从标准输入读取一行（包含管道重定向）。
- `read_int() -> i32`: 读取整数。
- `read_float() -> f64`: 读取浮点数。
- `is_eof() -> bool`: 标准输入是否读取到 EOF。

### 4.2 命令行参数 (CLI Args)
- `arg_count() -> i32`: 获取参数总数（含程序名）。
- `arg(i: i32) -> str`: 获取第 i 个参数（下标越界返回空串）。
- `arg_ok(i: i32) -> bool`: 检查下标是否有效 (`0 <= i < arg_count()`)。

### 4.3 字符串操作
- `len(s: str) -> i32`: 字符串字节长度。
- `concat(a: str, b: str) -> str`: 字符串拼接。
- `to_string(x) -> str`: 整数或浮点数转换为字符串。
- `split(s: str, d: str) -> vec<str>`: 按分隔符切分字符串。
- `substring(s: str, start: i32, end: i32) -> str`: 取 `[start, end)` 字节子串。
- `trim(s: str) -> str`: 移除首尾 ASCII 空白字符。
- `contains(s: str, sub: str) -> bool`: 是否包含子串。
- `parse_int(s: str) -> (bool, i32)`: 尝试解析整数，失败返回 `(false, 0)`。
- `parse_float(s: str) -> (bool, f64)`: 尝试解析浮点数，失败返回 `(false, 0.0)`。

### 4.4 数学函数 (`math::*`)
- `abs(x)`, `sqrt(x)`, `pow(x, y)`, `sin(x)`, `cos(x)`, `tan(x)`, `floor(x)`, `ceil(x)`, `round(x)`。

### 4.5 系统与进程
- `time() -> i64`: 当前 Unix 时间戳（秒）。
- `localtime(ts: i64) -> str`: 将时间戳格式化为 `YYYY-MM-DD hh:mm:ss` 本地时间字符串。
- `env_get(key: str) -> (bool, str)`: 读取环境变量，不存在返回 `(false, "")`。
- `rand() -> i32` / `srand(seed: i64)`: 伪随机数发生器。
- `sleep_ms(ms: i32)`: 毫秒级进程睡眠。
- `exit(code: i32)`: 立即退出进程并返回退出码。
- `panic(msg: str)`: 输出运行时错误信息并以退出码 1 终止。

### 4.6 文件 I/O
- `read_file(path: str) -> str`: 读入全部文件文本（≤2GB）。
- `read_file_ok(path: str) -> bool`: 文件是否存在且可读。
- `read_file_err(path: str) -> str`: 读取失败原因诊断文本。
- `write_file(path: str, content: str) -> bool`: 覆盖写入文件内容。

### 4.7 哈希表 (HashMap)
- `map_new() -> Map`: 创建哈希表。
- `map_put(m: Map, k: str, v: i32)`: 插入或更新键值对。
- `map_get(m: Map, k: str) -> i32`: 获取键对应的值（不存在返回 0）。
- `map_has(m: Map, k: str) -> bool`: 检查键是否存在。
- `map_remove(m: Map, k: str) -> bool`: 删除指定键。
- `map_len(m: Map) -> i32`: 当前键值对数量。

### 4.8 网络通信 (TCP)
- `tcp_connect(host: str, port: i32) -> i32`: 连接服务器，返回套接字描述符（失败返回 `-1`）。
- `tcp_listen(port: i32) -> i32`: 开启 TCP 监听。
- `tcp_accept(listener: i32) -> i32`: 接受传入连接。
- `tcp_send(sock: i32, data: str) -> i32`: 发送字符串。
- `tcp_recv(sock: i32, max_len: i32) -> str`: 接收最多指定字节数。
- `tcp_close(sock: i32)`: 关闭套接字。

### 4.9 多线程并发 (Threading)
- `spawn(func, arg: i32) -> i64`: 创建子线程并传入参数，返回线程句柄。
- `join(handle: i64) -> i32`: 等待子线程结束并获取返回值。

### 4.10 内存管理与引用计数
- `free_str(s: str)`, `free_vec(v: vec<T>)`, `free_box(b: Box<T>)`: 手动释放堆内存。
- `ref_count(b: Box<T>) -> i32`: 检查当前 `Box` 的引用计数。

---

## 5. 运行时错误列表

| 错误信息 | 触发条件 |
|---------|---------|
| `division by zero` | 整数除以 0 或对 0 取模 |
| `array index out of bounds` | 数组访问越界 |
| `vec index out of bounds` | 动态数组下标越界 |
| `vec pop from empty vec` | 空 `vec` 执行 `pop` 弹出 |
| `string index out of bounds` | 字符串字符下标越界 |
| `substring out of bounds` | 子串切片区间越界或倒置 |

---

## 6. 代码格式化规范与归一化范围说明

- **工具命令**：`huzc fmt [files/dirs]` 与 `huzc fmt --check`。
- **排版归一化**：统一采用 4 空格缩进；规范二元运算符与逗号两侧空格；括号与大括号格式归一化；保持严格幂等性（第二次运行零 diff）。
- **注释保留范围说明**：当前版本 `huzc fmt` 为基于 AST 语法树的代码美化器（AST Pretty Printer）。由于当前前端 AST 节点未保留源码行级注释，格式化过程会自动剥除注释。若源码高度依赖保留特定位置的行级注释，请暂缓使用格式化工具或手动排版。后续版本规划基于完整 CST / Token Span 重构以实现无损注释保留。
