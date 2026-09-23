//! 泛型函数实参类型推导：在调用点未显式给出 `<T>` 时，根据形参与实参类型反推。

use crate::codegen::qname::{bare_name, qualified};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
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

    /// 在泛型调用点根据形参列表和实参列表推导具体类型实参。
    pub(super) fn infer_call_type_args(
        &self,
        callee_name: &str,
        template: &FnStmt,
        args: &[Expr],
    ) -> Result<Vec<Type>> {
        if args.len() != template.params.len() {
            return Err(HuziError::new_global(format!(
                "泛型函数 '{}' 实参数量不匹配:期望 {} 个,实际 {} 个",
                callee_name,
                template.params.len(),
                args.len()
            )));
        }

        let mut inferred: HashMap<String, Type> = HashMap::new();
        for (param, arg) in template.params.iter().zip(args.iter()) {
            if !matches!(arg, Expr::Closure(_)) {
                if let Some(arg_ty) = self.infer_expr_type(arg) {
                    let _ = self.unify_type(
                        &param.param_type,
                        &arg_ty,
                        &template.type_params,
                        &mut inferred,
                    );
                }
            }
        }
        for (idx, (param, arg)) in template.params.iter().zip(args.iter()).enumerate() {
            let expected_ty = crate::codegen::generic::subst::substitute_type(&param.param_type, &inferred);
            let arg_ty = match arg {
                Expr::Closure(c) => self.infer_closure_type(c, Some(&expected_ty)),
                _ => self.infer_expr_type(arg),
            }.ok_or_else(|| {
                HuziError::new_global(format!(
                    "泛型函数 '{}' 第 {} 个实参类型无法推导:形参为 '{}: {}',请显式指定类型实参,如 `{}<{}>(...)`",
                    callee_name,
                    idx + 1,
                    param.name,
                    param.param_type,
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
                    "泛型函数 '{}' 的类型形参 '{}' 无法推导:期望由实参确定,实际没有对应推导来源;请显式指定,如 `{}<{}>(...)`",
                    callee_name, tp, callee_name,
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
                            "类型形参 '{}' 推导冲突:期望各实参推导结果一致,实际先后为 '{}' 与 '{}';帮助:统一对应实参类型,或显式写出类型实参",
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
                    if let Some((base_name, actual_args)) = self
                        .instantiated_struct_types
                        .get(mangled)
                        .or_else(|| self.instantiated_enum_types.get(mangled))
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
            Type::Fn(p_params, p_ret) => {
                if let Type::Fn(a_params, a_ret) = arg_ty {
                    if p_params.len() == a_params.len() {
                        for (pp, ap) in p_params.iter().zip(a_params) {
                            self.unify_type(pp, ap, type_params, inferred)?;
                        }
                        self.unify_type(p_ret, a_ret, type_params, inferred)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 根据泛型枚举变体构造的实参反推枚举的类型实参。
    pub(super) fn infer_enum_variant_type_args(
        &self,
        enum_name: &str,
        variant_name: &str,
        template: &EnumDef,
        args: &[Expr],
    ) -> Result<Vec<Type>> {
        let variant = template
            .variants
            .iter()
            .find(|v| v.name == variant_name)
            .ok_or_else(|| {
                HuziError::new_global(format!(
                    "枚举 '{}' 不存在变体 '{}'",
                    enum_name, variant_name
                ))
            })?;

        if args.len() != variant.payloads.len() {
            return Err(HuziError::new_global(format!(
                "枚举变体 '{}::{}' 载荷参数数量不匹配:期望 {} 个,实际 {} 个",
                enum_name,
                variant_name,
                variant.payloads.len(),
                args.len()
            )));
        }

        let mut inferred: HashMap<String, Type> = HashMap::new();
        for (idx, (payload_ty, arg)) in variant.payloads.iter().zip(args.iter()).enumerate() {
            let arg_ty = self.infer_expr_type(arg).ok_or_else(|| {
                HuziError::new_global(format!(
                    "枚举变体 '{}::{}' 第 {} 个载荷实参类型无法推导:期望为 '{}',请显式指定类型实参,如 `{}<...>::{}(...)`",
                    enum_name, variant_name, idx + 1, payload_ty, enum_name, variant_name
                ))
            })?;
            self.unify_type(payload_ty, &arg_ty, &template.type_params, &mut inferred)?;
        }

        let mut result = Vec::new();
        for tp in &template.type_params {
            if let Some(ty) = inferred.get(tp) {
                result.push(ty.clone());
            } else {
                return Err(HuziError::new_global(format!(
                    "泛型枚举 '{}' 的类型形参 '{}' 无法根据变体 '{}' 的实参推导:请显式指定类型实参,如 `{}<...>::{}(...)`",
                    enum_name, tp, variant_name, enum_name, variant_name
                )));
            }
        }
        Ok(result)
    }
}
