//! 泛型函数实参类型推导：在调用点未显式给出 `<T>` 时，根据形参与实参类型反推。

mod call_args;
mod unify;

use crate::codegen::qname::{bare_name, qualified};
use huzi_ast::*;
use std::collections::HashMap;

/// 类型推导器：跟踪局部变量类型与已知函数/结构体签名，完成实参类型推导。
#[derive(Clone)]
pub(super) struct TypeInferrer {
    var_scopes: Vec<HashMap<String, Type>>,
    pub(super) fn_signatures: HashMap<String, (Vec<Type>, Option<Type>)>,
    pub(super) struct_defs: HashMap<String, StructDef>,
    pub(super) instantiated_struct_types: HashMap<String, (String, Vec<Type>)>,
    pub(super) instantiated_enum_types: HashMap<String, (String, Vec<Type>)>,
    pub(super) known_enums: std::collections::HashSet<String>,
}

impl TypeInferrer {
    pub(super) fn new() -> Self {
        Self {
            var_scopes: vec![HashMap::new()],
            fn_signatures: HashMap::new(),
            struct_defs: HashMap::new(),
            instantiated_struct_types: HashMap::new(),
            instantiated_enum_types: HashMap::new(),
            known_enums: std::collections::HashSet::new(),
        }
    }

    pub(super) fn enter_scope(&mut self) {
        self.var_scopes.push(HashMap::new());
    }

    pub(super) fn leave_scope(&mut self) {
        if self.var_scopes.len() > 1 {
            self.var_scopes.pop();
        }
    }

    pub(super) fn insert_var(&mut self, name: &str, ty: Type) {
        if let Some(scope) = self.var_scopes.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }

