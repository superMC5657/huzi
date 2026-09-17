//! `print(box)` 的运行时递归打印:Box 自引用类型(如链表)在编译期
//! 内联展开会无限递归,故按结构体类型生成一次性打印机函数
//! (`huzi_print_struct_<Name>`,判空后递归调用,运行时遇到 null 终止)。

use super::{box_nest::BoxNest, CodeGen, StructFieldInfo};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, FunctionValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// 取(或生成)结构体的运行时打印机:`fn(ptr)` 无返回值,打印
    /// `Name {k: v, ...}`;Box 字段判空后递归调用同系列打印机。
    /// 先入缓存再发射函数体,直接自引用(`Box<Node>` 内嵌)可终止。
    pub(super) fn box_struct_printer(&mut self, struct_name: &str) -> Result<FunctionValue<'ctx>> {
        if let Some(f) = self.struct_printers.get(struct_name).copied() {
            return Ok(f);
        }
        let (def_st, fields) = self.structs.get(struct_name).cloned().ok_or_else(|| HuziError::new_global("print() does not support this value type"))?;
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let function = self.module.add_function(
            &format!("huzi_print_struct_{}", struct_name),
            self.context.void_type().fn_type(&[ptr_ty.into()], false),
            None,
        );
        self.struct_printers.insert(struct_name.to_string(), function);
        self.emit_struct_printer_body(function, struct_name, def_st, &fields)?;
        Ok(function)
    }

    /// 打印机函数体:字段逐个打印,Box 字段走判空递归调用,其余内联
    /// (按值嵌套无环,见 `check_type_cycles`,内联必终止)。
    fn emit_struct_printer_body(
        &mut self,
        function: FunctionValue<'ctx>,
        struct_name: &str,
        def_st: inkwell::types::StructType<'ctx>,
        fields: &[StructFieldInfo<'ctx>],
    ) -> Result<()> {
        let saved = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        let struct_ptr = function.get_nth_param(0).unwrap().into_pointer_value();
        self.emit_printf_text(&format!("{} {{", struct_name))?;
        for (i, info) in fields.iter().enumerate() {
            if i > 0 {
                self.emit_printf_text(", ")?;
            }
            self.emit_printf_text(&format!("{}: ", info.name))?;
            let field_ptr = self.builder.build_struct_gep(def_st, struct_ptr, i as u32, "pfield").unwrap();
            if Self::is_box_ast(&info.ast_ty) {
                self.emit_box_field_print(field_ptr, info)?;
            } else {
                self.emit_field_print(field_ptr, info)?;
            }
        }
        self.emit_printf_text("}")?;
        self.builder.build_return(None).unwrap();
        if let Some(saved) = saved {
            self.builder.position_at_end(saved);
        }
        Ok(())
    }

    /// Box 指针的判空递归打印:null 输出 `null`,非空调用 pointee 打印机。
    pub(super) fn emit_box_ptr_print(
        &mut self,
        ptr: PointerValue<'ctx>,
        pointee_ty: BasicTypeEnum<'ctx>,
    ) -> Result<()> {
        let name = self.struct_name_by_type(pointee_ty)?;
        let printer = self.box_struct_printer(&name)?;
        let function = self.current_function()?;
        let null_bb = self.context.append_basic_block(function, "box_print_null");
        let val_bb = self.context.append_basic_block(function, "box_print_val");
        let end_bb = self.context.append_basic_block(function, "box_print_end");
        let is_null = self.builder.build_is_null(ptr, "box_print_isnull").unwrap();
        self.builder.build_conditional_branch(is_null, null_bb, val_bb).unwrap();
        self.builder.position_at_end(null_bb);
        self.emit_printf_text("null")?;
        self.builder.build_unconditional_branch(end_bb).unwrap();
        self.builder.position_at_end(val_bb);
        self.builder.build_call(printer, &[ptr.into()], "box_print_call").unwrap();
        self.builder.build_unconditional_branch(end_bb).unwrap();
        self.builder.position_at_end(end_bb);
        Ok(())
    }

    /// 嵌套 Box 指针的判空递归打印:每层判空(null 输出 `null`),
    /// 到最内层调用结构体打印机;单层退化为 `emit_box_ptr_print`。
    pub(super) fn emit_box_nest_print(
        &mut self,
        ptr: PointerValue<'ctx>,
        nest: BoxNest<'ctx>,
    ) -> Result<()> {
        if nest.depth <= 1 {
            return self.emit_box_ptr_print(ptr, nest.ultimate);
        }
        let function = self.current_function()?;
        let null_bb = self.context.append_basic_block(function, "box_nest_null");
        let val_bb = self.context.append_basic_block(function, "box_nest_val");
        let end_bb = self.context.append_basic_block(function, "box_nest_end");
        let is_null = self.builder.build_is_null(ptr, "box_nest_isnull").unwrap();
        self.builder.build_conditional_branch(is_null, null_bb, val_bb).unwrap();
        self.builder.position_at_end(null_bb);
        self.emit_printf_text("null")?;
        self.builder.build_unconditional_branch(end_bb).unwrap();
        self.builder.position_at_end(val_bb);
        let ptr_ty: BasicTypeEnum<'ctx> = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .into();
        let inner = self
            .builder
            .build_load(ptr_ty, ptr, "box_nest_inner")
            .unwrap()
            .into_pointer_value();
        self.emit_box_nest_print(
            inner,
            BoxNest {
                ultimate: nest.ultimate,
                depth: nest.depth - 1,
            },
        )?;
        self.builder.build_unconditional_branch(end_bb).unwrap();
        self.builder.position_at_end(end_bb);
        Ok(())
    }

    /// 结构体 Box(含嵌套)字段的递归打印:null 输出 `null`,非空逐层解引用后递归。
    pub(super) fn emit_box_field_print(
        &mut self,
        field_ptr: PointerValue<'ctx>,
        info: &StructFieldInfo<'ctx>,
    ) -> Result<()> {
        let nest = self
            .box_nest_of_ast(&info.ast_ty)?
            .ok_or_else(|| HuziError::new_global("print() does not support this field type"))?;
        let ptr = self.builder.build_load(info.ty, field_ptr, "box_field_ptr").unwrap().into_pointer_value();
        self.emit_box_nest_print(ptr, nest)
    }

    /// `print(box_var)` / `print(box_field)` / `print(box(...))` / `print(null)`:
    /// Box 与 null 实参则打印并返回 true;其它返回 false(调用方走常规路径)。
    pub(super) fn try_emit_box_arg(
        &mut self,
        arg: &Expr,
        format_string: &mut String,
        args: &mut Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Result<bool> {
        if Self::is_null_expr(arg) {
            self.flush_print_chunk(format_string, args)?;
            self.emit_printf_text("null")?;
            return Ok(true);
        }
        if !self.is_box_expr(arg) {
            return Ok(false);
        }
        self.flush_print_chunk(format_string, args)?;
        if let Expr::Ident(name) = arg {
            let slot = self.scope_lookup(name).ok_or_else(|| self.unknown_variable_error(name))?;
            let nest = slot.box_inner.ok_or_else(|| HuziError::new_global("print() cannot determine the Box pointee type"))?;
            let ptr = self.builder.build_load(slot.ty, slot.ptr, "box_print_ptr").unwrap().into_pointer_value();
            self.emit_box_nest_print(ptr, nest)?;
            return Ok(true);
        }
        if let Expr::FieldAccess(_) = arg {
            let nest = self.box_nest_of_expr(arg).ok_or_else(|| HuziError::new_global("print() cannot determine the Box pointee type"))?;
            let (ptr_to_box, box_ty) = self.compile_addr(arg)?;
            let ptr = self.builder.build_load(box_ty, ptr_to_box, "box_print_ptr").unwrap().into_pointer_value();
            self.emit_box_nest_print(ptr, nest)?;
            return Ok(true);
        }
        if let Expr::BoxAlloc(inner) = arg {
            let (ptr_val, nest) = self.compile_box_alloc(inner, None)?;
            self.emit_box_nest_print(ptr_val.into_pointer_value(), nest)?;
            return Ok(true);
        }
        Err(HuziError::new_global("print() does not support this Box value; print its fields instead"))
    }
}
