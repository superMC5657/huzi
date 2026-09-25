//! `print` 的 vec 值打印:运行期循环按 `len` 逐元素递归打印。
//! (自 `builtins_print.rs` 纯搬移,零逻辑变化。)

use super::super::CodeGen;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 按名取 vec 打印三件套 (data, len, 元素类型),供 `print(v)` 使用。
    pub(super) fn vec_print_parts(
        &mut self,
        name: &str,
    ) -> Result<(
        PointerValue<'ctx>,
        IntValue<'ctx>,
        inkwell::types::BasicTypeEnum<'ctx>,
    )> {
        let slot = self.vec_slot_of(name)?;
        let parts = self.load_vec_parts(&slot)?;
        Ok((parts.data, parts.len, slot.elem.unwrap()))
    }

    /// 是否为 vec 运行期布局 `{ ptr, i32, i32 }`(命名结构体/元组另行判定,
    /// 此处仅用于嵌套元素值的兜底分流)。
    pub(super) fn is_vec_layout(&self, ty: inkwell::types::StructType<'ctx>) -> bool {
        if ty.count_fields() != 3 {
            return false;
        }
        let i32_ty: inkwell::types::BasicTypeEnum<'ctx> = self.context.i32_type().into();
        matches!(
            ty.get_field_type_at_index(0),
            Some(inkwell::types::BasicTypeEnum::PointerType(_))
        ) && ty.get_field_type_at_index(1) == Some(i32_ty)
            && ty.get_field_type_at_index(2) == Some(i32_ty)
    }

    /// 已组装 vec 值的打印(表达式位置的 `vec<T>()` 等):拆出 data/len 后进循环。
    pub(super) fn emit_vec_value(
        &mut self,
        vec_val: BasicValueEnum<'ctx>,
        elem_ty: BasicTypeEnum<'ctx>,
    ) -> Result<()> {
        let ty = vec_val.get_type();
        let st = match ty {
            BasicTypeEnum::StructType(st) => st,
            _ => return Err(HuziError::new_global("print() does not support this value type")),
        };
        let tmp = self.build_alloca(ty, "vec_print_val")?;
        self.builder.build_store(tmp, vec_val).unwrap();
        let data = self
            .builder
            .build_load(
                self.context.ptr_type(inkwell::AddressSpace::default()),
                self.builder.build_struct_gep(st, tmp, 0, "vec_print_data_ptr").unwrap(),
                "vec_print_data",
            )
            .unwrap()
            .into_pointer_value();
        let len = self
            .builder
            .build_load(
                self.context.i32_type(),
                self.builder.build_struct_gep(st, tmp, 1, "vec_print_len_ptr").unwrap(),
                "vec_print_len",
            )
            .unwrap()
            .into_int_value();
        self.emit_vec_loop(data, len, elem_ty)
    }

    /// `[e1, e2, ...]`:运行时按 len 循环,元素复用自身打印逻辑。
    pub(super) fn emit_vec_loop(
        &mut self,
        data: PointerValue<'ctx>,
        len: IntValue<'ctx>,
        elem_ty: BasicTypeEnum<'ctx>,
    ) -> Result<()> {
        self.emit_printf_text("[")?;
        let function = self.current_function()?;
        let i_type = self.context.i32_type();
        let loop_bb = self.context.append_basic_block(function, "vec_print_loop");
        let body_bb = self.context.append_basic_block(function, "vec_print_body");
        let after_bb = self.context.append_basic_block(function, "vec_print_after");
        let idx_alloca = self.build_alloca(i_type.into(), "vec_print_idx")?;
        self.builder.build_store(idx_alloca, i_type.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(loop_bb);
        let idx = self.builder.build_load(i_type, idx_alloca, "vec_print_i").unwrap().into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::ULT, idx, len, "vec_print_cond").unwrap();
        self.builder.build_conditional_branch(cond, body_bb, after_bb).unwrap();
        self.builder.position_at_end(body_bb);
        self.emit_vec_elem(data, elem_ty, idx_alloca)?;
        let next = self.builder.build_int_add(
            self.builder.build_load(i_type, idx_alloca, "vec_print_i").unwrap().into_int_value(),
            i_type.const_int(1, false),
            "vec_print_next",
        ).unwrap();
        self.builder.build_store(idx_alloca, next).unwrap();
        self.builder.build_unconditional_branch(loop_bb).unwrap();
        self.builder.position_at_end(after_bb);
        self.emit_printf_text("]")?;
        Ok(())
    }

    /// 循环体:首元素外先打印 `", "`,再装载并递归打印当前元素。
    fn emit_vec_elem(
        &mut self,
        data: PointerValue<'ctx>,
        elem_ty: BasicTypeEnum<'ctx>,
        idx_alloca: PointerValue<'ctx>,
    ) -> Result<()> {
        let i_type = self.context.i32_type();
        let idx = self.builder.build_load(i_type, idx_alloca, "vec_print_i").unwrap().into_int_value();
        let function = self.current_function()?;
        let sep_bb = self.context.append_basic_block(function, "vec_print_sep");
        let elem_bb = self.context.append_basic_block(function, "vec_print_elem");
        let need_sep = self.builder.build_int_compare(
            inkwell::IntPredicate::NE,
            idx,
            i_type.const_int(0, false),
            "vec_print_sep_cond",
        ).unwrap();
        self.builder.build_conditional_branch(need_sep, sep_bb, elem_bb).unwrap();
        self.builder.position_at_end(sep_bb);
        self.emit_printf_text(", ")?;
        self.builder.build_unconditional_branch(elem_bb).unwrap();
        self.builder.position_at_end(elem_bb);
        let idx = self.builder.build_load(i_type, idx_alloca, "vec_print_i").unwrap().into_int_value();
        let elem_ptr = unsafe { self.builder.build_gep(elem_ty, data, &[idx], "vec_print_elem_ptr").unwrap() };
        let elem = self.builder.build_load(elem_ty, elem_ptr, "vec_print_elem_val").unwrap();
        self.emit_value_print(elem)
    }
}
