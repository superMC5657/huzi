//! `Box<T>` + `null`:堆分配智能指针,支持自引用结构体(典型用例:单链表)、
//! 基础类型(`i32`/`i64`/`f64`/`bool`/`str`)与嵌套 `Box<Box<T>>`
//! (每层仍是指针,堆单元逐层持有下一层指针)。
//!
//! 表示:`Box<T>` 降为普通指针(`ty` 为 ptr),嵌套层数与最内层 pointee
//! (结构体或标量)记录在变量槽的 `box_inner: BoxNest`(变量)或字段的
//! `ast_ty` (结构体字段,按需解析)中。`box(expr)` 求值后 `malloc`
//! 存入并返回指针(复用 `vec.rs` 的堆分配模式);`null` 为空指针常量,
//! 只能出现在 `Box<T>` 期望位置。结构体字段读写逐层自动解引用,
//! 标量经前缀 `*b` 显式解引用(直达最内层,逐层空检查);`==`/`!=`
//! 支持 Box vs null(判空)与 Box vs Box(比指针)。RC 语义见 `runtime.rs`
//! (分配点计数为 1,退出点统一 release)。不做 `free`/GC(泄漏可接受,
//! 见 USAGE)。中间层 Box 不可具名取出:嵌套整体判空/打印/直达最内层,
//! 保持不透明。
//!
//! 子模块:`check`(赋值相容校验)、`alloc`(`box` 堆分配与空指针)、
//! `let_`(`let` 绑定 Box/null)、`compare`(取址解引用与 Box 比较)。

mod alloc;
mod check;
mod compare;
mod let_;

use super::{CodeGen, VarSlot};
use huzi_ast::*;
use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;

/// `==`/`!=` 的 Box 操作数:AST 表达式(判 Box/null 身份) + 已编译值(指针比较)。
pub(super) struct BoxOperand<'a, 'ctx> {
    pub(super) expr: &'a Expr,
    pub(super) value: BasicValueEnum<'ctx>,
}

impl<'ctx> CodeGen<'ctx> {
    /// LLVM 类型是否为已注册的具名结构体(`Box` 的合法 pointee)。
    pub(super) fn is_box_pointee(&self, ty: BasicTypeEnum<'ctx>) -> bool {
        self.struct_def_by_type(ty).is_some()
    }

    /// 变量槽是否为 Box(指针类型 + `box_inner` 标记 pointee)。
    pub(super) fn is_box_slot(slot: &VarSlot<'_>) -> bool {
        slot.box_inner.is_some()
    }

    /// AST 类型是否为 `Box<_>`。
    pub(super) fn is_box_ast(ty: &Type) -> bool {
        matches!(ty, Type::Box(_))
    }

    /// AST 类型是否为 `weak Box<_>`。
    pub(super) fn is_weak_ast(ty: &Type) -> bool {
        matches!(ty, Type::Weak(_))
    }

    /// AST 类型是否为强 Box 或弱 weak Box。
    pub(super) fn is_box_or_weak_ast(ty: &Type) -> bool {
        matches!(ty, Type::Box(_) | Type::Weak(_))
    }

    /// 变量是否为 weak 类型。
    pub(super) fn is_weak_var(&self, name: &str) -> bool {
        self.local_ast.get(name).map(Self::is_weak_ast).unwrap_or(false)
    }

    /// 表达式是否为 weak 类型。
    pub(super) fn is_weak_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Ident(name) => self.is_weak_var(name),
            Expr::FieldAccess(fa) => self
                .field_ast_type(&fa.base, &fa.field)
                .map(|t| Self::is_weak_ast(&t))
                .unwrap_or(false),
            _ => false,
        }
    }

    /// 表达式是否为 `null` 字面量。
    pub(super) fn is_null_expr(expr: &Expr) -> bool {
        matches!(expr, Expr::Null)
    }

    /// 表达式是否求值为 Box 或 weak Box(变量槽标记 / 字段 / `box` 构造)。
    pub(super) fn is_box_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::BoxAlloc(_) => true,
            Expr::Null => false,
            Expr::Ident(name) => self
                .scope_lookup(name)
                .map(|s| Self::is_box_slot(&s) || self.is_weak_var(name))
                .unwrap_or(false),
            Expr::FieldAccess(fa) => self
                .field_ast_type(&fa.base, &fa.field)
                .map(|t| Self::is_box_or_weak_ast(&t))
                .unwrap_or(false),
            Expr::Call(call) => {
                if let Expr::Ident(name) = &*call.callee {
                    let key = self.qualify_name(name);
                    self.fn_return_ast
                        .get(&key)
                        .map(Self::is_box_or_weak_ast)
                        .unwrap_or(false)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// 某结构体值的字段 AST 类型(`base` 为变量或字段链,逐层经 Box 解引用)。
    pub(super) fn field_ast_type(&self, base: &Expr, field: &str) -> Option<Type> {
        let (_, fields) = self.struct_def_of_expr(base)?;
        fields
            .iter()
            .find(|f| f.name == field)
            .map(|f| f.ast_ty.clone())
    }
}
