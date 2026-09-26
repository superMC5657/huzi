//! 结构体字面量构造(自 `aggregates.rs` 纯搬移,零逻辑变化)。

use super::super::{CodeGen, StructFieldInfo};
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    pub(in crate::codegen) fn compile_struct_literal(
        &mut self,
        expr: &StructLiteralExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let (struct_ty, fields) = self
            .structs
            .get(&expr.name)
            .cloned()
            .ok_or_else(|| {
                let mut message = format!("Unknown struct: {}", expr.name);
                if let Some(hint) =
                    huzi_error::did_you_mean(&expr.name, self.structs.keys().map(|s| s.as_str()))
                {
                    message.push_str(&format!("\n  help: {}", hint));
                }
                self.with_current_position(HuziError::new_global(message))
            })?;

        self.check_struct_literal_fields(expr, &fields)?;

        let tmp = self.build_alloca(struct_ty.into(), "struct_val")?;
        for (field_name, field_expr) in &expr.fields {
            let (index, info) = fields
                .iter()
                .enumerate()
                .find(|(_, info)| info.name == *field_name)
                .unwrap();
            // Box 字段先做 `box`/`null` 的 AST 校验(指针层面无法区分内外层)。
            self.check_box_assignable(field_expr, &info.ast_ty)?;
            // Map 字段校验键值特化(`Map<str,str>` 与 `Map` 不互通)。
            self.check_map_field_assignable(field_expr, &info.ast_ty)?;
            let value = self.compile_expr(field_expr)?;
            let value = self.coerce_value(info.ty, value)?;
            if Self::is_weak_ast(&info.ast_ty)
                && !matches!(field_expr, Expr::Null)
                && value.is_pointer_value()
            {
                self.emit_retain_weak(value.into_pointer_value())?;
            }
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

    /// 校验结构体字面量的字段完备性:无重复字段、无未知字段、无缺失字段。
    /// (自 `compile_struct_literal` 提炼,三段校验逐行原样搬移,零逻辑变化。)
    fn check_struct_literal_fields(
        &self,
        expr: &StructLiteralExpr,
        fields: &[StructFieldInfo<'ctx>],
    ) -> Result<()> {
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
                let mut message =
                    format!("Struct '{}' has no field '{}'", expr.name, name);
                if let Some(hint) = huzi_error::did_you_mean(
                    name,
                    fields.iter().map(|info| info.name.as_str()),
                ) {
                    message.push_str(&format!("\n  help: {}", hint));
                }
                return Err(self.with_current_position(HuziError::new_global(message)));
            }
        }
        for info in fields {
            if !expr.fields.iter().any(|(n, _)| n == &info.name) {
                return Err(HuziError::new_global(format!(
                    "Missing field '{}' in struct literal for '{}'",
                    info.name, expr.name
                )));
            }
        }
        Ok(())
    }
}
