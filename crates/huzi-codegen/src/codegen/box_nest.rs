//! `Box` 嵌套查询:AST 形状(层数 + 最内层名)与 LLVM 嵌套描述
//! (`BoxNest`:最内层 pointee + 层数)的纯查询 helper,供 `boxed.rs`
//! 的校验/分配/解引用与打印链使用。本模块不生成任何指令。
//!
//! 最内层 pointee 可以是具名结构体或标量(`i32`/`i64`/`f64`/
//! `bool`/`str`,另含 `u32`/`u64`/`f32`/`char`);嵌套每层仍是普通
//! 指针,堆单元逐层持有下一层指针,最内层持有值。

use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;

/// `Box` 嵌套描述:最内层 pointee LLVM 类型 + 嵌套层数
/// (`Box<Node>` 为 1,`Box<Box<i32>>` 为 2)。嵌套每层仍是普通
/// 指针,堆单元逐层持有下一层指针,最内层持有值。
#[derive(Clone, Copy)]
pub(super) struct BoxNest<'ctx> {
    pub(super) ultimate: BasicTypeEnum<'ctx>,
    pub(super) depth: u32,
}

impl<'ctx> CodeGen<'ctx> {
    /// `Box` 的嵌套形状(层数,最内层名),供赋值校验的精确比对。
    /// 非 Box 返回 None;最内层为结构体名或标量名,泛型/复合
    /// 仍返回 None(跳过校验,交由 LLVM 层决定)。
    pub(super) fn box_ast_shape(ty: &Type) -> Option<(u32, String)> {
        let mut depth = 0u32;
        let mut cur = ty;
        while let Type::Box(inner) = cur {
            depth += 1;
            cur = inner;
        }
        if depth == 0 {
            return None;
        }
        Self::canonical_leaf_name(cur).map(|n| (depth, n))
    }

    /// Box 最内层 AST 的规范名:结构体名原样,标量归一到
    /// `i32`/`i64`/`f64`/`bool`/`str` 等;其余返回 None。
    pub(super) fn canonical_leaf_name(ty: &Type) -> Option<String> {
        match ty {
            Type::I32 => Some("i32".to_string()),
            Type::U32 => Some("u32".to_string()),
            Type::I64 => Some("i64".to_string()),
            Type::U64 => Some("u64".to_string()),
            Type::F32 => Some("f32".to_string()),
            Type::F64 => Some("f64".to_string()),
            Type::Bool => Some("bool".to_string()),
            Type::Char => Some("char".to_string()),
            Type::Str => Some("str".to_string()),
            Type::Named(n) => Some(n.clone()),
            _ => None,
        }
    }

    /// 嵌套 Box 类型的展示(`(2, "Node")` -> `Box<Box<Node>>`)。
    pub(super) fn display_box_nest(depth: u32, name: &str) -> String {
        let mut s = name.to_string();
        for _ in 0..depth {
            s = format!("Box<{}>", s);
        }
        s
    }

