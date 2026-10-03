//! 含 vec/Box 字段结构体的按值使用门卫(纯 AST 检查,零 IR 改动)。
//!
//! 根因:Env 类结构体按值拷贝会别名化 vec/堆缓冲且 RC 未跟进;
//! `rc.rs` 本身正确,错在调用方按值传递。因此在 AST 层直接拒绝
//! 按值 `let`/形参/返回/赋值/结构体字面量,引导改 `Box<Env>` 或拆参。
//!
//! `Box<Env>`(顶层 `Box`/`weak`)为指针传递,不触发;纯标量结构体不触发。

use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    /// 字段类型是否直接携带堆句柄(`vec<T>` 或 `Box<T>`/`weak`)。
    fn field_is_heap_direct(ty: &Type) -> bool {
        Self::is_vec_ast(ty) || Self::is_box_or_weak_ast(ty)
    }

    /// 具名结构体是否含堆字段(含传递闭包,如 `Outer { inner: Env }`)。
    fn struct_has_heap_inner(&self, name: &str, visited: &mut Vec<String>) -> bool {
        if visited.iter().any(|n| n == name) {
            return false;
        }
        visited.push(name.to_string());
        let Some((_, fields)) = self.structs.get(name) else {
            visited.pop();
            return false;
        };
        let fields = fields.clone();
        for info in &fields {
            if Self::field_is_heap_direct(&info.ast_ty) {
                visited.pop();
                return true;
            }
            if self.type_refs_heap_struct(&info.ast_ty, visited) {
                visited.pop();
                return true;
            }
        }
        visited.pop();
        false
    }

    /// 字段类型是否引用了含堆结构体(具名/泛型实例/数组/元组透传)。
    fn type_refs_heap_struct(&self, ty: &Type, visited: &mut Vec<String>) -> bool {
        match ty {
            Type::Box(_) | Type::Weak(_) => false,
            Type::Named(n) => {
                if self.structs.contains_key(n) {
                    self.struct_has_heap_inner(n, visited)
                } else {
                    false
                }
            }
            Type::Applied(n, args) => {
                if n == "vec" || n == "Map" || n == "HashMap" || n == "map" {
                    return false;
                }
                if self.structs.contains_key(n) {
                    return self.struct_has_heap_inner(n, visited);
                }
                let mangled = super::generic::mangle_name(n, args);
                if self.structs.contains_key(&mangled) {
                    return self.struct_has_heap_inner(&mangled, visited);
                }
                false
            }
            Type::Array(elem, _) => {
                if Self::field_is_heap_direct(elem) {
                    return true;
                }
                self.type_refs_heap_struct(elem, visited)
            }
            Type::Tuple(elems) => elems.iter().any(|e| {
                Self::field_is_heap_direct(e) || self.type_refs_heap_struct(e, visited)
            }),
            _ => false,
        }
    }

    /// 类型按值使用时是否为被禁的含堆结构体;是则返回其结构体名。
    /// 顶层 `Box<T>`/`weak` 为指针传递,返回 None(放行 `Box<Env>`)。
    pub(super) fn heap_struct_of_type(&self, ty: &Type) -> Option<String> {
        let mut visited = Vec::new();
        self.heap_struct_inner(ty, &mut visited)
    }

    /// `heap_struct_of_type` 的递归实现。
    fn heap_struct_inner(&self, ty: &Type, visited: &mut Vec<String>) -> Option<String> {
        match ty {
            Type::Box(_) | Type::Weak(_) => None,
            Type::Named(n) => {
                if self.structs.contains_key(n) && self.struct_has_heap_inner(n, visited) {
                    Some(n.clone())
                } else {
                    None
                }
            }
            Type::Applied(n, args) => {
                if n == "vec" || n == "Map" || n == "HashMap" || n == "map" {
                    return None;
                }
                if self.structs.contains_key(n) && self.struct_has_heap_inner(n, visited) {
                    return Some(n.clone());
                }
                let mangled = super::generic::mangle_name(n, args);
                if self.structs.contains_key(&mangled)
                    && self.struct_has_heap_inner(&mangled, visited)
                {
                    return Some(mangled);
                }
                None
            }
            Type::Array(elem, _) => self.heap_struct_inner(elem, visited),
            Type::Tuple(elems) => {
                for e in elems {
                    if let Some(n) = self.heap_struct_inner(e, visited) {
                        return Some(n);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// 统一报错构造(无位置,调用方补行列):中文主句 + 改 `Box` 或拆参引导。
    pub(super) fn heap_struct_error(struct_name: &str, kind: &str) -> HuziError {
        HuziError::new_global(format!(
            "结构体 '{}' 含 vec/Box 字段,禁止{}(按值拷贝会导致堆内存别名且引用计数未跟进);请改用 `Box<{}>` 或拆参\n  help: 将类型改为 `Box<{}>` 并以 `box({} {{ ... }})` 构造,或拆为标量/句柄参数分别传递",
            struct_name, kind, struct_name, struct_name, struct_name
        ))
    }

    /// 统一入口:类型按值使用时若为含堆结构体则报错(含行列)。
    pub(super) fn reject_heap_struct_byvalue(&self, ty: &Type, kind: &str) -> Result<()> {
        if let Some(name) = self.heap_struct_of_type(ty) {
            return Err(self.with_current_position(Self::heap_struct_error(&name, kind)));
        }
        Ok(())
    }

    /// 值表达式的 AST 类型(结构体字面量/变量/调用返回/字段链)。
    fn value_ast_type(&self, expr: &Expr) -> Option<Type> {
        match expr {
            Expr::StructLiteral(sl) => {
                if sl.type_args.is_empty() {
                    Some(Type::Named(sl.name.clone()))
                } else {
                    Some(Type::Applied(sl.name.clone(), sl.type_args.clone()))
                }
            }
            Expr::Ident(name) => self.local_ast.get(name).cloned(),
            Expr::Call(c) => match &*c.callee {
                Expr::Ident(fname) => {
                    let key = self.qualify_name(fname);
                    self.fn_return_ast.get(&key).cloned()
                }
                _ => None,
            },
            Expr::FieldAccess(fa) => self.field_ast_type(&fa.base, &fa.field),
            _ => self.static_type_of_value(expr),
        }
    }

    /// 统一入口:值表达式按值流动时若为含堆结构体则报错(含行列)。
    pub(super) fn reject_heap_value_expr(&self, expr: &Expr, kind: &str) -> Result<()> {
        if matches!(expr, Expr::BoxAlloc(_) | Expr::Null) {
            return Ok(());
        }
        if let Some(ty) = self.value_ast_type(expr) {
            if let Some(name) = self.heap_struct_of_type(&ty) {
                return Err(self.with_current_position(Self::heap_struct_error(&name, kind)));
            }
        }
        Ok(())
    }
}
