# RFC: P4 用户泛型（显式单态化）

## 1. 目标与范围
- **目标**：为 Huzi 语言提供最小可用的泛型结构体（Generic Struct）与泛型函数（Generic Function），基于编译期单态化（Monomorphization）实现零运行时抽象开销。
- **IN**：
  - 显式类型参数定义：`struct Stack<T> { ... }`、`fn id<T>(x: T) -> T { ... }`。
  - 显式类型实参实例化：`Stack<i32>`、`id<i32>(42)`。
  - 支持多类型参数：`Pair<K, V>`、`fn zip<A, B>(...)`。
  - 名称修饰（Mangle）：`id<i32>` -> `id__i32`、`Pair<i32, str>` -> `Pair__i32_str`。
  - 实例化去重与缓存：相同类型实参仅实例化一次。
  - 修复 `vec` 字段类型注解语法：支持在结构体中声明 `v: vec<i32>`。
- **OUT**：
  - 无隐式类型推导（调用必须写显式实参 `f<T>(...)`）。
  - 无泛型枚举与模式匹配穷尽检查。
  - 无 `where` 子句与 trait 约束（留至 P5）。
  - 无特化（Specialization）。

## 2. 语法规范

### 2.1 泛型结构体
```huzi
struct Stack<T> {
    v: vec<T>,
    top: T,
}

struct Pair<K, V> {
    key: K,
    val: V,
}
```

### 2.2 泛型函数
```huzi
fn id<T>(x: T) -> T {
    return x
}

fn make_pair<K, V>(k: K, v: V) -> Pair<K, V> {
    return Pair<K, V> { key: k, val: v }
}
```

### 2.3 实例化语法
- 泛型函数调用：`id<i32>(42)`
- 泛型结构体实例化：`Pair<i32, str> { key: 1, val: "hello" }`
- 泛型类型标注：`let s: Stack<i32> = ...`

## 3. 名称修饰规则（Name Mangling）
类型转换为修饰标识符字符串：
- 基础标量：`i32`, `i64`, `u32`, `u64`, `f32`, `f64`, `bool`, `str`, `char`, `unit`
- 具名类型：`Name` 直接作为修饰段
- 复合类型：
  - `Box<T>` -> `Box__<mangled T>`
  - `vec<T>` -> `vec__<mangled T>`
  - `Name<T1, T2>` -> `Name__<mangled T1>_<mangled T2>`
- 单态化函数名：`<fn_name>__<mangled T1>_<mangled T2>`
- 单态化结构体名：`<struct_name>__<mangled T1>_<mangled T2>`

## 4. 单态化编译管线
1. **收集阶段**：
   - 顶层扫描时，凡 `!type_params.is_empty()` 的结构体与函数存入模板表（`generic_struct_templates` / `generic_fn_templates`），不立即发射 LLVM 实体。
2. **实例化阶段**：
   - 遍历 AST 或在遇到 `Type::Applied` / `CallExpr.type_args` / `StructLiteralExpr.type_args` 时触发按需单态化。
   - 检查实参个数，错配时报 `Generic 'X' expects N type argument(s), got M`。
   - 检查是否有未绑定的泛型参数，非法时报错。
   - 根据修饰名查询缓存：若已实例化则直接复用；若未实例化，执行 AST 深拷贝与类型参数替换。
   - 替换后的特化结构体注册进 `CodeGen.structs`，特化函数注册并排入待编译队列。
3. **编译阶段**：
   - 特化函数与特化结构体按既有代码生成逻辑进行编译与校验，复用现有所有类型检查与优化。