    /// AST 类型对应的 Box 嵌套描述(层数 + 最内层,其它 -> None)。
    /// 最内层须为已注册结构体或标量,否则报精确的 `Box<T>` 类型错误。
    pub(super) fn box_nest_of_ast(&self, ty: &Type) -> Result<Option<BoxNest<'ctx>>> {
        match ty {
            Type::Box(_) => {
                let mut depth = 0u32;
                let mut cur = ty;
                while let Type::Box(inner) = cur {
                    depth += 1;
                    cur = inner;
                }
                let ultimate = self.type_to_llvm(cur)?;
                if !self.is_box_pointee(ultimate) && !Self::is_box_scalar(cur, ultimate) {
                    return Err(HuziError::new_global(format!(
                        "Box<T> requires a named struct or scalar type (i32/i64/f64/bool/str) (found '{}')",
                        cur
                    )));
                }
                Ok(Some(BoxNest { ultimate, depth }))
            }
            _ => Ok(None),
        }
    }

    /// 表达式的 Box 嵌套描述(Box 变量槽 / Box 字段 / `box` 临时值)。
    /// 非 Box 返回 None;`box` 临时值按内容层数 +1 推导。
    pub(super) fn box_nest_of_expr(&self, expr: &Expr) -> Option<BoxNest<'ctx>> {
        match expr {
            Expr::Ident(name) => self.scope_lookup(name)?.box_inner,
            Expr::FieldAccess(fa) => {
                let ast = self.field_ast_type(&fa.base, &fa.field)?;
                self.box_nest_of_ast(&ast).ok()?
            }
            Expr::BoxAlloc(inner) => {
                let content = self.box_content_nest(inner)?;
                Some(BoxNest {
                    ultimate: content.ultimate,
                    depth: content.depth + 1,
                })
            }
            Expr::Call(call) => {
                if let Expr::Ident(name) = &*call.callee {
                    let key = self.qualify_name(name);
                    let ast = self.fn_return_ast.get(&key)?;
                    self.box_nest_of_ast(ast).ok()?
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// `box(E)` 中内容 E 的嵌套:Box 值沿用层数,结构体/标量值视为 0 层。
    /// 其它表达式返回 None(调用方报 `box()` 参数错误)。
    pub(super) fn box_content_nest(&self, inner: &Expr) -> Option<BoxNest<'ctx>> {
        match inner {
            Expr::Ident(name) => self.content_nest_of_ident(name),
            Expr::FieldAccess(fa) => self.content_nest_of_field(fa),
            Expr::StructLiteral(sl) => self.content_nest_of_struct(&sl.name),
            Expr::Literal(lit) => self.content_nest_of_literal(lit),
            Expr::Call(call) => self.content_nest_of_call(call),
            // E 本身是 `box` 临时值:内容嵌套即该临时值的嵌套(逐层剥离,必终止)。
            Expr::BoxAlloc(_) => self.box_nest_of_expr(inner),
            _ => None,
        }
    }

    /// 变量内容的嵌套:Box 变量沿用,结构体/标量变量视为 0 层。
    fn content_nest_of_ident(&self, name: &str) -> Option<BoxNest<'ctx>> {
        let slot = self.scope_lookup(name)?;
        if let Some(nest) = slot.box_inner {
            return Some(nest);
        }
        if self.struct_def_by_type(slot.ty).is_some() {
            return Some(BoxNest {
                ultimate: slot.ty,
                depth: 0,
            });
        }
        if Self::is_scalar_llvm(slot.ty, slot.elem, slot.array_len) {
            return Some(BoxNest {
                ultimate: slot.ty,
                depth: 0,
            });
        }
        None
    }

    /// 字段内容的嵌套:Box 字段沿用,结构体/标量字段视为 0 层。
    fn content_nest_of_field(&self, fa: &FieldAccessExpr) -> Option<BoxNest<'ctx>> {
        let ast = self.field_ast_type(&fa.base, &fa.field)?;
        if let Type::Box(_) = ast {
            return self.box_nest_of_ast(&ast).ok()?;
        }
        let ultimate = self.type_to_llvm(&ast).ok()?;
        if self.struct_def_by_type(ultimate).is_some() {
            return Some(BoxNest { ultimate, depth: 0 });
        }
        if Self::is_box_scalar(&ast, ultimate) {
            return Some(BoxNest { ultimate, depth: 0 });
        }
        None
    }

    /// 具名结构体的 0 层内容嵌套(供 `box(Node { ... })`)。
    fn content_nest_of_struct(&self, name: &str) -> Option<BoxNest<'ctx>> {
        let (st, _) = self.structs.get(name)?;
        Some(BoxNest {
            ultimate: (*st).into(),
            depth: 0,
        })
    }

    /// 字面量的 0 层内容嵌套(供 `box(42)`/`box("hi")` 等)。
    fn content_nest_of_literal(&self, lit: &Literal) -> Option<BoxNest<'ctx>> {
        let ultimate: BasicTypeEnum<'ctx> = match lit {
            Literal::Int(n) => {
                if *n >= i32::MIN as i64 && *n <= i32::MAX as i64 {
                    self.context.i32_type().into()
                } else {
                    self.context.i64_type().into()
                }
            }
            Literal::Float(_) => self.context.f64_type().into(),
            Literal::Bool(_) => self.context.bool_type().into(),
            Literal::String(_) => self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into(),
            Literal::Char(_) => self.context.i8_type().into(),
        };
        Some(BoxNest { ultimate, depth: 0 })
    }

    /// 调用结果的 0 层内容嵌套:返回 Box 沿用,返回结构体/标量视
    /// 为 0 层;其它返回 None。
    fn content_nest_of_call(&self, call: &CallExpr) -> Option<BoxNest<'ctx>> {
        let Expr::Ident(name) = &*call.callee else {
            return None;
        };
        let key = self.qualify_name(name);
        let ret = self.fn_return_ast.get(&key)?.clone();
        if let Type::Box(_) = ret {
            return self.box_nest_of_ast(&ret).ok()?;
        }
        let ultimate = self.type_to_llvm(&ret).ok()?;
        if self.struct_def_by_type(ultimate).is_some() {
            return Some(BoxNest { ultimate, depth: 0 });
        }
        if Self::is_box_scalar(&ret, ultimate) {
            return Some(BoxNest { ultimate, depth: 0 });
        }
        None
    }

    /// 槽位 LLVM 类型是否为标量值(含 `str` 指针):整数/浮点直接
    /// 认定;指针仅当非数组/非 vec/非 Box 时视为 `str`。
    fn is_scalar_llvm(
        ty: BasicTypeEnum<'ctx>,
        elem: Option<BasicTypeEnum<'ctx>>,
        array_len: Option<u32>,
    ) -> bool {
        use inkwell::types::BasicTypeEnum as BTE;
        match ty {
            BTE::IntType(_) | BTE::FloatType(_) => true,
            BTE::PointerType(_) => elem.is_some() && array_len.is_none(),
            _ => false,
        }
    }

    /// LLVM 结构体类型反查注册名(供 `box(变量)` 的结构名校验)。
    fn struct_name_of_llvm(&self, ty: BasicTypeEnum<'ctx>) -> Option<String> {
        let st = match ty {
            BasicTypeEnum::StructType(st) => st,
            _ => return None,
        };
        self.structs
            .iter()
            .find(|(_, (def_st, _))| *def_st == st)
            .map(|(name, _)| name.clone())
    }

    /// LLVM 究极类型反查展示名:结构体查注册表,标量按位宽归一
    /// (`i1`->`bool`,`i8`->`char`,指针->`str`);其余返回 None。
    pub(super) fn llvm_leaf_name(&self, ty: BasicTypeEnum<'ctx>) -> Option<String> {
        use inkwell::types::BasicTypeEnum as BTE;
        match ty {
            BTE::StructType(_) => self.struct_name_of_llvm(ty),
            BTE::IntType(t) => Some(match t.get_bit_width() {
                1 => "bool".to_string(),
                8 => "char".to_string(),
                32 => "i32".to_string(),
                64 => "i64".to_string(),
                _ => return None,
            }),
            BTE::FloatType(t) => Some(if t == self.context.f32_type() {
                "f32".to_string()
            } else if t == self.context.f64_type() {
                "f64".to_string()
            } else {
                return None;
            }),
            BTE::PointerType(_) => Some("str".to_string()),
            _ => None,
        }
    }

    /// `box(E)` 内容 E 的形状(实际 Box 层数,最内层名):结构体/标量
    /// 字面量/变量/Box 临时值可推导,其它返回 None(跳过校验)。
    pub(super) fn box_content_shape(&self, inner: &Expr) -> Option<(u32, String)> {
        match inner {
            Expr::StructLiteral(sl) => Some((1, sl.name.clone())),
            Expr::Literal(lit) => Some((1, Self::lit_shape_name(lit).to_string())),
            Expr::Ident(name) => {
                let slot = self.scope_lookup(name)?;
                if let Some(nest) = slot.box_inner {
                    Some((nest.depth + 1, self.llvm_leaf_name(nest.ultimate)?))
                } else if slot.array_len.is_some() || Self::is_vec_slot(&slot) {
                    None
                } else if self.struct_def_by_type(slot.ty).is_some() {
                    Some((1, self.struct_name_of_llvm(slot.ty)?))
                } else {
                    Some((1, self.llvm_leaf_name(slot.ty)?))
                }
            }
            Expr::FieldAccess(fa) => {
                let ast = self.field_ast_type(&fa.base, &fa.field)?;
                if let Type::Box(_) = ast {
                    let (d, n) = Self::box_ast_shape(&ast)?;
                    Some((d + 1, n))
                } else {
                    Some((1, Self::canonical_leaf_name(&ast)?))
                }
            }
            Expr::BoxAlloc(deeper) => {
                let (depth, name) = self.box_content_shape(deeper)?;
                Some((depth + 1, name))
            }
            _ => None,
        }
    }

    /// 字面量标量名(供形状校验展示,与内容嵌套的位宽划分一致)。
    fn lit_shape_name(lit: &Literal) -> &'static str {
        match lit {
            Literal::Int(n) => {
                if *n >= i32::MIN as i64 && *n <= i32::MAX as i64 {
                    "i32"
                } else {
                    "i64"
                }
            }
            Literal::Float(_) => "f64",
            Literal::Bool(_) => "bool",
            Literal::String(_) => "str",
            Literal::Char(_) => "char",
        }
    }
}
