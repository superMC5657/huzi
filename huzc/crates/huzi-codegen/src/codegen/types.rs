use super::{CodeGen, EnumInfo, EnumVariantInfo, StructFieldInfo};
use inkwell::AddressSpace;
use inkwell::values::PointerValue;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn register_struct_names(&mut self, defs: &[StructDef]) -> Result<()> {
        for def in defs {
            if self.structs.contains_key(&def.name) || self.enums.contains_key(&def.name) {
                return Err(HuziError::new_global(format!(
                    "Duplicate type definition: {}",
                    def.name
                )));
            }
            let st = self.context.opaque_struct_type(&def.name);
            self.structs.insert(def.name.clone(), (st, Vec::new()));
        }
        Ok(())
    }

    /// Pass 2 (结构体)：解析字段类型并设置结构体体部。
    pub(super) fn resolve_struct_bodies(&mut self, defs: &[StructDef]) -> Result<()> {
        for def in defs {
            let mut fields: Vec<StructFieldInfo<'ctx>> = Vec::new();
            for f in &def.fields {
                if f.field_type == Type::Named(def.name.clone()) {
                    return Err(HuziError::new_global(format!(
                        "Struct '{}' cannot contain itself by value",
                        def.name
                    )));
                }
                let ty = self.type_to_llvm(&f.field_type)?;
                if fields.iter().any(|info| info.name == f.name) {
                    return Err(HuziError::new_global(format!(
                        "Duplicate field '{}' in struct '{}'",
                        f.name, def.name
                    )));
                }
                fields.push(StructFieldInfo {
                    name: f.name.clone(),
                    ty,
                    ast_ty: f.field_type.clone(),
                });
            }

            let (st, slot) = self.structs.get_mut(&def.name).unwrap();
            let field_types: Vec<inkwell::types::BasicTypeEnum<'ctx>> =
                fields.iter().map(|info| info.ty).collect();
            st.set_body(&field_types, false);
            *slot = fields;
        }

        Ok(())
    }

    /// Pass 1 (枚举)：创建布局并插入占位符，以便负载可通过 type_to_llvm 引用任意枚举（无论定义顺序）。
    pub(super) fn register_enum_names(&mut self, defs: &[EnumDef]) -> Result<()> {
        for def in defs {
            if self.structs.contains_key(&def.name) || self.enums.contains_key(&def.name) {
                return Err(HuziError::new_global(format!(
                    "Duplicate type definition: {}",
                    def.name
                )));
            }

            let is_data = def.variants.iter().any(|v| !v.payloads.is_empty());
            let (llvm, payload_union) = if is_data {
                let union_st = self.context.opaque_struct_type(&format!("{}.payload", def.name));
                let enum_st = self.context.opaque_struct_type(&def.name);
                (Some(enum_st), Some(union_st))
            } else {
                (None, None)
            };

            self.enums.insert(
                def.name.clone(),
                EnumInfo {
                    name: def.name.clone(),
                    variants: Vec::new(),
                    llvm,
                    payload_union,
                },
            );
        }
        Ok(())
    }

    /// Pass 2 (枚举)：解析负载类型，设置枚举体部与变体信息。
    pub(super) fn resolve_enum_bodies(&mut self, defs: &[EnumDef]) -> Result<()> {
        for def in defs {
            let mut variants: Vec<EnumVariantInfo<'ctx>> = Vec::new();
            let mut payload_slot = 0u32;
            for v in &def.variants {
                if variants.iter().any(|info| info.name == v.name) {
                    return Err(HuziError::new_global(format!(
                        "Duplicate variant '{}' in enum '{}'",
                        v.name, def.name
                    )));
                }

                let (payload, ast_payloads, slot) = if v.payloads.is_empty() {
                    (None, Vec::new(), None)
                } else if v.payloads.len() == 1 {
                    let ty = self.type_to_llvm(&v.payloads[0])?;
                    let slot = payload_slot;
                    payload_slot += 1;
                    (Some(ty), vec![v.payloads[0].clone()], Some(slot))
                } else {
                    // 多负载变体将匿名字段结构体存储为
                    // 其唯一的联合体成员。
                    let mut field_types = Vec::with_capacity(v.payloads.len());
                    for t in &v.payloads {
                        field_types.push(self.type_to_llvm(t)?);
                    }
                    let field_st = self.context.struct_type(&field_types, false);
                    let slot = payload_slot;
                    payload_slot += 1;
                    (Some(field_st.into()), v.payloads.clone(), Some(slot))
                };

                variants.push(EnumVariantInfo {
                    name: v.name.clone(),
                    tag: variants.len() as u32,
                    payload,
                    ast_payloads,
                    payload_slot: slot,
                });
            }

            let info = self.enums.get_mut(&def.name).unwrap();
            info.variants = variants;

            if let (Some(enum_st), Some(union_st)) = (info.llvm, info.payload_union) {
                let payload_types: Vec<inkwell::types::BasicTypeEnum<'ctx>> = info
                    .variants
                    .iter()
                    .filter_map(|v| v.payload)
                    .collect();
                union_st.set_body(&payload_types, false);
                let i32_ty = self.context.i32_type().into();
                enum_st.set_body(&[i32_ty, union_st.into()], false);
            }
        }

        Ok(())
    }

    /// 根据 LLVM 结构体类型查找已注册的携带数据枚举。
    pub(super) fn enum_data_by_type(
        &self,
        ty: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Option<&EnumInfo<'ctx>> {
        let st = match ty {
            inkwell::types::BasicTypeEnum::StructType(st) => st,
            _ => return None,
        };
        self.enums
            .values()
            .find(|info| matches!(info.llvm, Some(llvm_st) if llvm_st == st))
    }

    pub(super) fn resolve_elem_type(
        &self,
        array_expr: &Expr,
        fallback: Option<inkwell::types::BasicTypeEnum<'ctx>>,
    ) -> Result<inkwell::types::BasicTypeEnum<'ctx>> {
        if let Expr::Ident(name) = array_expr {
            if let Some(slot) = self.scope_lookup(name) {
                if let Some(elem) = slot.elem {
                    return Ok(elem);
                }
            }
        }
        if let Expr::ArrayLiteral(elements) = array_expr {
            if let Some(first) = elements.first() {
                if let Some(ty) = self.infer_expr_type(first) {
                    return Ok(ty);
                }
            }
        }
        if let Expr::FieldAccess(fa) = array_expr {
            if let Some((_, fields)) = self.struct_def_of_expr(&fa.base) {
                if let Some(info) = fields.iter().find(|info| info.name == fa.field) {
                    if let Type::Array(elem, _) = &info.ast_ty {
                        return self.type_to_llvm(elem);
                    }
                    if let Type::Applied(name, args) = &info.ast_ty {
                        if name == "vec" && !args.is_empty() {
                            return self.type_to_llvm(&args[0]);
                        }
                    }
                }
            }
        }
        if let Expr::Literal(Literal::String(_)) = array_expr {
            return Ok(self.context.i8_type().into());
        }
        fallback.ok_or_else(|| HuziError::new_global("Cannot determine array element type"))
    }

    /// 静态推断表达式的 LLVM 类型(不生成任何指令),用于数组字面量
    /// 的元素类型推断;推断不出(如结构体字面量元素)返回 None。
    pub(super) fn infer_expr_type(
        &self,
        expr: &Expr,
    ) -> Option<inkwell::types::BasicTypeEnum<'ctx>> {
        match expr {
            Expr::Literal(lit) => Some(match lit {
                Literal::Int(n) => {
                    if *n >= i32::MIN as i64 && *n <= i32::MAX as i64 {
                        self.context.i32_type().into()
                    } else {
                        self.context.i64_type().into()
                    }
                }
                Literal::Float(_) => self.context.f64_type().into(),
                Literal::Bool(_) => self.context.bool_type().into(),
                Literal::String(_) => self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into(),
                Literal::Char(_) => self.context.i8_type().into(),
            }),
            Expr::Ident(name) => self.scope_lookup(name).map(|slot| slot.ty),
            _ => None,
        }
    }

    pub(super) fn coerce_index(
        &self,
        index_val: inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>> {        if !index_val.is_int_value() {
            return Err(HuziError::new_global("Array index must be an integer"));
        }
        let int_val = index_val.into_int_value();
        if int_val.get_type().get_bit_width() == 32 {
            Ok(int_val)
        } else if int_val.get_type().get_bit_width() < 32 {
            Ok(self
                .builder
                .build_int_s_extend(int_val, self.context.i32_type(), "index_i32")
                .unwrap())
        } else {
            Ok(self
                .builder
                .build_int_truncate(int_val, self.context.i32_type(), "index_i32")
                .unwrap())
        }
    }

    pub(super) fn type_to_llvm(&self, ty: &Type) -> Result<inkwell::types::BasicTypeEnum<'ctx>> {
        match ty {
            Type::I32 | Type::U32 => Ok(self.context.i32_type().into()),
            Type::I64 | Type::U64 => Ok(self.context.i64_type().into()),
            Type::F32 => Ok(self.context.f32_type().into()),
            Type::F64 => Ok(self.context.f64_type().into()),
            Type::Bool => Ok(self.context.bool_type().into()),
            Type::Char => Ok(self.context.i8_type().into()),
            Type::Str => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            Type::Unit => Ok(self.context.i32_type().into()),
            // 数组退化为指针（LLVM 不透明指针使两者等价）；元素类型在 VarSlot 中跟踪。
            Type::Array(_, _) => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            // `Box<T>` 降阶为指向 T 的 LLVM 存储的裸指针；因此持有 Box 字段的结构体
            // 具有固定大小，且通过 Box 形成的任意引用循环均合法（参见 check_type_cycles）。
            // 嵌套的 `Box<Box<..>>` 同样是裸指针：每层堆单元持有下一层的指针，
            // 因而内存布局保持扁平。
            // T 为具名结构体或标量（`i32`/`i64`/`f64`/`bool`/`str`，及 `u32`/`u64`/`f32`/`char`）；
            // 拒绝其他复合类型。
            Type::Box(inner) => {
                // 嵌套层直接放行(指针套指针);单层须为结构体或标量。
                if matches!(&**inner, Type::Box(_)) {
                    self.type_to_llvm(inner)?;
                    return Ok(self.context.ptr_type(AddressSpace::default()).into());
                }
                let inner_ty = self.type_to_llvm(inner)?;
                if self.is_box_pointee(inner_ty) || Self::is_box_scalar(inner, inner_ty) {
                    return Ok(self.context.ptr_type(AddressSpace::default()).into());
                }
                return Err(HuziError::new_global(format!(
                    "Box<T> requires a named struct or scalar type (i32/i64/f64/bool/str) (found '{}')",
                    inner
                )));
            }
            // 元组是字面量结构体：LLVM 按结构比对它们，因此两个 `(i32, str)` 元组类型始终相等。
            Type::Tuple(elems) => {
                let mut field_types = Vec::with_capacity(elems.len());
                for elem in elems {
                    field_types.push(self.type_to_llvm(elem)?);
                }
                Ok(self.context.struct_type(&field_types, false).into())
            }
            Type::Named(name) => match name.as_str() {
                "i32" | "u32" => Ok(self.context.i32_type().into()),
                "i64" | "u64" => Ok(self.context.i64_type().into()),
                "f32" => Ok(self.context.f32_type().into()),
                "f64" => Ok(self.context.f64_type().into()),
                "bool" => Ok(self.context.bool_type().into()),
                "char" => Ok(self.context.i8_type().into()),
                "str" => Ok(self.context.ptr_type(AddressSpace::default()).into()),
                "map" | "Map" | "HashMap" => Ok(self.vec_struct_type().into()),
                other => {
                    if let Some((st, _)) = self.structs.get(other) {
                        return Ok((*st).into());
                    }
                    if let Some(info) = self.enums.get(other) {
                        // 简单枚举为其 i32 tag；带数据的枚举为带标签的结构体。
                        return Ok(match info.llvm {
                            Some(st) => st.into(),
                            None => self.context.i32_type().into(),
                        });
                    }
                    Err(HuziError::new_global(format!("Unsupported type: {}", other)))
                }
            },
            Type::Generic(g) => Err(HuziError::new_global(format!(
                "Unresolved generic type parameter '{}'",
                g
            ))),
            Type::Applied(name, args) => {
                if name == "vec" || name == "map" || name == "Map" || name == "HashMap" {
                    return Ok(self.vec_struct_type().into());
                }
                let mangled = crate::codegen::generic::mangle_name(name, args);
                if let Some((st, _)) = self.structs.get(&mangled) {
                    return Ok((*st).into());
                }
                Err(HuziError::new_global(format!("Unsupported type: {}", name)))
            }
        }
    }

    /// 将一个值转换为布尔真值的 i1。
    pub(super) fn to_i1(&self, value: inkwell::values::BasicValueEnum<'ctx>) -> Result<inkwell::values::IntValue<'ctx>> {
        match value {
            inkwell::values::BasicValueEnum::IntValue(iv)
                if iv.get_type().get_bit_width() == 1 =>
            {
                Ok(iv)
            }
            inkwell::values::BasicValueEnum::IntValue(iv) => {
                let zero = iv.get_type().const_int(0, false);
                Ok(self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, iv, zero, "to_bool")
                    .unwrap())
            }
            inkwell::values::BasicValueEnum::PointerValue(pv) => Ok(self
                .builder
                .build_is_not_null(pv, "to_bool")
                .unwrap()),
            _ => Err(HuziError::new_global(
                "Value cannot be used as a condition",
            )),
        }
    }

    /// 当属于无损/预期的数值强制转换时，将值转换为目标类型；否则报告类型不匹配错误。
    pub(super) fn coerce_value(
        &self,
        target: inkwell::types::BasicTypeEnum<'ctx>,
        value: inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if value.get_type() == target {
            return Ok(value);
        }

        match (target, value) {
            (inkwell::types::BasicTypeEnum::IntType(it), inkwell::values::BasicValueEnum::IntValue(iv)) => {
                let tw = it.get_bit_width();
                let sw = iv.get_type().get_bit_width();
                if tw > sw {
                    Ok(self
                        .builder
                        .build_int_s_extend(iv, it, "coerce")
                        .unwrap()
                        .into())
                } else if tw < sw {
                    Ok(self
                        .builder
                        .build_int_truncate(iv, it, "coerce")
                        .unwrap()
                        .into())
                } else {
                    Ok(iv.into())
                }
            }
            (inkwell::types::BasicTypeEnum::FloatType(ft), inkwell::values::BasicValueEnum::IntValue(iv)) => {
                Ok(self
                    .builder
                    .build_signed_int_to_float(iv, ft, "coerce")
                    .unwrap()
                    .into())
            }
            (inkwell::types::BasicTypeEnum::IntType(it), inkwell::values::BasicValueEnum::FloatValue(fv)) => {
                Ok(self
                    .builder
                    .build_float_to_signed_int(fv, it, "coerce")
                    .unwrap()
                    .into())
            }
            (inkwell::types::BasicTypeEnum::FloatType(ft), inkwell::values::BasicValueEnum::FloatValue(fv)) => {
                if fv.get_type() == ft {
                    Ok(fv.into())
                } else {
                    Ok(self.builder.build_float_cast(fv, ft, "coerce").unwrap().into())
                }
            }
            (inkwell::types::BasicTypeEnum::IntType(it), inkwell::values::BasicValueEnum::PointerValue(pv))
                if it.get_bit_width() == 64 =>
            {
                Ok(self.builder.build_ptr_to_int(pv, it, "coerce").unwrap().into())
            }
            _ => Err(HuziError::new_global(format!(
                "Type mismatch: expected {}, got {}",
                target,
                value.get_type()
            ))),
        }
    }

    pub(super) fn build_alloca(
        &self,
        ty: inkwell::types::BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>> {
        let function = self.current_function()?;
        let entry = function
            .get_first_basic_block()
            .ok_or_else(|| HuziError::new_global("Function has no entry block"))?;
        let builder = self.context.create_builder();
        // 插入到第一条指令之前，确保 alloca 始终落在入口块的最前端，绝不位于终结指令之后。
        if let Some(first) = entry.get_first_instruction() {
            builder.position_before(&first);
        } else {
            builder.position_at_end(entry);
        }
        builder
            .build_alloca(ty, name)
            .map_err(|_| HuziError::new_global("Failed to build alloca"))
    }

    // ==================== 标准库辅助函数 ====================

    /// 判定是否为句柄传递的容器类型 (Map 或 vec<T>)。
    pub(super) fn is_container_handle_type(ty: &Type) -> bool {
        match ty {
            Type::Named(n) if n == "Map" || n == "HashMap" || n == "map" => true,
            Type::Applied(n, _) if n == "vec" || n == "Map" || n == "HashMap" || n == "map" => true,
            _ => false,
        }
    }

    /// `Box` 标量究极类型判定:整数/浮点直接放行;指针仅当 AST 为
    /// `str` 时放行(数组等其它指针 composites 仍拒绝)。
    pub(super) fn is_box_scalar(
        ast: &Type,
        llvm_ty: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> bool {
        use inkwell::types::BasicTypeEnum as BTE;
        match llvm_ty {
            BTE::IntType(_) | BTE::FloatType(_) => true,
            BTE::PointerType(_) => {
                matches!(ast, Type::Str) || matches!(ast, Type::Named(n) if n == "str")
            }
            _ => false,
        }
    }
}
