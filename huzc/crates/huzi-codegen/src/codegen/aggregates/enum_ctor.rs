//! 枚举构造与变体解析(自 `aggregates.rs` 纯搬移,零逻辑变化)。

use super::super::{CodeGen, EnumInfo, EnumVariantInfo};
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    // ==================== 枚举函数 ====================

    pub(in crate::codegen) fn compile_enum_construct(
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
                type_args: Vec::new(),
            });
        }

        let mangled = format!("{}__{}", expr.enum_name, expr.variant);
        if self.functions.contains_key(&mangled) {
            return self.compile_call(&CallExpr {
                callee: Box::new(Expr::Ident(mangled)),
                arguments: expr.args.clone(),
                type_args: expr.type_args.clone(),
            });
        }

        let (info, vinfo) = self.resolve_enum_variant(expr)?;

        let enum_st = match info.llvm {
            None => {
                // 简单枚举：值即为其 tag 本身。
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

    /// 查找由 `Enum::Variant` 表达式指定的枚举与变体。
    fn resolve_enum_variant(
        &self,
        expr: &EnumConstructExpr,
    ) -> Result<(EnumInfo<'ctx>, EnumVariantInfo<'ctx>)> {
        let info = self
            .enums
            .get(&expr.enum_name)
            .cloned()
            .ok_or_else(|| {
                if self.structs.contains_key(&expr.enum_name) {
                    let mut message = format!(
                        "Type '{}' has no static method or variant '{}'",
                        expr.enum_name, expr.variant
                    );
                    let method_candidates: Vec<&str> = self
                        .functions
                        .keys()
                        .filter_map(|k| {
                            let (ty, meth) = k.rsplit_once("__")?;
                            if ty == expr.enum_name
                                || ty.ends_with(&format!("::{}", expr.enum_name))
                            {
                                Some(meth)
                            } else {
                                None
                            }
                        })
                        .collect();
                    if let Some(hint) =
                        huzi_error::did_you_mean(&expr.variant, method_candidates)
                    {
                        message.push_str(&format!("\n  help: {}", hint));
                    }
                    self.with_current_position(HuziError::new_global(message))
                } else {
                    let mut message = format!("Unknown enum: {}", expr.enum_name);
                    if let Some(hint) = huzi_error::did_you_mean(
                        &expr.enum_name,
                        self.enums.keys().map(|s| s.as_str()),
                    ) {
                        message.push_str(&format!("\n  help: {}", hint));
                    }
                    self.with_current_position(HuziError::new_global(message))
                }
            })?;
        let vinfo = info
            .variants
            .iter()
            .find(|v| v.name == expr.variant)
            .cloned()
            .ok_or_else(|| {
                let mut message = format!(
                    "Enum '{}' has no variant '{}'",
                    expr.enum_name, expr.variant
                );
                if let Some(hint) = huzi_error::did_you_mean(
                    &expr.variant,
                    info.variants.iter().map(|v| v.name.as_str()),
                ) {
                    message.push_str(&format!("\n  help: {}", hint));
                }
                self.with_current_position(HuziError::new_global(message))
            })?;
        Ok((info, vinfo))
    }

    fn check_enum_variant_arity(expr: &EnumConstructExpr, expected: usize) -> Result<()> {
        if expr.args.len() == expected {
            return Ok(());
        }
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
        Err(HuziError::new_global(format!(
            "Variant '{}::{}' expects {} arguments, got {}",
            expr.enum_name,
            expr.variant,
            expected,
            expr.args.len()
        )))
    }

    /// 为携带数据的枚举变体构建 `{ i32 tag, payload_union }`：
    /// 检查实参数量，存储判别码，存储负载字段，并加载构建完成的值。
    fn build_data_enum_value(
        &mut self,
        expr: &EnumConstructExpr,
        info: &EnumInfo<'ctx>,
        vinfo: &EnumVariantInfo<'ctx>,
        enum_st: inkwell::types::StructType<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        Self::check_enum_variant_arity(expr, vinfo.ast_payloads.len())?;

        let payload_union = info.payload_union.unwrap();
        let tmp = self.build_alloca(enum_st.into(), "enum_val")?;

        // 在字段 0 处存储判别码。
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

        // 将负载存入字段 1 联合体中该变体对应的槽位。
        // 单负载变体保留原始裸值；多负载变体逐成员填充匿名字段结构体。
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

    /// 将各个构造函数实参存入多负载变体的匿名字段结构体对应字段中（`slot_ptr` 指向该结构体）。
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
}
