//! 内置函数前置声明与小助手:libc/libm/网络/线程/参数声明 + 布尔串与串缓冲。
//!
//! 子模块:`libc`(C 运行时声明)、`libm_net_thread`(数学/网络/线程声明)。
//! (自 `builtins.rs` 纯搬移,零逻辑变化。)

mod libc;
mod libm_net_thread;

use super::CodeGen;
use huzi_error::Result;
use inkwell::values::PointerValue;

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn prelude(&mut self) -> Result<()> {
        self.declare_libc_functions();
        self.declare_libm_functions();
        self.declare_net_functions();
        self.declare_thread_functions();
        self.declare_arg_support();
        if cfg!(windows) {
            self.declare_windows_argv_imports();
        }
        Ok(())
    }

    /// 根据给定的 i1 条件选择并构建全局 "true"/"false" 字符串。
    pub(super) fn build_bool_str(
        &mut self,
        cond: inkwell::values::IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let true_ptr = match self.module.get_global("huzi_str_true") {
            Some(g) => g.as_pointer_value(),
            None => unsafe { self.builder.build_global_string("true", "huzi_str_true").unwrap() }
                .as_pointer_value(),
        };
        let false_ptr = match self.module.get_global("huzi_str_false") {
            Some(g) => g.as_pointer_value(),
            None => unsafe {
                self.builder
                    .build_global_string("false", "huzi_str_false")
                    .unwrap()
            }
            .as_pointer_value(),
        };

        let selected = self
            .builder
            .build_select(cond, true_ptr, false_ptr, "bool_str")
            .unwrap();

        Ok(selected.into_pointer_value())
    }

    /// 通过 malloc 分配给定大小的字符串缓冲区。
    pub(super) fn alloc_str_buffer(&mut self, size: u64) -> Result<PointerValue<'ctx>> {
        let malloc_fn = self.module.get_function("malloc").unwrap();
        let buffer_size = self.context.i32_type().const_int(size, false);
        let buffer = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "str_buffer")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        Ok(buffer)
    }
}
