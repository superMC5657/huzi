//! `print` 的结构体值打印:`Name {k1: v1, ...}` 编译期字段展开与递归打印。
//! (自 `builtins_print.rs` 纯搬移,零逻辑变化。)

use super::super::{CodeGen, StructFieldInfo};
use huzi_ast::Type;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 按名查结构体定义,无则报错(调用方已确认是结构体值)。
    pub(in crate::codegen) fn struct_name_by_type(&self, ty: BasicTypeEnum<'ctx>) -> Result<String> {
        let st = match ty {
            BasicTypeEnum::StructType(st) => st,
            _ => return Err(HuziError::new_global("print() does not support this value type")),
        };
        self.structs
            .iter()
            .find(|(_, (def_st, _))| *def_st == st)
            .map(|(name, _)| name.clone())
            .ok_or_else(|| HuziError::new_global("print() does not support this value type"))
    }

    /// `Name {k1: v1, k2: v2}`:字段编译期展开,逐字段递归打印。
    pub(super) fn emit_struct_value(&mut self, value: BasicValueEnum<'ctx>) -> Result<()> {
        let ty = value.get_type();
        let name = self.struct_name_by_type(ty)?;
        let (def_st, fields) = self
            .struct_def_by_type(ty)
            .cloned()
            .ok_or_else(|| HuziError::new_global("print() does not support this value type"))?;
        let tmp = self.build_alloca(ty, "struct_print")?;
        self.builder.build_store(tmp, value).unwrap();
        self.emit_printf_text(&format!("{} {{", name))?;
        for (i, info) in fields.iter().enumerate() {
            if i > 0 {
                self.emit_printf_text(", ")?;
            }
            self.emit_printf_text(&format!("{}: ", info.name))?;
            let field_ptr = self
                .builder
                .build_struct_gep(def_st, tmp, i as u32, "struct_print_field")
                .unwrap();
            self.emit_field_print(field_ptr, info)?;
        }
        self.emit_printf_text("}")?;
        Ok(())
    }

    /// 单个结构体字段:数组字段按编译期长度展开,其它递归通用分发。
    pub(in crate::codegen) fn emit_field_print(
        &mut self,
        field_ptr: PointerValue<'ctx>,
        info: &StructFieldInfo<'ctx>,
    ) -> Result<()> {
        // Box 字段递归打印(空指针输出 `null`)。
        if Self::is_box_ast(&info.ast_ty) {
            return self.emit_box_field_print(field_ptr, info);
        }
        if let Type::Array(elem_ast_ty, size) = &info.ast_ty {
            let elem_ty = self.type_to_llvm(elem_ast_ty)?;
            let arr_ptr = self
                .builder
                .build_load(info.ty, field_ptr, "struct_arr")
                .unwrap()
                .into_pointer_value();
            self.emit_printf_text("[")?;
            for i in 0..*size {
                if i > 0 {
                    self.emit_printf_text(", ")?;
                }
                let idx = self.context.i32_type().const_int(i as u64, false);
                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(elem_ty, arr_ptr, &[idx], "struct_arr_elem")
                        .unwrap()
                };
                let elem = self.builder.build_load(elem_ty, elem_ptr, "struct_arr_val").unwrap();
                self.emit_value_print(elem)?;
            }
            self.emit_printf_text("]")?;
            return Ok(());
        }
        let value = self.builder.build_load(info.ty, field_ptr, "struct_field_val").unwrap();
        self.emit_value_print(value)
    }
}
