# Huzi 编程语言开发 TODO

## 项目概述

Huzi 是一种简洁的编译型编程语言，语法类似 Python，编译后生成高效的可执行文件。Huzc 是 Huzi 语言的编译器，使用 Rust 开发，LLVM-18 作为后端。

当前项目结构：
```
huzc/
├── crates/
│   ├── huzc/           # 编译器入口
│   ├── huzi-lexer/     # 词法分析器 ✅
│   ├── huzi-parser/    # 语法分析器 ✅
│   ├── huzi-codegen/  # LLVM 代码生成 ✅
│   ├── huzi-ast/      # AST 定义 ✅
│   └── huzi-error/    # 错误处理 ✅
└── examples/          # 示例程序
```

---

## 已实现功能 (2026-03-05 更新)

| 模块 | 状态 | 说明 |
|------|------|------|
| Lexer | ✅ 完成 | 关键字、数字、字符串、字符、运算符、注释 (//) |
| Parser | ✅ 完成 | let/let mut、函数、if-elif-else、for、while、return、数组字面量 |
| Codegen | ✅ 完成 | LLVM IR 生成、类型支持 (i32/i64/f32/f64/bool/str/char)、数组、break/continue、if 表达式、短路逻辑运算 |
| huzc 入口 | ✅ 完成 | 命令行参数、编译 - 链接流程、平台自适应输出 |

---

## 待开发功能

### P0 - 高优先级

#### 1. print 函数完善
- [x] 支持整数打印 `print(42)`
- [x] 支持浮点数打印 `print(3.14)`
- [x] 支持布尔值打印 `print(true)`
- [x] 支持多参数 `print("value:", x)`

#### 2. 类型系统增强
- [x] 变量类型自动推导 (根据右侧表达式)
- [x] 函数参数类型检查
- [x] 变量类型验证 (赋值/返回/调用参数类型不匹配时报错，数值类型自动转换)

---

### P1 - 标准库

#### 3. 基本输入输出
- [x] `read_line()` - 读取一行输入
- [x] `read_int()` - 读取整数
- [x] `read_float()` - 读取浮点数
- [x] `is_eof()` - stdin 是否读到末尾(管道输入结束检测)
- [x] `arg_count()` / `arg(i)` - 命令行参数访问(main 以 `main(argc, argv)` ABI 编译)

#### 4. 字符串操作
- [x] `len(s)` - 获取字符串长度
- [x] `concat(a, b)` - 字符串拼接
- [x] `to_string(x)` - 转换为字符串

#### 5. 数学函数
- [x] `abs(x)` - 绝对值
- [x] `sqrt(x)` - 平方根
- [x] `pow(x, n)` - 幂运算
- [x] `sin(x)`, `cos(x)` - 三角函数
- [x] `tan(x)` - 三角函数
- [x] `floor(x)`, `ceil(x)`, `round(x)` - 取整函数

---

### P2 - 语言特性

#### 6. 复合类型
- [x] 数组 `let arr: [i32; 5] = [1, 2, 3, 4, 5]`
- [x] 数组索引访问 `arr[0]`
- [x] 数组字面量 `let arr = [1, 2, 3]`
- [x] 元组 `(1, "hello", true)`（字面量/类型标注 `(i32, str)`/`.0` 元素访问与赋值/print/函数传参返回/嵌套，见 examples/21_tuples.hz）

#### 7. 结构体
- [x] 结构体定义 `struct Point { x: i32, y: i32 }`
- [x] 结构体实例化 `let p = Point { x: 1, y: 2 }`
- [x] 字段访问 `p.x`
- [x] 字段赋值 `p.x = 5`（受 `let mut` 约束）、嵌套字段 `a.b.x`、函数传参/返回结构体（按值语义）、数组元素字段 `arr[0].x`、结构体数组字段（见 examples/structs.hz）

#### 8. 枚举
- [x] 枚举定义 `enum Color { Red, Green, Blue }`
- [x] 枚举带数据 `enum Result { Ok(i32), Err(str) }`
- [x] match 表达式（变体模式、payload 绑定、`_` 通配符；要求 wildcard 兜底）
- [x] 变体引用 `Color::Red` / 构造 `Result::Ok(42)`（`::` 路径语法，见 examples/enums.hz）

#### 9. 模块系统
- [x] import 语句 (`import math` / `import mods.helpers`,限定调用 `模块::函数`)
- [x] 标准库模块 (内置模块 `math`,数学函数限定形式)
- [x] 文件模块 (相对导入文件目录/工作目录解析,点分名对应子路径,按路径去重、循环导入截断)

---

### P3 - 编译器优化

#### 10. 代码生成优化
- [x] 内联简单函数 (由 `opt` pass 序列承担)
- [x] 常量折叠 (由 `opt` pass 序列承担)
- [x] 公共子表达式消除 (由 `opt` pass 序列承担)
- [x] 简化 main.rs 代码逻辑 (移除 emit-llvm/only-compile)

#### 11. LLVM 优化
- [x] 启用 LLVM 优化 passes (调用 LLVM 自带的 `opt -S -O<n>` 对 IR 做全量 pass 优化)
- [x] 优化级别选项 (`--opt-level 0-3`;`--release` 等价于 `--opt-level 2`,显式指定时优先)

---

### P4 - 开发者体验

#### 12. 错误处理改进
- [x] 解析错误报告真实行号/列号 (Token 携带 span)
- [x] 错误位置高亮 (错误报告带源码行摘录 + `^` 列指示,终端红色高亮、管道输出自动去色)
- [x] 建议修复方案 (Levenshtein 距离 "did you mean";codegen 未定义变量/函数给出最近候选名)

#### 13. 调试支持
- [x] DWARF 调试信息生成 (编译单元/子程序/结构体-枚举-元组类型描述,见 codegen/debuginfo.rs)
- [x] 行号信息 (语句级 DILocation + .debug_line 行号表;AST 语句携带 Span)
- [x] 与 LLDB/GDB 集成 (`-g` 生成 DWARF;gdb 断点/print/单步实测通过,见 docs/USAGE.md「调试」)

#### 14. 测试
- [x] 单元测试 (各 crate 共 33 个:lexer 分词/span、parser 优先级/结构/for-in、codegen 类型映射/verify/运行时检查、error 建议/渲染)
- [x] 集成回归测试 (test.sh 编译并运行全部示例,校验退出码)

---

### P5 - 平台与发布

#### 15. 跨平台支持
- [x] Windows 支持 (x86_64) - ✅ 已完成
- [x] Linux 支持 - ✅ 代码已适配
- [x] macOS 支持 - ✅ 代码已适配
- [x] ARM64 支持 - ✅ 代码已适配(linker 按 ARCH 选 triple;CI 跑 macos-14/aarch64)

#### 16. 发布配置
- [x] Cargo.toml 完善 (resolver = "2")
- [x] 版本号管理
- [x] CI (.github/workflows/ci.yml:linux-x64/windows-x64/macos-arm64,构建零警告+单测+test.sh)

---

## 建议开发顺序

1. **数组支持** → ✅ 已完成
2. **print 函数完善** → ✅ 已完成
3. **标准库函数** → ✅ 已完成 (read_*, len, concat (可变参数), to_string, math 函数)
4. **类型验证** → ✅ 已完成 (基础版)
5. **结构体/枚举** → ✅ 已完成 (结构体 2026-08-29，枚举+match 2026-08-29)
6. **元组** → ✅ 已完成 (2026-08-29)
7. **错误改进/调试** → ✅ 已完成 (2026-08-30:高亮 + 修复建议;2026-08-30:DWARF 调试支持)
8. **模块系统** → ✅ 已完成 (2026-08-30:import/文件模块/内置 math 模块,见 examples/22_modules.hz)

---

## 参考

- 文档：`docs/USAGE.md`
- 技术文档：`docs/TECHNICAL.md`

---

## 2026-09-12 更新:Vec 动态数组(P2 复合类型补充)

- **语法**:`let mut v = vec(1, 2, 3)`(元素类型由首元素推导,至少 1 个元素,不接受类型标注)
- **操作**:`push(v, x)`(满时容量翻倍,需 `let mut`,返回 0)/`v[i]`/`v[i] = x`(动态越界检查)/`len(v)`/`for x in v`(进入前一次性读长度)
- **实现**:纯 codegen 层(`codegen/vec.rs`),表示为栈上匿名结构体 `{ data: ptr, len: i32, cap: i32 }`,槽判定为"结构体 ty + elem 标记";未碰 lexer/parser/AST;prelude 新增 `realloc` 声明
- **约束**:`vec()` 不接受空参数;vec 整体不可 print/to_string;`print(v)` 报友好错误
- **验证**:单元测试 +1(`vec_push_grows_and_verifies`),示例 examples/29_vec.hz,负例 vec_oob;test.sh 35 passed
- **文档**:docs/USAGE.md 新增「Vec 动态数组使用」章节、类型表行、运行时错误行

---

## 2026-09-01 更新:移除快照回归机制描述

- `test.sh` 的回归验证始终为:**编译并运行 examples/ 全部示例,校验退出码**(当前 28 个)。
- 此前文档描述的 `test/snapshots/` 输出快照逐字节比对与 `UPDATE=1` 重建流程,在实际脚本中并不存在(快照目录从未入库、脚本无比对逻辑),相关描述已从文档移除。
- 同期功能:`strcmp` 字符串比较、除零/越界运行时检查、`rand/srand/time/exit/sleep_ms`、`read_file/write_file`、for-in 数组遍历(见各功能提交)。

---

## 2026-08-30 更新(三)

完成 **argc/argv 与管道 stdin**(P1 第 3 项补充)与**四则计算器模块化示例**(两次提交):

- **内置函数**:`arg_count()`/`arg(i)`(越界返回空串)/`is_eof()`;main 以 C 的 `i32 main(i32, ptr)` ABI 编译,实参存入模块级全局;read_* 读到 EOF 置位 `huzi_eof` 标志(codegen 新增 args.rs)。
- **链接修复**:msvc 路径去掉 `/ENTRY:main`,改用默认 `mainCRTStartup`,CRT 启动后正确传入 argc/argv(原先 OS 直接调 main,参数为垃圾值)。
- **索引修复**:字符串参数 `s: str` 解析为 `Named("str")`,compile_fn 参数槽补上 elem 元数据,`s[i]` 可用。
- **示例**:examples/25_calculator.hz(主程序编排)+ mods/calc_input.hz(输入:读行/找运算符/文本→数值)+ mods/calc_core.hz(四则运算);支持小数、空格、管道与交互输入,q 或 EOF 退出,含非法表达式与除零提示。
- **文档**:USAGE.md 补充命令行参数/管道输入章节;test.sh 为 24 号管道示例喂 stdin。

---

## 2026-08-30 更新(二)

完成**调试支持**(P4 第 13 项全部三个子项,两次提交):

- **AST 携带位置**(纯重构提交):huzi-ast 新增 `Span`/`Spanned<T>`,Program/Block 语句携带行列号;parser 在语句起点记录位置,合成语句(if 表达式/match 手臂/elif 折叠)继承对应关键字位置;codegen `compile_stmt` 改为 `(&Stmt, Span)` 签名。行为零变化(示例输出不变)。
- **DWARF 生成**(功能提交):`codegen/debuginfo.rs` —— `-g` 时创建 DICompileUnit,每个函数挂 DISubprogram,语句按 Span 设置 DILocation,参数与 let 变量挂 `llvm.dbg.declare`(含结构体/数据枚举/元组的 DICompositeType,成员偏移按 LLVM 布局计算);文件模块按各自路径生成 DIFile。
- **CLI/流水线**:`-g/--debug` 选项,隐含 opt-level 0;llc 加 `-debugger-tune=gdb`;链接器加 `/DEBUG`(msvc)或 `-g`(clang/mingw)。
- **关键修复**:模块须带 `Debug Info Version` 标志,否则 llc 报 "invalid version (0)" 丢弃全部调试信息;`fs::canonicalize` 的 `\\?\` 前缀须剥除,否则 gdb 找不到源文件。
- **验证**:`llvm-dwarfdump` 确认编译单元/子程序/变量/行号表;GDB 14.2 实测断点按源码行命中、`print x` 得到正确值、源码行显示正常(Windows 建议 `-l mingw` 配合 gdb)。单元测试 +4(parser span、codegen DI 元数据/无 DI 干净输出);示例输出不变。

## 2026-08-30 更新

完成了开发者体验与优化相关共 8 项 TODO(4 次提交):

- **优化级别**:新增 `--opt-level 0-3`;`--release` 等价于 level 2。内联/常量折叠/CSE 由 `opt` 内置 pass 序列承担。
- **错误体验**:错误报告带源码行摘录 + `^` 列指示(huzi-error render);未定义变量/函数附 Levenshtein "did you mean" 建议(huzi-error suggest)。
- **单元测试**:lexer/parser/codegen/error 共 22 个 `#[cfg(test)]` 测试。

同日完成**模块系统**(P2 最后一个语言特性缺口):

- `import` 语句:内置模块(`math`)与文件模块;点分名(`mods.helpers`)对应子路径,符号绑定为末段名。
- 文件模块只允许 fn/struct/enum/import,限定调用 `模块::函数`(`::` 复用枚举构造语法,codegen 按已注册模块调度);模块 struct/enum 注册为全局名。
- 加载器支持路径去重、循环导入截断、同名多文件报错(入口 crates/huzc/src/modules.rs)。
- 示例 examples/22_modules.hz + examples/mods/helpers.hz,文档见 docs/USAGE.md「模块系统」。

## 2026-08-29 修复记录

本轮修复了此前实现中的大量正确性问题，全部 26 个示例现在可编译并正确运行：

- **控制流**：elif 分支此前被静默丢弃；分支内 return 产生非法 IR；while 循环控制流错误。
  现已全部修复，并新增 `break` / `continue`。
- **解析器**：修复 `let mut` 语法；支持数组元素赋值 `arr[0] = x`；支持 if 表达式
  `let m = if c { a } else { b }`（含 elif 链）。
- **错误定位**：Token 携带行列号，解析/词法错误报告真实位置（此前硬编码 line 1）。
- **类型系统**：bool 统一为 i1；赋值/返回/调用参数做类型协调，不匹配报错；
  `mut` 不可变性强制；块级作用域。
- **print**：布尔打印 true/false；字符串作为参数传递（修复 `%` 格式串注入）；
  修复比较结果打印垃圾数字。
- **read_line**：真正实现（此前只分配缓冲区不读输入）。
- **入口**：顶层级语句自动合成 main；无 main 时报编译错误。
- **工程**：verify 失败改为硬失败；alloca 固定插入 entry 顶部；
  Windows SDK 路径自动探测；clippy 清零；test.sh 变为真实回归测试。
