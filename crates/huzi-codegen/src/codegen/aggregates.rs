use super::{CodeGen, EnumInfo, EnumVariantInfo};
use inkwell::types::BasicType;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_struct_literal(
        &mut self,
        expr: &StructLiteralExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let (struct_ty, fields) = self
            .structs
            .get(&expr.name)
            .cloned()
            .ok_or_else(|| HuziError::new_global(format!("Unknown struct: {}", expr.name)))?;

        for (i, (name, _)) in expr.fields.iter().enumerate() {
            if expr.fields[..i].iter().any(|(n, _)| n == name) {
                return Err(HuziError::new_global(format!(
                    "Duplicate field '{}' in struct literal",
                    name
                )));
            }
        }
        for (name, _) in &expr.fields {
            if !fields.iter().any(|info| info.name == *name) {
                return Err(HuziError::new_global(format!(
                    "Struct '{}' has no field '{}'",
                    expr.name, name
                )));
            }
        }
        for info in &fields {
            if !expr.fields.iter().any(|(n, _)| n == &info.name) {
                return Err(HuziError::new_global(format!(
                    "Missing field '{}' in struct literal for '{}'",
                    info.name, expr.name
                )));
            }
        }

        let tmp = self.build_alloca(struct_ty.into(), "struct_val")?;
        for (field_name, field_expr) in &expr.fields {
            let (index, info) = fields
                .iter()
                .enumerate()
                .find(|(_, info)| info.name == *field_name)
                .unwrap();
            let value = self.compile_expr(field_expr)?;
            let value = self.coerce_value(info.ty, value)?;
            let field_ptr = self
                .builder
                .build_struct_gep(struct_ty, tmp, index as u32, "field_ptr")
                .unwrap();
            self.builder.build_store(field_ptr, value).unwrap();
        }

        let loaded = self
            .builder
            .build_load(struct_ty, tmp, "struct_load")
            .unwrap();
        Ok(loaded)
    }

    // ==================== Enum Functions ====================

    pub(super) fn compile_enum_construct(
        &mut self,
        expr: &EnumConstructExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        // `mod::fn(args)` 与 `Enum::Variant(args)` 语法相同:首个路径段是
        // 已导入模块时按函数调用处理。内置模块(m 由 add_module 注册但无
        // AST)回退到非限定名走 builtin 调度;文件模块用限定名查函数表。
        if let Some(m) = self.modules.iter().find(|m| m.name == expr.enum_name) {
            let callee_name = if m.program.is_none() {
                expr.variant.clone()
            } else {
                format!("{}::{}", m.name, expr.variant)
            };
            return self.compile_call(&CallExpr {
                callee: Box::new(Expr::Ident(callee_name)),
                arguments: expr.args.clone(),
            });
        }

        let (info, vinfo) = self.resolve_enum_variant(expr)?;

        let enum_st = match info.llvm {
            None => {
                // Simple enum: the value is the tag itself.
                if !expr.args.is_empty() {
                    return Err(HuziError::new_global(format!(
                        "Unit variant '{}::{}' takes no arguments",
                        expr.enum_name, expr.variant
                    )));
                }
                return Ok(self
                    .context
                    .i32_type()
                    .const_int(vinfo.tag as u64, false)
                    .into());
            }
            Some(st) => st,
        };

        self.build_data_enum_value(expr, &info, &vinfo, enum_st)
    }

    /// Look up the enum and the variant named by an `Enum::Variant` expr.
    fn resolve_enum_variant(
        &self,
        expr: &EnumConstructExpr,
    ) -> Result<(EnumInfo<'ctx>, EnumVariantInfo<'ctx>)> {
        let info = self
            .enums
            .get(&expr.enum_name)
            .cloned()
            .ok_or_else(|| HuziError::new_global(format!("Unknown enum: {}", expr.enum_name)))?;
        let vinfo = info
            .variants
            .iter()
            .find(|v| v.name == expr.variant)
            .cloned()
            .ok_or_else(|| {
                HuziError::new_global(format!(
                    "Enum '{}' has no variant '{}'",
                    expr.enum_name, expr.variant
                ))
            })?;
        Ok((info, vinfo))
    }

    /// Build `{ i32 tag, payload union }` for a data-carrying enum variant:
    /// check arity, store the discriminant, store the payload fields, and
    /// load the finished value.
    fn build_data_enum_value(
        &mut self,
        expr: &EnumConstructExpr,
        info: &EnumInfo<'ctx>,
        vinfo: &EnumVariantInfo<'ctx>,
        enum_st: inkwell::types::StructType<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let expected = vinfo.ast_payloads.len();
        if expr.args.len() != expected {
            if expected == 0 {
                return Err(HuziError::new_global(format!(
                    "Unit variant '{}::{}' takes no arguments",
                    expr.enum_name, expr.variant
                )));
            }
            if expected == 1 {
                return Err(HuziError::new_global(format!(
                    "Variant '{}::{}' expects exactly 1 argument",
                    expr.enum_name, expr.variant
                )));
            }
            return Err(HuziError::new_global(format!(
                "Variant '{}::{}' expects {} arguments, got {}",
                expr.enum_name,
                expr.variant,
                expected,
                expr.args.len()
            )));
        }

        let payload_union = info.payload_union.unwrap();
        let tmp = self.build_alloca(enum_st.into(), "enum_val")?;

        // Store the discriminant in field 0.
        let tag_ptr = self
            .builder
            .build_struct_gep(enum_st, tmp, 0, "enum_tag_ptr")
            .unwrap();
        self.builder
            .build_store(
                tag_ptr,
                self.context.i32_type().const_int(vinfo.tag as u64, false),
            )
            .unwrap();

        // Store the payload into the variant's slot of the union in field 1.
        // Single-payload variants keep the bare value; multi-payload ones
        // fill the anonymous field struct member by member.
        if let Some(payload_ty) = vinfo.payload {
            let union_ptr = self
                .builder
                .build_struct_gep(enum_st, tmp, 1, "enum_payload_ptr")
                .unwrap();
            let slot_ptr = self
                .builder
                .build_struct_gep(
                    payload_union,
                    union_ptr,
                    vinfo.payload_slot.unwrap(),
                    "enum_slot_ptr",
                )
                .unwrap();
            if vinfo.ast_payloads.len() == 1 {
                let arg = self.compile_expr(&expr.args[0])?;
                let arg = self.coerce_value(payload_ty, arg)?;
                self.builder.build_store(slot_ptr, arg).unwrap();
            } else {
                self.store_multi_payload_fields(payload_ty, slot_ptr, &expr.args)?;
            }
        }

        let loaded = self
            .builder
            .build_load(enum_st, tmp, "enum_load")
            .unwrap();
        Ok(loaded)
    }

    /// Store each constructor argument into its field of a multi-payload
    /// variant's anonymous field struct (`slot_ptr` points at the struct).
    fn store_multi_payload_fields(
        &mut self,
        payload_ty: inkwell::types::BasicTypeEnum<'ctx>,
        slot_ptr: inkwell::values::PointerValue<'ctx>,
        args: &[Expr],
    ) -> Result<()> {
        let field_st = payload_ty.into_struct_type();
        for (i, arg_expr) in args.iter().enumerate() {
            let field_ty = field_st.get_field_type_at_index(i as u32).unwrap();
            let arg = self.compile_expr(arg_expr)?;
            let arg = self.coerce_value(field_ty, arg)?;
            let field_ptr = self
                .builder
                .build_struct_gep(field_st, slot_ptr, i as u32, "enum_field_ptr")
                .unwrap();
            self.builder.build_store(field_ptr, arg).unwrap();
        }
        Ok(())
    }

    pub(super) fn compile_array_index(
        &mut self,
        expr: &huzi_ast::ArrayIndexExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        // vec 下标读走动态长度路径。
        if let huzi_ast::Expr::Ident(name) = &*expr.array {
            if let Some(slot) = self.scope_lookup(name) {
                if Self::is_vec_slot(&slot) {
                    return self.compile_vec_index_load(name, &expr.index);
                }
            }
        }
        let array_ptr = self.compile_expr(&expr.array)?;
        let array_ptr_val = if array_ptr.is_pointer_value() {
            array_ptr.into_pointer_value()
        } else {
            return Err(HuziError::new_global("Indexed value is not an array"));
        };

        let elem_type = self.resolve_elem_type(&expr.array, None)?;

        let index_val = self.compile_expr(&expr.index)?;
        let index_i32 = self.coerce_index(index_val)?;

        if self.is_string_index(&expr.array, elem_type)? {
            self.emit_str_bounds_check(array_ptr_val, index_i32)?;
        } else {
            self.emit_bounds_check(&expr.array, index_i32)?;
        }

        // Build GEP to get element pointer
        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, array_ptr_val, &[index_i32], "elem_ptr")
                .unwrap()
        };

        // Load the element value
        let loaded = self
            .builder
            .build_load(elem_type, elem_ptr, "load_elem")
            .unwrap();

        Ok(loaded)
    }

    pub(super) fn compile_array_literal(
        &mut self,
        elements: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if elements.is_empty() {
            return Err(HuziError::new_global("Empty array literal not supported"));
        }

        // Compile all elements
        let mut elem_values = Vec::new();
        for elem in elements {
            let val = self.compile_expr(elem)?;
            elem_values.push(val);
        }

        // Get element type from first element
        let elem_type = elem_values[0].get_type();

        // Create array type
        let array_type = elem_type.array_type(elements.len() as u32);

        // Allocate space for the array
        let array_ptr = self.build_alloca(array_type.into(), "array")?;

        // Store each element
        for (i, val) in elem_values.iter().enumerate() {
            let val = self.coerce_value(elem_type, *val)?;
            let index = self.context.i32_type().const_int(i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(elem_type, array_ptr, &[index], "elem_ptr")
                    .unwrap()
            };
            self.builder.build_store(elem_ptr, val).unwrap();
        }

        Ok(array_ptr.into())
    }

    pub(super) fn compile_if_expr(
        &mut self,
        expr: &IfExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let cond_value = self.compile_expr(&expr.condition)?;
        let cond = self.to_i1(cond_value)?;

        let function = self.current_function()?;
        let then_block = self.context.append_basic_block(function, "ifex_then");
        let else_block = self.context.append_basic_block(function, "ifex_else");
        let merge_block = self.context.append_basic_block(function, "ifex_merge");

        self.builder
            .build_conditional_branch(cond, then_block, else_block)
            .unwrap();

        self.builder.position_at_end(then_block);
        let then_val = self.compile_block_value(&expr.then_branch)?;
        let result_type = then_val.get_type();
        let result_ptr = self.build_alloca(result_type, "ifex_val")?;
        self.builder.build_store(result_ptr, then_val).unwrap();
        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        self.builder.position_at_end(else_block);
        let else_val = self.compile_block_value(&expr.else_branch)?;
        let else_val = self.coerce_value(result_type, else_val)?;
        self.builder.build_store(result_ptr, else_val).unwrap();
        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        self.builder.position_at_end(merge_block);
        let result = self
            .builder
            .build_load(result_type, result_ptr, "ifex_load")
            .unwrap();

        Ok(result)
    }

    // ==================== Output ====================

}
