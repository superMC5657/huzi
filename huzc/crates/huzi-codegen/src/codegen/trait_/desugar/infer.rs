//! 接收者类型推导:供方法解析前确定接收者具名类型。

use super::super::TraitDesugarer;
use huzi_ast::*;
use std::collections::HashMap;

impl TraitDesugarer {
    pub(in super::super) fn infer_expr_type(&self, expr: &Expr, env: &HashMap<String, Type>) -> Option<Type> {
        match expr {
            Expr::Ident(name) => env.get(name).cloned(),
            Expr::Literal(lit) => match lit {
                Literal::Int(_) => Some(Type::I32),
                Literal::Float(_) => Some(Type::F64),
                Literal::Bool(_) => Some(Type::Bool),
                Literal::String(_) => Some(Type::Str),
                Literal::Char(_) => Some(Type::Char),
            },
            Expr::FString(_) => Some(Type::Str),
            Expr::StructLiteral(s) => {
                if !s.type_args.is_empty() {
                    Some(Type::Applied(s.name.clone(), s.type_args.clone()))
                } else {
                    Some(Type::Named(s.name.clone()))
                }
            }
            Expr::EnumConstruct(e) => self.infer_enum_construct(e),
            Expr::FieldAccess(fa) => self.infer_field_access(fa, env),
            Expr::Call(c) => self.infer_call(c),
            Expr::MethodCall(mc) => self.infer_method_call(mc, env),
            Expr::ArrayLiteral(elems) => {
                let elem_ty = elems
                    .first()
                    .and_then(|e| self.infer_expr_type(e, env))
                    .unwrap_or(Type::I32);
                Some(Type::Array(Box::new(elem_ty), elems.len()))
            }
            Expr::BoxAlloc(inner) => {
                let inner_ty = self.infer_expr_type(inner, env)?;
                Some(Type::Box(Box::new(inner_ty)))
            }
            Expr::Binary(b) => match b.operator {
                BinOp::Eq
                | BinOp::Neq
                | BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::And
                | BinOp::Or => Some(Type::Bool),
                _ => self.infer_expr_type(&b.left, env),
            },
            Expr::Unary(u) => match u.operator {
                UnOp::Not => Some(Type::Bool),
                UnOp::Neg => self.infer_expr_type(&u.operand, env),
                UnOp::Deref => match self.infer_expr_type(&u.operand, env) {
                    Some(Type::Box(inner)) => Some(*inner),
                    other => other,
                },
            },
            Expr::Match(m) => self.infer_match(m, env),
            _ => None,
        }
    }

    /// 推导枚举构造的返回类型:修饰名/全名/变体名查函数表,未知回落具名类型。
    fn infer_enum_construct(&self, e: &EnumConstructExpr) -> Option<Type> {
        let mangled = format!("{}__{}", e.enum_name, e.variant);
        if let Some(ret) = self.fn_return_types.get(&mangled) {
            if let Some(r) = ret {
                return Some(r.clone());
            }
        }
        let full_name = format!("{}::{}", e.enum_name, e.variant);
        if let Some(ret) = self.fn_return_types.get(&full_name).or_else(|| self.fn_return_types.get(&e.variant)) {
            if let Some(r) = ret {
                return Some(r.clone());
            }
        }
        if self.known_types.contains(&e.enum_name) {
            if !e.type_args.is_empty() {
                return Some(Type::Applied(e.enum_name.clone(), e.type_args.clone()));
            } else {
                return Some(Type::Named(e.enum_name.clone()));
            }
        }
        None
    }

    /// 推导字段访问类型:基址经 Box 解引用到结构体名后查字段表。
    fn infer_field_access(&self, fa: &FieldAccessExpr, env: &HashMap<String, Type>) -> Option<Type> {
        let base_ty = self.infer_expr_type(&fa.base, env)?;
        let s_name = match base_ty {
            Type::Named(n) => n,
            Type::Box(inner) => match *inner {
                Type::Named(n) => n,
                _ => return None,
            },
            _ => return None,
        };
        self.struct_fields.get(&s_name)?.get(&fa.field).cloned()
    }

    /// 推导调用返回类型:vec/map 构造按形参特化,其余查函数返回表。
    fn infer_call(&self, c: &CallExpr) -> Option<Type> {
        if let Expr::Ident(name) = &*c.callee {
            if name == "vec" {
                return Some(Type::Applied("vec".to_string(), c.type_args.clone()));
            }
            if name == "map" || name == "Map" || name == "HashMap" {
                return Some(Type::Applied("map".to_string(), c.type_args.clone()));
            }
            self.fn_return_types.get(name).cloned().flatten()
        } else {
            None
        }
    }

    /// 推导方法调用返回类型:先查该类型的方法返回表,再回落全局/vec 前缀函数表。
    fn infer_method_call(&self, mc: &MethodCallExpr, env: &HashMap<String, Type>) -> Option<Type> {
        let receiver_ty = self.infer_expr_type(&mc.receiver, env)?;
        let base_name = match &receiver_ty {
            Type::Named(type_name) => Some(type_name.as_str()),
            Type::Applied(type_name, _) => Some(type_name.as_str()),
            _ => None,
        };
        if let Some(type_name) = base_name {
            if let Some(ret) = self
                .method_return_types
                .get(type_name)
                .and_then(|m| m.get(&mc.method))
            {
                return ret.clone();
            }
        }
        if let Some(ret) = self.fn_return_types.get(&mc.method) {
            return ret.clone();
        }
        let vec_fn = format!("vec_{}", mc.method);
        self.fn_return_types.get(&vec_fn).cloned().flatten()
    }

    /// 推导 match 类型:以首臂尾表达式(值位置)类型为代表。
    fn infer_match(&self, m: &MatchExpr, env: &HashMap<String, Type>) -> Option<Type> {
        if let Some(first_arm) = m.arms.first() {
            if let Some(stmt) = first_arm.body.statements.last() {
                if let Stmt::Expr(e) = &stmt.node {
                    return self.infer_expr_type(&e.expr, env);
                }
            }
        }
        None
    }
}
