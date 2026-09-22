use super::{CodeGen, StructFieldInfo};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    // ==================== 作用域辅助函数 ====================

    /// 将 Windows 控制台切换至 UTF-8 代码页（65001），以便 `printf` 能正确显示中文及其他非 ASCII 字符。
    /// 字符串字面量以 UTF-8 字节存储；若不切换，控制台会用其默认代码页（如 GBK）解码从而导致乱码。
    /// 在非 Windows 平台为空操作。
    pub(super) fn emit_console_utf8_setup(&mut self) {
        if !cfg!(windows) {
            return;
        }
        let set_cp_fn = self.module.get_function("SetConsoleOutputCP").unwrap();
        let utf8_cp = self.context.i32_type().const_int(65001, false);
        self.builder
            .build_call(set_cp_fn, &[utf8_cp.into()], "console_utf8")
            .unwrap();
    }

    pub(super) fn compile_print(
        &mut self,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.is_empty() {
            let empty_str = unsafe { self.builder.build_global_string("", "empty_str").unwrap() };
            let printf_fn = self.module.get_function("printf").unwrap();
            let call = self
                .builder
                .build_call(
                    printf_fn,
                    &[empty_str.as_pointer_value().into()],
                    "print_empty",
                )
                .unwrap();
            return Ok(call.try_as_basic_value().unwrap_left());
        }

        let mut format_string = String::new();
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();

        for arg in arguments.iter() {
            // Box 与 null 直接打印:null 输出 `null`,Box 递归展开字段。
            if self.try_emit_box_arg(arg, &mut format_string, &mut args)? {
                continue;
            }
            // vec 实参走运行期循环打印(先落盘待定的标量片段)。
            if self.try_emit_vec_arg(arg, &mut format_string, &mut args)? {
                continue;
            }
            // 结构体实参走编译期字段展开(同上先落盘)。
            if self.try_emit_struct_arg(arg, &mut format_string, &mut args)? {
                continue;
            }
            let value = self.compile_expr(arg)?;
            // 函数返回的结构体等无 AST 线索的值,按编译后类型兜底。
            if let inkwell::values::BasicValueEnum::StructValue(sv) = value {
                let ty = sv.get_type();
                if !self.is_tuple_type(ty) && self.struct_def_by_type(ty.into()).is_some() {
                    self.flush_print_chunk(&mut format_string, &mut args)?;
                    self.emit_struct_value(value)?;
                    continue;
                }
            }
            self.format_print_value(value, &mut format_string, &mut args)?;
        }

        format_string.push('\n');
        Ok(self.call_printf(&format_string, args))
    }

    /// vec 实参则打印并返回 true;非 vec 返回 false(调用方走常规路径)。
    fn try_emit_vec_arg(
        &mut self,
        arg: &Expr,
        format_string: &mut String,
        args: &mut Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>,
    ) -> Result<bool> {
        if let Expr::Ident(name) = arg {
            if let Some(slot) = self.scope_lookup(name) {
                if Self::is_vec_slot(&slot) {
                    self.flush_print_chunk(format_string, args)?;
                    let (data, len, elem_ty) = self.vec_print_parts(name)?;
                    self.emit_vec_loop(data, len, elem_ty)?;
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if let Expr::VecEmpty(elem_ast_ty) = arg {
            self.flush_print_chunk(format_string, args)?;
            let elem_ty = self.type_to_llvm(elem_ast_ty)?;
            let vec_val = self.compile_vec_empty_value(elem_ast_ty)?;
            self.emit_vec_value(vec_val, elem_ty)?;
            return Ok(true);
        }
        // `print(split(s, d))` 直接打印临时 vec<str>。
        if let Expr::Call(call) = arg {
            if Self::is_split_ctor(call) {
                self.flush_print_chunk(format_string, args)?;
                let vec_val = self.compile_split(&call.arguments)?;
                let str_ty = self.context.ptr_type(inkwell::AddressSpace::default()).into();
                self.emit_vec_value(vec_val, str_ty)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 结构体实参则打印并返回 true;非结构体返回 false。
    fn try_emit_struct_arg(
        &mut self,
        arg: &Expr,
        format_string: &mut String,
        args: &mut Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>,
    ) -> Result<bool> {
        let is_struct = matches!(arg, Expr::StructLiteral(_)) || self.struct_def_of_expr(arg).is_some();
        if !is_struct {
            return Ok(false);
        }
        self.flush_print_chunk(format_string, args)?;
        let (base_ptr, base_ty) = self.compile_addr(arg)?;
        let value = self.builder.build_load(base_ty, base_ptr, "struct_print_val").unwrap();
        self.emit_struct_value(value)?;
        Ok(true)
    }

    /// 将一个待打印的值追加到 printf 格式化字符串和参数列表中，
    /// 并按照 C 可变参数约定进行提升或窄化转换。
    pub(super) fn format_print_value(
        &mut self,
        value: inkwell::values::BasicValueEnum<'ctx>,
        format_string: &mut String,
        args: &mut Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>,
    ) -> Result<()> {
        // 元组按 `(v1, v2, ...)` 打印，每个字段分别进行格式化。
        if let inkwell::values::BasicValueEnum::StructValue(sv) = value {
            if self.is_tuple_type(sv.get_type()) {
                return self.format_tuple_value(sv.into(), format_string, args);
            }
        }

        match value {
            inkwell::values::BasicValueEnum::IntValue(iv)
                if iv.get_type().get_bit_width() == 1 =>
            {
                // 布尔值打印为 true/false。
                let s = self.build_bool_str(iv)?;
                format_string.push_str("%s");
                args.push(s.into());
            }
            inkwell::values::BasicValueEnum::IntValue(iv) => {
                match iv.get_type().get_bit_width() {
                    8 => {
                        // char 按字符打印。
                        format_string.push_str("%c");
                        let c = self
                            .builder
                            .build_int_z_extend(iv, self.context.i32_type(), "char_promote")
                            .unwrap();
                        args.push(c.into());
                    }
                    64 => {
                        // Windows 为 LLP64（long=32 位），%ld 会截断 i64；
                        // %lld（long long）在 Windows/Linux/macOS 均为 64 位。
                        format_string.push_str("%lld");
                        args.push(iv.into());
                    }
                    _ => {
                        format_string.push_str("%d");
                        args.push(iv.into());
                    }
                }
            }
            inkwell::values::BasicValueEnum::FloatValue(fv) => {
                // 可变参数将 float 提升为 double
                let f64_val = if fv.get_type() == self.context.f64_type() {
                    fv
                } else {
                    self.builder
                        .build_float_ext(fv, self.context.f64_type(), "f_promote")
                        .unwrap()
                };
                if fv.get_type() == self.context.f32_type() {
                    format_string.push_str("%g");
                } else {
                    format_string.push_str("%f");
                }
                args.push(f64_val.into());
            }
            inkwell::values::BasicValueEnum::PointerValue(pv) => {
                format_string.push_str("%s");
                args.push(pv.into());
            }
            _ => {
                return Err(HuziError::new_global(
                    "print() does not support this value type",
                ))
            }
        }

        Ok(())
    }

    /// 按名取 vec 打印三件套 (data, len, 元素类型),供 `print(v)` 使用。
    fn vec_print_parts(
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
    fn is_vec_layout(&self, ty: inkwell::types::StructType<'ctx>) -> bool {
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

    /// 单次 printf 调用(无换行),返回调用值。
    pub(super) fn call_printf(
        &mut self,
        format: &str,
        args: Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> BasicValueEnum<'ctx> {
        let printf_fn = self.module.get_function("printf").unwrap();
        let format_ptr = unsafe { self.builder.build_global_string(format, "print_str").unwrap() };
        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> =
            vec![format_ptr.as_pointer_value().into()];
        call_args.extend(args);
        self.builder
            .build_call(printf_fn, &call_args, "print_call")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
    }

    /// 落盘待定的标量片段(无换行),清空缓冲。
    pub(super) fn flush_print_chunk(
        &mut self,
        format_string: &mut String,
        args: &mut Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Result<()> {
        if format_string.is_empty() && args.is_empty() {
            return Ok(());
        }
        let taken: Vec<BasicMetadataValueEnum<'ctx>> = std::mem::take(args);
        let format = std::mem::take(format_string);
        self.call_printf(&format, taken);
        Ok(())
    }

    /// 无参数的纯文本 printf(括号/分隔符/`Name {` 等)。
    pub(super) fn emit_printf_text(&mut self, text: &str) -> Result<()> {
        self.call_printf(text, Vec::new());
        Ok(())
    }

    /// 单个标量/元组值的直接打印(复用 format_print_value,不换行)。
    fn emit_scalar_print(&mut self, value: BasicValueEnum<'ctx>) -> Result<()> {
        let mut format_string = String::new();
        let mut args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
        self.format_print_value(value, &mut format_string, &mut args)?;
        self.call_printf(&format_string, args);
        Ok(())
    }

    /// 任意一等值的打印分发:命名结构体递归,其它走标量/元组路径。
    fn emit_value_print(&mut self, value: BasicValueEnum<'ctx>) -> Result<()> {
        if let BasicValueEnum::StructValue(sv) = value {
            let ty = sv.get_type();
            if self.struct_def_by_type(ty.into()).is_some() {
                return self.emit_struct_value(value);
            }
            if self.is_vec_layout(ty) {
                return Err(HuziError::new_global(
                    "print() does not support nested vec (vec of vec)",
                ));
            }
        }
        self.emit_scalar_print(value)
    }

    /// 按名查结构体定义,无则报错(调用方已确认是结构体值)。
    pub(super) fn struct_name_by_type(&self, ty: BasicTypeEnum<'ctx>) -> Result<String> {
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
    pub(super) fn emit_field_print(
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
    fn emit_vec_loop(
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
