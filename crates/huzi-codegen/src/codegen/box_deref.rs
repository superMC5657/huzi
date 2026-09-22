//! 前缀 `*b` 解引用:穿透全部 Box 层直达最内层值(结构体或标量),
//! 逐层空检查(空指针触发运行时错误并退出,见 `runtime.rs`)。
//!
//! 中间层保持不透明(与字段自动解引用一致):`Box<Box<i32>>` 经
//! 单次 `*` 直达 `i32`,无需逐层具名取出。ABI 不变(仍为 ptr +
//! 负偏移 RC 头),RC 计数语义不变。

use super::box_nest::BoxNest;
use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// `*b` 读值:操作数须为 Box,空字面量与非 Box 均报编译期错误;
    /// 运行期逐层判空,末层装载最内层值返回。
    pub(super) fn compile_deref(&mut self, operand: &Expr) -> Result<BasicValueEnum<'ctx>> {
        if Self::is_null_expr(operand) {
            return Err(HuziError::new_global(
                "Cannot dereference 'null'; dereference a Box<T> variable instead (e.g. `*b`)",
            ));
        }
        let nest = self.box_nest_of_expr(operand).ok_or_else(|| {
            HuziError::new_global(
                "Dereference '*' requires a Box<T> operand (found a non-Box value); write `*b` where `b: Box<T>`",
            )
        })?;
        let ptr = self.compile_box_ptr(operand)?;
        self.load_box_chain(ptr, &nest)
    }

    /// `*b = v` 写值:返回末层单元地址与类型,调用方校验并存入。
    pub(super) fn compile_deref_store(
        &mut self,
        unary: &UnaryExpr,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        if Self::is_null_expr(&unary.operand) {
            return Err(HuziError::new_global(
                "Cannot dereference 'null'; assign through a Box<T> variable instead (e.g. `*b = v`)",
            ));
        }
        let nest = self.box_nest_of_expr(&unary.operand).ok_or_else(|| {
            HuziError::new_global(
                "Dereference '*' requires a Box<T> operand (found a non-Box value); write `*b = v` where `b: Box<T>`",
            )
        })?;
        self.ensure_mutable(&unary.operand)?;
        let ptr = self.compile_box_ptr(&unary.operand)?;
        let (cell, cell_ty) = self.walk_box_chain(ptr, &nest)?;
        let value = self.coerce_value(cell_ty, value)?;
        self.builder.build_store(cell, value).unwrap();
        Ok(value)
    }

    /// 操作数编译为 Box 指针(非指针值报内部错,正常不可达)。
    fn compile_box_ptr(&mut self, operand: &Expr) -> Result<PointerValue<'ctx>> {
        match self.compile_expr(operand)? {
            BasicValueEnum::PointerValue(pv) => Ok(pv),
            _ => Err(HuziError::new_global(
                "Dereference '*' requires a Box<T> pointer value",
            )),
        }
    }

    /// 读链:逐层空检查,末层装载最内层值。
    fn load_box_chain(
        &mut self,
        ptr: PointerValue<'ctx>,
        nest: &BoxNest<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        let (cell, cell_ty) = self.walk_box_chain(ptr, nest)?;
        Ok(self.builder.build_load(cell_ty, cell, "box_deref_val").unwrap())
    }

    /// 走链:逐层空检查并装载下一层指针,返回末层单元地址与类型。
    fn walk_box_chain(
        &mut self,
        ptr: PointerValue<'ctx>,
        nest: &BoxNest<'ctx>,
    ) -> Result<(PointerValue<'ctx>, BasicTypeEnum<'ctx>)> {
        let mut cur = ptr;
        let ptr_ty: BasicTypeEnum<'ctx> = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .into();
        for i in 0..nest.depth {
            self.emit_deref_null_check(cur)?;
            if i + 1 == nest.depth {
                return Ok((cur, nest.ultimate));
            }
            cur = self
                .builder
                .build_load(ptr_ty, cur, "box_deref_next")
                .unwrap()
                .into_pointer_value();
        }
        Err(HuziError::new_global("Invalid Box nesting depth"))
    }

    /// 单层空检查:空指针打印运行时错误并以退出码 1 终止。
    fn emit_deref_null_check(&mut self, ptr: PointerValue<'ctx>) -> Result<()> {
        let cond = self.ptr_is_not_null(ptr);
        self.emit_runtime_check(cond, "Runtime error: dereference of null Box\n\0", &[])
    }
}
