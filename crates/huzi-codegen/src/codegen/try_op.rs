use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    /// `expr?` — Result 解包:成功产出 `value` 字段,失败提前返回整个
    /// Result 值。提前返回经 emit_defers/Box 清理,与显式 `return`
    /// 共用同一出口语义。
    ///
    /// 操作数按字段名(`ok`/`value`/`err`)鸭子类型校验:`ok` 必须为
    /// bool;所在函数的返回类型必须与操作数类型一致,否则编译错误。
    pub(super) fn compile_try(
        &mut self,
        expr: &TryExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        static SHAPE_MSG: &str =
            "?: 操作数必须是 Result<...>(结构体需含 ok/value/err 字段)";

        let value = self.compile_expr(&expr.inner)?;
        let result_val = match value {
            inkwell::values::BasicValueEnum::StructValue(sv) => sv,
            _ => return Err(HuziError::new_global(SHAPE_MSG)),
        };

        let (ok_idx, value_idx) = {
            let (_, fields) = match self.struct_def_by_type(result_val.get_type().into()) {
                Some(def) => def,
                None => return Err(HuziError::new_global(SHAPE_MSG)),
            };
            let idx_of = |name: &str| fields.iter().position(|f| f.name == name);
            let (ok_idx, value_idx) = match (idx_of("ok"), idx_of("value"), idx_of("err")) {
                (Some(ok_idx), Some(value_idx), Some(_)) => (ok_idx, value_idx),
                _ => return Err(HuziError::new_global(SHAPE_MSG)),
            };
            let ok_is_bool = fields[ok_idx].ty.is_int_type()
                && fields[ok_idx].ty.into_int_type().get_bit_width() == 1;
            if !ok_is_bool {
                return Err(HuziError::new_global("?: Result 的 ok 字段必须为 bool"));
            }
            // RFC §3.6 限制:value 字段为 Box<T> 时提前返回无法安全
            // 释放该 Box,规范明确"暂不支持",编译期直接拒绝。
            // LLVM 层面 Box 与指针同形,须按 AST 字段类型判断。
            if matches!(fields[value_idx].ast_ty, Type::Box(_)) {
                return Err(HuziError::new_global(
                    "?: Result 的 value 字段不支持 Box<T>(RFC §3.6:暂不支持)",
                ));
            }
            (ok_idx, value_idx)
        };

        let ret_ty = self
            .current_function()?
            .get_type()
            .get_return_type();
        if ret_ty != Some(result_val.get_type().into()) {
            return Err(HuziError::new_global(
                "?: 所在函数必须返回与操作数相同的 Result<...> 类型",
            ));
        }

        let ok = self
            .builder
            .build_extract_value(result_val, ok_idx as u32, "res_ok")
            .unwrap()
            .into_int_value();

        let function = self.current_function()?;
        let cont_bb = self.context.append_basic_block(function, "try_cont");
        let err_bb = self.context.append_basic_block(function, "try_err");
        self.builder
            .build_conditional_branch(ok, cont_bb, err_bb)
            .unwrap();

        self.builder.position_at_end(err_bb);
        self.emit_defers()?;
        self.emit_release_active_boxes(None)?;
        self.builder.build_return(Some(&result_val)).unwrap();

        self.builder.position_at_end(cont_bb);
        let unwrapped = self
            .builder
            .build_extract_value(result_val, value_idx as u32, "try_val")
            .unwrap();
        Ok(unwrapped)
    }
}
