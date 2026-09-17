use super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::PointerValue;
use std::collections::HashMap;

/// 当前函数内注册的一条 `defer` 语句。
#[derive(Clone)]
pub(super) struct DeferEntry<'ctx> {
    pub(super) flag: PointerValue<'ctx>,
    pub(super) stmt: Spanned<Stmt>,
    pub(super) scopes: Vec<HashMap<String, VarSlot<'ctx>>>,
}

impl<'ctx> CodeGen<'ctx> {
    /// 在入口块创建 defer_flag 并初始化为 0 (false)。
    fn build_defer_flag(&self) -> Result<PointerValue<'ctx>> {
        let ty = self.context.bool_type();
        let function = self.current_function()?;
        let entry = function
            .get_first_basic_block()
            .ok_or_else(|| HuziError::new_global("Function has no entry block"))?;
        let builder = self.context.create_builder();
        if let Some(first) = entry.get_first_instruction() {
            builder.position_before(&first);
        } else {
            builder.position_at_end(entry);
        }
        let ptr = builder.build_alloca(ty, "defer_flag").unwrap();
        builder.build_store(ptr, ty.const_int(0, false)).unwrap();
        Ok(ptr)
    }

    /// 注册一条 `defer` 语句:在入口块分配标志位,在当前位置置 1,压入 defer 栈。
    pub(super) fn compile_defer(&mut self, inner: &Spanned<Stmt>) -> Result<()> {
        let flag_ptr = self.build_defer_flag()?;
        let ty = self.context.bool_type();
        self.builder
            .build_store(flag_ptr, ty.const_int(1, false))
            .unwrap();

        self.defer_stack.push(DeferEntry {
            flag: flag_ptr,
            stmt: inner.clone(),
            scopes: self.scopes.clone(),
        });
        Ok(())
    }

    /// 统一退出点:按 LIFO 逆序执行当前已注册的 defer 栈。
    pub(super) fn emit_defers(&mut self) -> Result<()> {
        if self.defer_stack.is_empty() {
            return Ok(());
        }
        let entries: Vec<DeferEntry<'ctx>> = self.defer_stack.clone();
        for entry in entries.into_iter().rev() {
            self.emit_single_defer(entry)?;
        }
        Ok(())
    }

    /// 单条 defer 条件分发:flag 为真时执行 stmt,否则跳过。
    fn emit_single_defer(&mut self, entry: DeferEntry<'ctx>) -> Result<()> {
        let cond = self
            .builder
            .build_load(self.context.bool_type(), entry.flag, "defer_run")
            .unwrap()
            .into_int_value();
        let function = self.current_function()?;
        let then_block = self.context.append_basic_block(function, "defer_then");
        let cont_block = self.context.append_basic_block(function, "defer_cont");
        self.builder
            .build_conditional_branch(cond, then_block, cont_block)
            .unwrap();

        self.builder.position_at_end(then_block);
        let saved_scopes = std::mem::replace(&mut self.scopes, entry.scopes);
        self.compile_stmt(&entry.stmt.node, entry.stmt.span)?;
        self.scopes = saved_scopes;

        if self.at_open_end() {
            self.builder.build_unconditional_branch(cont_block).unwrap();
        }
        self.builder.position_at_end(cont_block);
        Ok(())
    }
}
