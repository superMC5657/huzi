# RFC: Result<T> 与后缀 `?` 运算符（冻结）

> 状态：已实现并冻结。更新日期：2026-09-20。
> 关联：`rfc_p4_generics.md`（单态化）、`rfc_generic_inference.md`（调用点推导）。

## 1. 目标

错误处理从"哨兵元组 `(bool, 值, str)`"升级为具名的泛型 `Result<T>`（自举标准库
`core.result`），并引入 Rust 风格的后缀 `?` 运算符，让错误传播一行完成：

```huzi
import core.result

fn checked_double(raw: str) -> Result<i32> {
    let n = parse_num(raw)?
    if n > 100 {
        return result::err_i32("too large")
    }
    return result::ok_i32(n * 2)
}
```

## 2. 类型与构造（core/result，纯 Huzi）

```huzi
struct Result<T> {
    ok: bool,
    value: T,
    err: str,
}
```

* 错误类型固定为 `str`（错误信息）；`err` 字段永远存在。
* 构造器为逐类型具体函数：`ok_i32/err_i32/ok_str/err_str`。
* 泛型辅助：`is_ok<T> / is_err<T> / unwrap_or<T>(r, dflt) / err_msg<T>(r)`。
* 旧哨兵元组 API（`unwrap_or_i32` 等）已移除，由泛型版替代。

## 3. `?` 语义（编译器）

1. **形式**：后缀运算符 `expr?`，绑定优先级高于全部二元运算符；可链式
   （`parse(a)? + parse(b)?`）与嵌套。
2. **操作数**：鸭子类型校验——结构体值，且含字段 `ok`（必须 bool）、`value`、
   `err`（按字段名定位，与声明顺序无关）；否则编译错误。
3. **成功路径**：表达式值为 `value` 字段。
4. **失败路径**：立即从当前函数返回**整个 Result 值**（语义等价 `return expr`）。
   经 `emit_defers` / Box 清理出口，defer 照常执行。
5. **所在函数返回类型**：必须与操作数类型一致（LLVM 类型相等校验），即返回
   同一个 `Result<T>`；否则编译错误。`main`（`-> i32`）中不能用 `?`。
6. **限制（OUT）**：
   * 无泛型错误类型（`err` 恒为 `str`）；无 `Result<T, E>` 双参数。
   * `Box<T>` 作为 `value` 字段类型未定义行为，暂不支持。
   * `?` 不做自动包装/转换（错误类型统一，无需 From）。

## 4. 顺带修复的既有缺口（模块泛型）

实现中发现模块（导入库）中的泛型完全不可用，本次一并修复：

* 模块内泛型 struct/fn 模板会被 `register_module_types` / 函数体编译直接消费，
  在未替换的 `T` 上报 `Unresolved generic type parameter`；现在模板跳过注册与
  编译，只编译单态化产物（与主程序一致）。
* 类型注册与签名注册拆分：先注册全部具名类型（含主程序里的单态化产物
  `Result__i32`），再统一编译模块/主程序签名，避免签名编译时类型缺失。
* 限定调用 `mod::gen_fn(args)` 在 AST 层解析为 `EnumConstruct`（与
  `Enum::Variant` 同形），monomorphizer 原本不处理，导致模块泛型函数永不
  实例化；现在非已知枚举的 `EnumConstruct` 按限定调用参与推导与实例化
  （推断/实例化逻辑抽为 `try_monomorphize_call` 共用），推断器签名查找与
  模板查找支持 `末段::` 限定名回退。

## 5. 测试与验收

* 示例 `58_result_question.hz`：`?` 链、双 `?` 求和、错误信息提取。
* 负例：`question_non_result_fn`（函数返回类型不符）、
  `question_non_result_operand`（操作数非结构体）、
  `question_missing_fields`（缺 err 字段）。
* `huzi-src`：core/result 重写 + core_test / package_entry_test 更新。
* 门禁：`cargo build --workspace` 零警告、`cargo test --workspace` 全过、
  `bash test.sh` 109 项全过、`huzc fmt --check` 示例全过。
