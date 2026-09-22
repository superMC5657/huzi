use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_abs(&mut self, arguments: &[Expr]) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("abs() requires exactly 1 argument"));
        }

        let arg = self.compile_expr(&arguments[0])?;

        if arg.is_int_value() {
            let int_val = arg.into_int_value();
            let zero = int_val.get_type().const_int(0, false);
            let is_neg = self
                .builder
                .build_int_compare(inkwell::IntPredicate::SLT, int_val, zero, "abs_neg")
                .unwrap();
            let negated = self.builder.build_int_neg(int_val, "abs_negated").unwrap();
            let result = self
                .builder
                .build_select(is_neg, negated, int_val, "abs")
                .unwrap();
            Ok(result)
        } else if arg.is_float_value() {
            let float_val = arg.into_float_value();
            let fabs_fn = self.module.get_function("fabs").unwrap();
            let arg_f64 = if float_val.get_type() == self.context.f64_type() {
                float_val
            } else {
                self.builder
                    .build_float_cast(float_val, self.context.f64_type(), "to_f64")
                    .unwrap()
            };
            let result = self
                .builder
                .build_call(fabs_fn, &[arg_f64.into()], "abs_result")
                .unwrap()
                .try_as_basic_value()
                .unwrap_left();
            Ok(result)
        } else {
            Err(HuziError::new_global("abs() requires a numeric argument"))
        }
    }

    /// 单参数 libm 封装函数（sqrt/sin/cos/tan/floor/ceil/round）：
    /// 将参数强制转换为 f64，调用 C 运行时函数，返回 f64。
    pub(super) fn compile_libm_unary(
        &mut self,
        fn_name: &str,
        arguments: &[Expr],
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global(format!(
                "{}() requires exactly 1 argument",
                fn_name
            )));
        }

        let f = self
            .module
            .get_function(fn_name)
            .ok_or_else(|| self.unknown_function_error(fn_name))?;
        let arg = self.compile_expr(&arguments[0])?;
        let arg_f64 = self.to_f64(arg, &format!("{}()", fn_name))?;

        let result = self
            .builder
            .build_call(f, &[arg_f64.into()], "libm_result")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left();

        Ok(result)
    }

    pub(super) fn compile_pow(&mut self, arguments: &[Expr]) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global("pow() requires exactly 2 arguments"));
        }

        let pow_fn = self.module.get_function("pow").unwrap();
        let base = self.compile_expr(&arguments[0])?;
        let exp = self.compile_expr(&arguments[1])?;
        let base_f64 = self.to_f64(base, "pow()")?;
        let exp_f64 = self.to_f64(exp, "pow()")?;

        let result = self
            .builder
            .build_call(pow_fn, &[base_f64.into(), exp_f64.into()], "pow_result")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left();

        Ok(result)
    }

    /// 为数学内置函数将任意数值转换为 f64。
    pub(super) fn to_f64(
        &self,
        arg: inkwell::values::BasicValueEnum<'ctx>,
        fn_name: &str,
    ) -> Result<inkwell::values::FloatValue<'ctx>> {
        match arg {
            inkwell::values::BasicValueEnum::IntValue(iv) => Ok(self
                .builder
                .build_signed_int_to_float(iv, self.context.f64_type(), "to_f64")
                .unwrap()),
            inkwell::values::BasicValueEnum::FloatValue(fv) => {
                if fv.get_type() == self.context.f64_type() {
                    Ok(fv)
                } else {
                    Ok(self
                        .builder
                        .build_float_cast(fv, self.context.f64_type(), "to_f64")
                        .unwrap())
                }
            }
            _ => Err(HuziError::new_global(format!(
                "{} requires a numeric argument",
                fn_name
            ))),
        }
    }
}
