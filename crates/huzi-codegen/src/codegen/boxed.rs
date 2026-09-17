//! `Box<T>` + `null`:堆分配智能指针,支持自引用结构体(典型用例:单链表)
//! 与嵌套 `Box<Box<Node>>`(每层仍是指针,堆单元逐层持有下一层指针)。
//!
//! 表示:`Box<T>` 降为普通指针(`ty` 为 ptr),嵌套层数与最内层 pointee
//! 结构体记录在变量槽的 `box_inner: BoxNest`(变量)或字段的 `ast_ty`
//! (结构体字段,按需解析)中。`box(expr)` 求值后 `malloc` 存入并返回
//! 指针(复用 `vec.rs` 的堆分配模式);`null` 为空指针常量,只能出现在
//! `Box<T>` 期望位置。字段读写逐层自动解引用;`==`/`!=` 支持 Box vs
//! null(判空)与 Box vs Box(比指针)。不做 `free`/GC(泄漏可接受,见 USAGE)。
//! 中间层 Box 不可具名取出:嵌套整体判空/打印/直达最内层字段,保持不透明。

use super::{box_nest::BoxNest, CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
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

    /// 表达式是否为 `null` 字面量。
    pub(super) fn is_null_expr(expr: &Expr) -> bool {
        matches!(expr, Expr::Null)
    }

    /// 表达式是否求值为 Box(变量槽标记 / Box 字段 / `box` 构造)。
    pub(super) fn is_box_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::BoxAlloc(_) => true,
            Expr::Null => false,
            Expr::Ident(name) => self
                .scope_lookup(name)
                .map(|s| Self::is_box_slot(&s))
                .unwrap_or(false),
            Expr::FieldAccess(fa) => self
                .field_ast_type(&fa.base, &fa.field)
                .map(|t| Self::is_box_ast(&t))
                .unwrap_or(false),
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

    /// 编译前校验 `box(..)`/`null` 与期望 AST 类型相容。LLVM 层面
    /// `Box<Node>` 与 `Box<Other>` 都是指针,此处做层数 + 结构名比对补位。
    /// 嵌套层数不一致(如 `box(box(..))` 进 `Box<Node>` 槽)同样报错。
    pub(super) fn check_box_assignable(&self, value_expr: &Expr, expected: &Type) -> Result<()> {
        match value_expr {
            Expr::Null => {
                if !Self::is_box_ast(expected) {
                    return Err(HuziError::new_global(format!(
                        "null can only be assigned to a Box<T> slot (found '{}'); add a `: Box<...>` annotation or assign to a Box field/parameter",
                        expected
                    )));
                }
                Ok(())
            }
            Expr::BoxAlloc(inner) => {
                if !Self::is_box_ast(expected) {
                    return Err(HuziError::new_global(format!(
                        "Cannot assign a Box value to non-Box type '{}'; use a `: Box<...>` slot",
                        expected
                    )));
                }
                self.check_box_nest_match(inner, expected)
            }
            _ => Ok(()),
        }
    }

    /// `box(E)` 实际形状与期望 `Box` 形状的层数 + 结构名比对。
    /// 内容不可推导(如函数调用结果)时跳过,交由 LLVM 层决定。
    fn check_box_nest_match(&self, inner: &Expr, expected: &Type) -> Result<()> {
        let (edepth, ename) = Self::box_ast_shape(expected).ok_or_else(|| {
            HuziError::new_global(format!(
                "Cannot assign a Box value to non-Box type '{}'; use a `: Box<...>` slot",
                expected
            ))
        })?;
        let Some((adepth, aname)) = self.box_content_shape(inner) else {
            return Ok(());
        };
        if adepth != edepth || aname != ename {
            return Err(HuziError::new_global(format!(
                "Box type mismatch: value holds '{}', but the slot expects '{}'",
                Self::display_box_nest(adepth, &aname),
                expected
            )));
        }
        Ok(())
    }

    /// `box(expr)` — 求值后在堆上分配单个内容单元并存入,返回 Box 指针。
    /// 有 AST 期望(`Box<T>`,可嵌套)时把 inner 协调到直接内容类型;
    /// 否则由内容推导嵌套(结构体内容 1 层,Box 内容层数 +1)。
    /// 返回新 Box 值的嵌套描述(层数 + 最内层结构体)。
    pub(super) fn compile_box_alloc(
        &mut self,
        inner: &Expr,
        expected: Option<&Type>,
    ) -> Result<(BasicValueEnum<'ctx>, BoxNest<'ctx>)> {
        if Self::is_null_expr(inner) {
            return Err(HuziError::new_global(
                "box(null) is meaningless; use `null` directly for an empty Box slot",
            ));
        }
        let mut val = self.compile_expr(inner)?;
        let nest = match expected {
            Some(exp @ Type::Box(t)) => {
                let target = self.type_to_llvm(t)?;
                val = self.coerce_value(target, val)?;
                self.box_nest_of_ast(exp)?.ok_or_else(|| {
                    HuziError::new_global(format!(
                        "Cannot assign a Box value to non-Box type '{}'; use a `: Box<...>` slot",
                        exp
                    ))
                })?
            }
            Some(other) => {
                return Err(HuziError::new_global(format!(
                    "Cannot assign a Box value to non-Box type '{}'; use a `: Box<...>` slot",
                    other
                )))
            }
            None => {
                let content = self.box_content_nest(inner).ok_or_else(|| {
                    HuziError::new_global(format!(
                        "box() requires a struct value (found '{}'); write box(Node {{ ... }})",
                        val.get_type()
                    ))
                })?;
                BoxNest {
                    ultimate: content.ultimate,
                    depth: content.depth + 1,
                }
            }
        };
        // 堆单元类型:单层为结构体,嵌套为指针(持有下一层指针)。
        let cell_ty = if nest.depth == 1 {
            nest.ultimate
        } else {
            self.context.ptr_type(inkwell::AddressSpace::default()).into()
        };
        let one = self.context.i32_type().const_int(1, false);
        let heap = self
            .builder
            .build_array_malloc(cell_ty, one, "box_alloc")
            .map_err(|_| HuziError::new_global("Failed to allocate Box storage"))?;
        self.builder.build_store(heap, val).unwrap();
        Ok((heap.into(), nest))
    }

    /// 空指针常量(LLVM 层面与 `str` 同为指针,合法性由各期望位置校验)。
    pub(super) fn compile_null(&self) -> Result<BasicValueEnum<'ctx>> {
        Ok(self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null()
            .into())
    }

    /// 取址后若基址是 Box 则自动解引用:按嵌套层数逐层装载堆指针,
    /// 最终类型切为最内层 pointee(`outer.val` 可穿透 `Box<Box<Node>>`)。
    pub(super) fn compile_addr_deref(
        &mut self,
        expr: &Expr,
    ) -> Result<(
        inkwell::values::PointerValue<'ctx>,
        BasicTypeEnum<'ctx>,
    )> {
        let (mut ptr, slot_ty) = self.compile_addr(expr)?;
        let Some(nest) = self.box_nest_of_expr(expr) else {
            return Ok((ptr, slot_ty));
        };
        let mut load_ty = slot_ty;
        let ptr_ty: BasicTypeEnum<'ctx> = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .into();
        for _ in 0..nest.depth {
            let loaded = self
                .builder
                .build_load(load_ty, ptr, "box_deref")
                .unwrap()
                .into_pointer_value();
            ptr = loaded;
            load_ty = ptr_ty;
        }
        Ok((ptr, nest.ultimate))
    }

    /// `==`/`!=` 的 Box 路径:Box vs null 判空,Box vs Box 比指针。
    /// 调用方已确认至少一侧是 Box 或 null。
    pub(super) fn build_box_compare(
        &mut self,
        op: &BinOp,
        left: BoxOperand<'_, 'ctx>,
        right: BoxOperand<'_, 'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        if *op != BinOp::Eq && *op != BinOp::Neq {
            return Err(HuziError::new_global(
                "Box<T> values only support '==' and '!=' (use `x == null` to test for empty)",
            ));
        }
        let l_box = self.is_box_expr(left.expr);
        let r_box = self.is_box_expr(right.expr);
        let l_null = Self::is_null_expr(left.expr);
        let r_null = Self::is_null_expr(right.expr);
        if l_null && r_null {
            return Err(HuziError::new_global(
                "Cannot compare 'null' with 'null'; compare a Box<T> value with null instead",
            ));
        }
        if (l_null && !r_box) || (r_null && !l_box) {
            return Err(HuziError::new_global(
                "null can only be compared with a Box<T> value using '==' or '!='",
            ));
        }
        if (l_box && !r_box && !r_null) || (r_box && !l_box && !l_null) {
            return Err(HuziError::new_global(
                "Cannot compare a Box<T> value with a non-Box value; compare it with null or another Box instead",
            ));
        }
        let i64_ty = self.context.i64_type();
        let l_ptr = match left.value {
            BasicValueEnum::PointerValue(pv) => pv,
            _ => return Err(HuziError::new_global("Box value is not a pointer")),
        };
        let r_ptr = match right.value {
            BasicValueEnum::PointerValue(pv) => pv,
            _ => return Err(HuziError::new_global("Box value is not a pointer")),
        };
        let l_int = self.builder.build_ptr_to_int(l_ptr, i64_ty, "box_l").unwrap();
        let r_int = self.builder.build_ptr_to_int(r_ptr, i64_ty, "box_r").unwrap();
        let pred = if *op == BinOp::Eq {
            inkwell::IntPredicate::EQ
        } else {
            inkwell::IntPredicate::NE
        };
        Ok(self
            .builder
            .build_int_compare(pred, l_int, r_int, "box_cmp")
            .unwrap()
            .into())
    }

    /// 是否走 Box 比较路径(任一侧是 Box 或 null)。
    pub(super) fn is_box_comparison(&self, left: &Expr, right: &Expr) -> bool {
        self.is_box_expr(left) || self.is_box_expr(right) || Self::is_null_expr(left) || Self::is_null_expr(right)
    }

    /// `let x[: Box<T>] = null` — 标注须为 Box(裸 `let x = null` 无法推导)。
    pub(super) fn compile_let_null(&mut self, stmt: &LetStmt, span: Span) -> Result<()> {
        let ann = stmt.type_annotation.as_ref().ok_or_else(|| {
            HuziError::new_global(
                "Cannot infer the type of 'null'; add a `: Box<T>` annotation (e.g. `let x: Box<Node> = null`)",
            )
        })?;
        if !Self::is_box_ast(ann) {
            return Err(HuziError::new_global(format!(
                "null can only be assigned to a Box<T> slot (found '{}')",
                ann
            )));
        }
        let ptr_ty = self.type_to_llvm(ann)?;
        let box_inner = self.box_nest_of_ast(ann)?;
        let alloca = self.build_alloca(ptr_ty, &stmt.name)?;
        self.builder.build_store(alloca, ptr_ty.const_zero()).unwrap();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: ptr_ty,
                elem: None,
                array_len: None,
                mutable: stmt.mutable,
                box_inner,
            },
        );
        self.declare_local(&stmt.name, alloca, ptr_ty, span);
        Ok(())
    }

    /// `let x[: Box<T>] = box(inner)` — 有标注时校验 inner 与 T,无标注时推导。
    /// 嵌套标注(`Box<Box<Node>>`)记录层数 + 最内层,供后续逐层解引用。
    pub(super) fn compile_let_box(&mut self, stmt: &LetStmt, inner: &Expr, span: Span) -> Result<()> {
        if let Some(ann) = &stmt.type_annotation {
            self.check_box_assignable(&Expr::BoxAlloc(Box::new(inner.clone())), ann)?;
            let (val, nest) = self.compile_box_alloc(inner, Some(ann))?;
            let ptr_ty = self.type_to_llvm(ann)?;
            self.insert_box_slot(&stmt.name, ptr_ty, val, nest, stmt.mutable, span)?;
            return Ok(());
        }
        let (val, nest) = self.compile_box_alloc(inner, None)?;
        let var_ty = val.get_type();
        self.insert_box_slot(&stmt.name, var_ty, val, nest, stmt.mutable, span)?;
        Ok(())
    }

    /// Box 变量槽插入(标注/推导路径共用):指针类型 + 嵌套描述。
    fn insert_box_slot(
        &mut self,
        name: &str,
        slot_ty: BasicTypeEnum<'ctx>,
        val: BasicValueEnum<'ctx>,
        nest: BoxNest<'ctx>,
        mutable: bool,
        span: Span,
    ) -> Result<()> {
        let alloca = self.build_alloca(slot_ty, name)?;
        self.builder.build_store(alloca, val).unwrap();
        self.scope_insert(
            name.to_string(),
            VarSlot {
                ptr: alloca,
                ty: slot_ty,
                elem: None,
                array_len: None,
                mutable,
                box_inner: Some(nest),
            },
        );
        self.declare_local(name, alloca, slot_ty, span);
        Ok(())
    }
}