    pub(super) fn lookup_var(&self, name: &str) -> Option<&Type> {
        for scope in self.var_scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty);
            }
        }
        None
    }

    /// 根据表达式形态推导其静态类型。
    pub(super) fn infer_expr_type(&self, expr: &Expr) -> Option<Type> {
        match expr {
            Expr::Literal(lit) => match lit {
                Literal::Int(_) => Some(Type::Named("i32".to_string())),
                Literal::Float(_) => Some(Type::Named("f64".to_string())),
                Literal::String(_) => Some(Type::Named("str".to_string())),
                Literal::Char(_) => Some(Type::Named("char".to_string())),
                Literal::Bool(_) => Some(Type::Named("bool".to_string())),
            },
            Expr::Ident(name) => self.lookup_var(name).cloned().or_else(|| {
                self.fn_signatures.get(name).map(|(params, ret)| {
                    let ret_ty = ret.clone().unwrap_or_else(|| Type::Named("void".to_string()));
                    Type::Fn(params.clone(), Box::new(ret_ty))
                })
            }),
            Expr::BoxAlloc(inner) => {
                let inner_ty = self.infer_expr_type(inner)?;
                Some(Type::Box(Box::new(inner_ty)))
            }
            Expr::VecEmpty(ty) => Some(Type::Applied("vec".to_string(), vec![ty.clone()])),
            Expr::StructLiteral(s) => Some(Type::Named(s.name.clone())),
            Expr::Binary(b) => self.infer_binary_type(b),
            Expr::Unary(u) => self.infer_unary_type(u),
            Expr::FieldAccess(f) => self.infer_field_access_type(f),
            Expr::ArrayIndex(a) => self.infer_array_index_type(a),
            Expr::TupleLiteral(elems) => {
                let types: Option<Vec<Type>> =
                    elems.iter().map(|e| self.infer_expr_type(e)).collect();
                types.map(Type::Tuple)
            }
            Expr::ArrayLiteral(elems) => {
                let first = elems.first()?;
                let elem_ty = self.infer_expr_type(first)?;
                Some(Type::Array(Box::new(elem_ty), elems.len()))
            }
            Expr::Call(c) => self.infer_call_expr_type(c),
            Expr::Closure(c) => self.infer_closure_type(c, None),
            Expr::EnumConstruct(ec) => self.infer_enum_construct_type(ec),
            Expr::If(i) => self.infer_block_type(&i.then_branch),
            Expr::Assign(a) => self.infer_expr_type(&a.value),
            Expr::FString(_) => Some(Type::Str),
            _ => None,
        }
    }

    fn infer_enum_construct_type(&self, ec: &EnumConstructExpr) -> Option<Type> {
        let full_name = qualified(&ec.enum_name, &ec.variant);
        if let Some((_, ret)) = self
            .fn_signatures
            .get(&full_name)
            .or_else(|| self.fn_signatures.get(&ec.variant))
        {
            return ret.clone();
        }
        if let Some(ty) = self.infer_builtin_name_type(&ec.variant) {
            return Some(ty);
        }
        Some(Type::Named(ec.enum_name.clone()))
    }

    fn infer_block_type(&self, block: &Block) -> Option<Type> {
        let last = block.statements.last()?;
        match &last.node {
            Stmt::Expr(e) => self.infer_expr_type(&e.expr),
            Stmt::Return(r) => r.value.as_ref().and_then(|v| self.infer_expr_type(v)),
            _ => None,
        }
    }

    fn infer_binary_type(&self, b: &BinaryExpr) -> Option<Type> {
        match b.operator {
            BinOp::Eq
            | BinOp::Neq
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::Le
            | BinOp::Ge
            | BinOp::And
            | BinOp::Or => Some(Type::Named("bool".to_string())),
            _ => self
                .infer_expr_type(&b.left)
                .or_else(|| self.infer_expr_type(&b.right)),
        }
    }

    fn infer_unary_type(&self, u: &UnaryExpr) -> Option<Type> {
        match u.operator {
            UnOp::Not => Some(Type::Named("bool".to_string())),
            UnOp::Neg => self.infer_expr_type(&u.operand),
            // 前缀解引用 `*b`:穿透全部 Box 层直达最内层(与 codegen 一致),
            // 其它情形返回 None 交由调用点报"无法推导"(不放宽检查)。
            UnOp::Deref => {
                let mut cur = self.infer_expr_type(&u.operand)?;
                let mut peeled = false;
                while let Type::Box(inner) = cur {
                    cur = *inner;
                    peeled = true;
                }
                if peeled {
                    Some(cur)
                } else {
                    None
                }
            },
        }
    }

    fn infer_field_access_type(&self, f: &FieldAccessExpr) -> Option<Type> {
        let base_ty = self.infer_expr_type(&f.base)?;
        let struct_name = match base_ty {
            Type::Named(n) => n,
            Type::Box(inner) => match *inner {
                Type::Named(n) => n,
                _ => return None,
            },
            _ => return None,
        };
        let sdef = self.struct_defs.get(&struct_name)?;
        sdef.fields
            .iter()
            .find(|field| field.name == f.field)
            .map(|field| field.field_type.clone())
    }

    fn infer_array_index_type(&self, a: &ArrayIndexExpr) -> Option<Type> {
        let arr_ty = self.infer_expr_type(&a.array)?;
        match arr_ty {
            Type::Array(elem, _) => Some(*elem),
            Type::Applied(name, args) if name == "vec" => args.first().cloned(),
            Type::Named(name) if name == "str" => Some(Type::Named("char".to_string())),
            _ => None,
        }
    }

    fn infer_builtin_name_type(&self, name: &str) -> Option<Type> {
        match name {
            "len" | "abs" | "read_int" | "rand" | "arg_count" | "map_len" | "ref_count" | "weak_count" => {
                Some(Type::Named("i32".to_string()))
            }
            "time" => Some(Type::Named("i64".to_string())),
            "sqrt" | "sin" | "cos" | "tan" | "floor" | "ceil" | "round" | "read_float" => {
                Some(Type::Named("f64".to_string()))
            }
            "to_string" | "concat" | "substring" | "trim" | "read_line" | "read_file"
            | "arg" | "format" => Some(Type::Named("str".to_string())),
            "contains" | "is_eof" | "arg_ok" | "read_file_ok" | "map_has" => {
                Some(Type::Named("bool".to_string()))
            }
            "process_run" => Some(Type::Tuple(vec![
                Type::Named("i32".to_string()),
                Type::Named("str".to_string()),
            ])),
            "map_keys" => Some(Type::Applied(
                "vec".to_string(),
                vec![Type::Named("str".to_string())],
            )),
            _ => None,
        }
    }

    fn infer_call_expr_type(&self, c: &CallExpr) -> Option<Type> {
        if let Some(Type::Fn(_, ret)) = self.infer_expr_type(&c.callee) {
            return Some(*ret);
        }
        let name = match &*c.callee {
            Expr::Ident(n) => n,
            _ => return None,
        };
        if let Some((_, ret)) = self.fn_signatures.get(name) {
            return ret.clone();
        }
        // 限定调用(`result::ok_i32`):签名按定义名(末段)收录。
        let bare = bare_name(name);
        if bare != name.as_str() {
            if let Some((_, ret)) = self.fn_signatures.get(bare) {
                return ret.clone();
            }
        }
        if let Some(ty) = self.infer_builtin_name_type(name.as_str()) {
            return Some(ty);
        }
        match name.as_str() {
            "vec" => {
                let first = c.arguments.first()?;
                let elem_ty = self.infer_expr_type(first)?;
                Some(Type::Applied("vec".to_string(), vec![elem_ty]))
            }
            "box" => {
                let first = c.arguments.first()?;
                let inner_ty = self.infer_expr_type(first)?;
                Some(Type::Box(Box::new(inner_ty)))
            }
            "map_new" => Some(Type::Named("Map".to_string())),
            _ => None,
        }
    }

    pub(super) fn infer_closure_type(
        &self,
        c: &ClosureExpr,
        expected: Option<&Type>,
    ) -> Option<Type> {
        let (expected_params, expected_ret) = match expected {
            Some(Type::Fn(params, ret)) => (Some(params), Some(ret.as_ref())),
            _ => (None, None),
        };
        let mut param_types = Vec::with_capacity(c.params.len());
        for (i, p) in c.params.iter().enumerate() {
            if let Some(t) = &p.param_type {
                param_types.push(t.clone());
            } else if let Some(exp_p) = expected_params.and_then(|ps| ps.get(i)) {
                param_types.push(exp_p.clone());
            } else {
                return None;
            }
        }
        let ret_type = if let Some(r) = &c.return_type {
            r.clone()
        } else {
            let mut sub_infer = self.clone();
            sub_infer.enter_scope();
            for (p, t) in c.params.iter().zip(param_types.iter()) {
                sub_infer.insert_var(&p.name, t.clone());
            }
            let body_ty = match &c.body {
                ClosureBody::Expr(e) => sub_infer.infer_expr_type(e),
                ClosureBody::Block(b) => sub_infer.infer_block_type(b),
            };
            if let Some(bt) = body_ty {
                bt
            } else if let Some(exp_r) = expected_ret {
                exp_r.clone()
            } else {
                return None;
            }
        };
        Some(Type::Fn(param_types, Box::new(ret_type)))
    }
}
