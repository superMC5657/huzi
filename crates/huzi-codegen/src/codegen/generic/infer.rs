//! 泛型函数实参类型推导：在调用点未显式给出 `<T>` 时，根据形参与实参类型反推。

use huzi_ast::*;
use huzi_error::{HuziError, Result};
use std::collections::HashMap;

/// 类型推导器：跟踪局部变量类型与已知函数/结构体签名，完成实参类型推导。
pub(super) struct TypeInferrer {
    var_scopes: Vec<HashMap<String, Type>>,
    pub(super) fn_signatures: HashMap<String, (Vec<Type>, Option<Type>)>,
    pub(super) struct_defs: HashMap<String, StructDef>,
    pub(super) instantiated_struct_types: HashMap<String, (String, Vec<Type>)>,
    pub(super) known_enums: std::collections::HashSet<String>,
}

impl TypeInferrer {
    pub(super) fn new() -> Self {
        Self {
            var_scopes: vec![HashMap::new()],
            fn_signatures: HashMap::new(),
            struct_defs: HashMap::new(),
            instantiated_struct_types: HashMap::new(),
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
            Expr::Ident(name) => self.lookup_var(name).cloned(),
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
            Expr::EnumConstruct(ec) => self.infer_enum_construct_type(ec),
            Expr::If(i) => self.infer_block_type(&i.then_branch),
            Expr::Assign(a) => self.infer_expr_type(&a.value),
            _ => None,
        }
    }

    fn infer_enum_construct_type(&self, ec: &EnumConstructExpr) -> Option<Type> {
        let full_name = format!("{}::{}", ec.enum_name, ec.variant);
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
            "len" | "abs" | "read_int" | "rand" | "arg_count" | "map_len" | "ref_count" => {
                Some(Type::Named("i32".to_string()))
            }
            "time" => Some(Type::Named("i64".to_string())),
            "sqrt" | "sin" | "cos" | "tan" | "floor" | "ceil" | "round" | "read_float" => {
                Some(Type::Named("f64".to_string()))
            }
            "to_string" | "concat" | "substring" | "trim" | "read_line" | "read_file"
            | "arg" => Some(Type::Named("str".to_string())),
            "contains" | "is_eof" | "arg_ok" | "read_file_ok" | "map_has" => {
                Some(Type::Named("bool".to_string()))
            }
            _ => None,
        }
    }

    fn infer_call_expr_type(&self, c: &CallExpr) -> Option<Type> {
        let name = match &*c.callee {
            Expr::Ident(n) => n,
            _ => return None,
        };
        if let Some((_, ret)) = self.fn_signatures.get(name) {
            return ret.clone();
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

    /// 在泛型调用点根据形参列表和实参列表推导具体类型实参。
    pub(super) fn infer_call_type_args(
        &self,
        callee_name: &str,
        template: &FnStmt,
        args: &[Expr],
    ) -> Result<Vec<Type>> {
        if args.len() != template.params.len() {
            return Err(HuziError::new_global(format!(
                "Generic function '{}' expects {} argument(s), got {}",
                callee_name,
                template.params.len(),
                args.len()
            )));
        }

        let mut inferred: HashMap<String, Type> = HashMap::new();
        for (param, arg) in template.params.iter().zip(args.iter()) {
            let arg_ty = self.infer_expr_type(arg).ok_or_else(|| {
                HuziError::new_global(format!(
                    "Cannot infer type of argument for generic function '{}', please specify explicitly with {}<{}>",
                    callee_name,
                    callee_name,
                    template.type_params.join(", ")
                ))
            })?;
            self.unify_type(
                &param.param_type,
                &arg_ty,
                &template.type_params,
                &mut inferred,
            )?;
        }

        let mut result = Vec::new();
        for tp in &template.type_params {
            if let Some(ty) = inferred.get(tp) {
                result.push(ty.clone());
            } else {
                return Err(HuziError::new_global(format!(
                    "Cannot infer type argument '{}' for generic function '{}', please specify explicitly with {}<{}>",
                    tp,
                    callee_name,
                    callee_name,
                    template.type_params.join(", ")
                )));
            }
        }
        Ok(result)
    }

    /// 将形参类型与实参类型进行统一匹配，收集类型变量的具体绑定。
    pub(super) fn unify_type(
        &self,
        param_ty: &Type,
        arg_ty: &Type,
        type_params: &[String],
        inferred: &mut HashMap<String, Type>,
    ) -> Result<()> {
        match param_ty {
            Type::Generic(name) | Type::Named(name) if type_params.contains(name) => {
                if let Some(existing) = inferred.get(name) {
                    if existing != arg_ty {
                        return Err(HuziError::new_global(format!(
                            "Type inference conflict for '{}': deduced both '{}' and '{}'",
                            name, existing, arg_ty
                        )));
                    }
                } else {
                    inferred.insert(name.clone(), arg_ty.clone());
                }
            }
            Type::Box(inner_p) => {
                if let Type::Box(inner_a) = arg_ty {
                    self.unify_type(inner_p, inner_a, type_params, inferred)?;
                }
            }
            Type::Applied(p_name, p_args) => {
                if let Type::Applied(a_name, a_args) = arg_ty {
                    if p_name == a_name && p_args.len() == a_args.len() {
                        for (pa, aa) in p_args.iter().zip(a_args) {
                            self.unify_type(pa, aa, type_params, inferred)?;
                        }
                    }
                } else if let Type::Named(mangled) = arg_ty {
                    if let Some((base_name, actual_args)) =
                        self.instantiated_struct_types.get(mangled)
                    {
                        if p_name == base_name && p_args.len() == actual_args.len() {
                            for (pa, aa) in p_args.iter().zip(actual_args) {
                                self.unify_type(pa, aa, type_params, inferred)?;
                            }
                        }
                    }
                }
            }
            Type::Array(p_elem, _) => {
                if let Type::Array(a_elem, _) = arg_ty {
                    self.unify_type(p_elem, a_elem, type_params, inferred)?;
                }
            }
            Type::Tuple(p_elems) => {
                if let Type::Tuple(a_elems) = arg_ty {
                    if p_elems.len() == a_elems.len() {
                        for (pe, ae) in p_elems.iter().zip(a_elems) {
                            self.unify_type(pe, ae, type_params, inferred)?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
