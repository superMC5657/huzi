//! `box` 堆分配与空指针:嵌套解析、堆单元分配、alloca 初始化。

use super::super::{box_nest::BoxNest, CodeGen};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// `box(expr)` — 求值后在堆上分配单个内容单元并存入,返回 Box 指针。
    /// 有 AST 期望(`Box<T>`,可嵌套)时把 inner 协调到直接内容类型;
    /// 否则由内容推导嵌套(结构体内容 1 层,Box 内容层数 +1)。
    /// 返回新 Box 值的嵌套描述(层数 + 最内层结构体)。
    pub(in super::super) fn compile_box_alloc(
        &mut self,
        inner: &Expr,
        expected: Option<&Type>,
    ) -> Result<(BasicValueEnum<'ctx>, BoxNest<'ctx>)> {
        if Self::is_null_expr(inner) {
            return Err(HuziError::new_global(
                "box(null) is meaningless; use `null` directly for an empty Box slot",
            ));
        }
        // 门卫放行窗:`box(Env { ... })` 内层按值字面量为合法堆构造,
        // 计数内 `compile_struct_literal` 跳过按值门卫(零 IR 改动)。
        self.in_box_alloc += 1;
        let compiled = self.compile_expr(inner);
        self.in_box_alloc = self.in_box_alloc.saturating_sub(1);
        let mut val = compiled?;
        let nest = match expected {
            Some(exp) => {
                let (coerced, nest) = self.resolve_box_alloc_expected(exp, val)?;
                val = coerced;
                nest
            }
            None => self.infer_box_alloc_nest(inner, val)?,
        };
        let user_ptr = self.alloc_box_cell(val, &nest)?;
        Ok((user_ptr.into(), nest))
    }

    /// 有 Box 期望时:把 inner 协调到直接内容类型并取期望嵌套描述。
    /// 期望非 Box(含 weak 剥离后仍非 Box)时报错。
    fn resolve_box_alloc_expected(
        &mut self,
        expected: &Type,
        val: BasicValueEnum<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, BoxNest<'ctx>)> {
        match expected {
            exp @ Type::Box(t) => {
                let target = self.type_to_llvm(t)?;
                let coerced = self.coerce_value(target, val)?;
                let nest = self.box_nest_of_ast(exp)?.ok_or_else(|| {
                    HuziError::new_global(format!(
                        "Cannot assign a Box value to non-Box type '{}'; use a `: Box<...>` slot",
                        exp
                    ))
                })?;
                Ok((coerced, nest))
            }
            other => Err(HuziError::new_global(format!(
                "Cannot assign a Box value to non-Box type '{}'; use a `: Box<...>` slot",
                other
            ))),
        }
    }

    /// 无期望时:由内容推导嵌套(结构体内容 1 层,Box 内容层数 +1)。
    /// 标量内容同样支持(`i32`/`i64`/`f64`/`bool`/`str`),均经 `box_nest` 判定。
    fn infer_box_alloc_nest(
        &self,
        inner: &Expr,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BoxNest<'ctx>> {
        let content = self.box_content_nest(inner).ok_or_else(|| {
            HuziError::new_global(format!(
                "box() requires a struct or scalar value (i32/i64/f64/bool/str) (found '{}'); write box(Node {{ ... }}) or box(42)",
                val.get_type()
            ))
        })?;
        Ok(BoxNest {
            ultimate: content.ultimate,
            depth: content.depth + 1,
        })
    }

    /// 堆单元分配:malloc(内容 + 16 字节双计数头)+ 计数器初始化 + 存入内容。
    /// 单层单元存结构体,嵌套单元存下一层指针;返回指向用户区的指针。
    fn alloc_box_cell(
        &mut self,
        val: BasicValueEnum<'ctx>,
        nest: &BoxNest<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        // 堆单元类型:单层为结构体,嵌套为指针(持有下一层指针)。
        let cell_ty = if nest.depth == 1 {
            nest.ultimate
        } else {
            self.context.ptr_type(inkwell::AddressSpace::default()).into()
        };
        let cell_bytes = self.elem_bytes_i32(cell_ty)?;
        let sixteen = self.context.i32_type().const_int(16, false);
        let eight = self.context.i32_type().const_int(8, false);
        let total_bytes = self.builder.build_int_add(cell_bytes, sixteen, "box_sz").unwrap();
        let malloc_fn = self.module.get_function("malloc").expect("malloc in prelude");
        let raw_ptr = self
            .builder
            .build_call(malloc_fn, &[total_bytes.into()], "box_raw")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        let one_i64 = self.context.i64_type().const_int(1, false);
        // raw_ptr + 0: weak_count = 1
        self.builder.build_store(raw_ptr, one_i64).unwrap();
        // raw_ptr + 8: strong_count = 1
        let strong_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), raw_ptr, &[eight], "box_strong_init")
                .unwrap()
        };
        self.builder.build_store(strong_ptr, one_i64).unwrap();
        // raw_ptr + 16: user_ptr
        let user_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), raw_ptr, &[sixteen], "box_user")
                .unwrap()
        };
        self.builder.build_store(user_ptr, val).unwrap();
        Ok(user_ptr)
    }

    /// 空指针常量(LLVM 层面与 `str` 同为指针,合法性由各期望位置校验)。
    pub(in super::super) fn compile_null(&self) -> Result<BasicValueEnum<'ctx>> {
        Ok(self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null()
            .into())
    }

    /// 在入口块为 Box 槽创建 alloca 并初始化为 null(保证未进入赋值分支时槽位为 safe null)。
    pub(in super::super) fn build_box_alloca(
        &self,
        ty: BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>> {
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
        let ptr = builder
            .build_alloca(ty, name)
            .map_err(|_| HuziError::new_global("Failed to build alloca"))?;
        builder.build_store(ptr, ty.const_zero()).unwrap();
        Ok(ptr)
    }
}
