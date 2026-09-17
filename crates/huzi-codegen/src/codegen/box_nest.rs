//! `Box` 嵌套查询:AST 形状(层数 + 最内层名)与 LLVM 嵌套描述
//! (`BoxNest`:最内层结构体 + 层数)的纯查询 helper,供 `boxed.rs`
//! 的校验/分配/解引用与打印链使用。本模块不生成任何指令。

use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;

/// `Box` 嵌套描述:最内层 pointee 结构体 LLVM 类型 + 嵌套层数
/// (`Box<Node>` 为 1,`Box<Box<Node>>` 为 2)。嵌套每层仍是普通
/// 指针,堆单元逐层持有下一层指针,最内层持有结构体值。
#[derive(Clone, Copy)]
pub(super) struct BoxNest<'ctx> {
    pub(super) ultimate: BasicTypeEnum<'ctx>,
    pub(super) depth: u32,
}

impl<'ctx> CodeGen<'ctx> {
    /// `Box` 的嵌套形状(层数,最内层名),供赋值校验的精确比对。
    /// 非 Box 返回 None;解析期已保证最内层为具名或嵌套 Box。
    pub(super) fn box_ast_shape(ty: &Type) -> Option<(u32, &str)> {
        let mut depth = 0u32;
        let mut cur = ty;
        while let Type::Box(inner) = cur {
            depth += 1;
            cur = inner;
        }
        if depth == 0 {
            return None;
        }
        match cur {
            Type::Named(n) => Some((depth, n)),
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

    /// AST 类型对应的 Box 嵌套描述(层数 + 最内层结构体,其它 -> None)。
    /// 最内层须为已注册结构体,否则报精确的 `Box<T>` 类型错误。
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
                if !self.is_box_pointee(ultimate) {
                    return Err(HuziError::new_global(format!(
                        "Box<T> requires a named struct type (found '{}')",
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
            _ => None,
        }
    }

    /// `box(E)` 中内容 E 的嵌套:Box 值沿用层数,结构体值视为 0 层。
    /// 其它表达式返回 None(调用方报 `box()` 参数错误)。
    pub(super) fn box_content_nest(&self, inner: &Expr) -> Option<BoxNest<'ctx>> {
        match inner {
            Expr::Ident(name) => self.content_nest_of_ident(name),
            Expr::FieldAccess(fa) => self.content_nest_of_field(fa),
            Expr::StructLiteral(sl) => self.content_nest_of_struct(&sl.name),
            // E 本身是 `box` 临时值:内容嵌套即该临时值的嵌套(逐层剥离,必终止)。
            Expr::BoxAlloc(_) => self.box_nest_of_expr(inner),
            _ => None,
        }
    }

    /// 变量内容的嵌套:Box 变量沿用,结构体变量视为 0 层。
    fn content_nest_of_ident(&self, name: &str) -> Option<BoxNest<'ctx>> {
        let slot = self.scope_lookup(name)?;
        if let Some(nest) = slot.box_inner {
            return Some(nest);
        }
        self.struct_def_by_type(slot.ty)?;
        Some(BoxNest {
            ultimate: slot.ty,
            depth: 0,
        })
    }

    /// 字段内容的嵌套:Box 字段沿用,具名结构体字段视为 0 层。
    fn content_nest_of_field(&self, fa: &FieldAccessExpr) -> Option<BoxNest<'ctx>> {
        let ast = self.field_ast_type(&fa.base, &fa.field)?;
        if let Type::Box(_) = ast {
            return self.box_nest_of_ast(&ast).ok()?;
        }
        if let Type::Named(_) = ast {
            let ultimate = self.type_to_llvm(&ast).ok()?;
            self.struct_def_by_type(ultimate)?;
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

    /// `box(E)` 内容 E 的形状(实际 Box 层数,最内层名):结构体字面量/
    /// 变量/Box 临时值可推导,其它返回 None(跳过校验,LLVM 层决定)。
    pub(super) fn box_content_shape(&self, inner: &Expr) -> Option<(u32, String)> {
        match inner {
            Expr::StructLiteral(sl) => Some((1, sl.name.clone())),
            Expr::Ident(name) => {
                let slot = self.scope_lookup(name)?;
                if let Some(nest) = slot.box_inner {
                    Some((nest.depth + 1, self.struct_name_of_llvm(nest.ultimate)?))
                } else {
                    Some((1, self.struct_name_of_llvm(slot.ty)?))
                }
            }
            Expr::BoxAlloc(deeper) => {
                let (depth, name) = self.box_content_shape(deeper)?;
                Some((depth + 1, name))
            }
            _ => None,
        }
    }
}
