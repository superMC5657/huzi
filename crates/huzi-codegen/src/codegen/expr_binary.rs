use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};


impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_binary(
        &mut self,
        expr: &BinaryExpr,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        // && and || short-circuit; handle them before evaluating operands.
        match expr.operator {
            BinOp::And => return self.compile_short_circuit(&expr.left, &expr.right, true),
            BinOp::Or => return self.compile_short_circuit(&expr.left, &expr.right, false),
            _ => {}
        }

        // Box 判空/判等走指针比较,不进字符串(strcmp)/数值路径。
        if self.is_box_comparison(&expr.left, &expr.right) {
            let left = self.compile_expr(&expr.left)?;
            let right = self.compile_expr(&expr.right)?;
            return self.build_box_compare(
                &expr.operator,
                super::boxed::BoxOperand {
                    expr: &expr.left,
                    value: left,
                },
                super::boxed::BoxOperand {
                    expr: &expr.right,
                    value: right,
                },
            );
        }

        let mut left = self.compile_expr(&expr.left)?;
        let mut right = self.compile_expr(&expr.right)?;

        self.coerce_binary_operands(&mut left, &mut right);

        let value = match expr.operator {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                self.build_arithmetic(&expr.operator, &left, &right)?
            }
            BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                // Data-carrying enums compare by tag first, then by payload
                // fields; mismatched enum types are a compile error.
                if let Some(eq) = self.try_build_data_enum_compare(&expr.operator, &left, &right)? {
                    return Ok(eq);
                }
                if left.is_pointer_value() && right.is_pointer_value() {
                    // 字符串以 `i8*` 表示;两个指针操作数按 strcmp 比较,
                    // 数组变量同样以指针装载,直接比较是无意义的,显式报错。
                    if self.expr_is_array(&expr.left) && self.expr_is_array(&expr.right) {
                        return Err(HuziError::new_global(
                            "Arrays cannot be compared directly; compare elements instead",
                        ));
                    }
                    self.build_string_compare(&expr.operator, &left, &right)?
                } else {
                    let (int_pred, float_pred) = Self::compare_predicates(&expr.operator);
                    self.build_int_or_float_compare(int_pred, float_pred, &left, &right)
                }
            }
            BinOp::And | BinOp::Or => unreachable!("short-circuit handled above"),
        };

        Ok(value)
    }

    /// Mixed int/float: convert the int operand to the float operand's type.
    /// Same-type ints: sign-extend the narrower operand to the wider width.
    fn coerce_binary_operands(
        &mut self,
        left: &mut inkwell::values::BasicValueEnum<'ctx>,
        right: &mut inkwell::values::BasicValueEnum<'ctx>,
    ) {
        if left.is_float_value() && right.is_int_value() {
            let float_ty = left.into_float_value().get_type();
            let int_val = right.into_int_value();
            *right = self
                .builder
                .build_signed_int_to_float(int_val, float_ty, "to_float")
                .unwrap()
                .into();
        } else if left.is_int_value() && right.is_float_value() {
            let float_ty = right.into_float_value().get_type();
            let int_val = left.into_int_value();
            *left = self
                .builder
                .build_signed_int_to_float(int_val, float_ty, "to_float")
                .unwrap()
                .into();
        } else if left.is_int_value() && right.is_int_value() {
            let lw = left.into_int_value().get_type().get_bit_width();
            let rw = right.into_int_value().get_type().get_bit_width();
            if lw < rw {
                let target = right.into_int_value().get_type();
                *left = self
                    .builder
                    .build_int_s_extend(left.into_int_value(), target, "widen")
                    .unwrap()
                    .into();
            } else if rw < lw {
                let target = left.into_int_value().get_type();
                *right = self
                    .builder
                    .build_int_s_extend(right.into_int_value(), target, "widen")
                    .unwrap()
                    .into();
            }
        }
    }

    fn build_arithmetic(
        &mut self,
        op: &BinOp,
        left: &inkwell::values::BasicValueEnum<'ctx>,
        right: &inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if left.is_int_value() {
            self.build_int_arithmetic(op, left, right)
        } else {
            self.build_float_arithmetic(op, left, right)
        }
    }

    fn build_int_arithmetic(
        &mut self,
        op: &BinOp,
        left: &inkwell::values::BasicValueEnum<'ctx>,
        right: &inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let (l, r) = (left.into_int_value(), right.into_int_value());
        let value = match *op {
            BinOp::Add => self
                .builder
                .build_int_add(l, r, "add")
                .unwrap()
                .into(),
            BinOp::Sub => self
                .builder
                .build_int_sub(l, r, "sub")
                .unwrap()
                .into(),
            BinOp::Mul => self
                .builder
                .build_int_mul(l, r, "mul")
                .unwrap()
                .into(),
            BinOp::Div => {
                self.emit_div_zero_check(r, false)?;
                self.builder
                    .build_int_signed_div(l, r, "div")
                    .unwrap()
                    .into()
            }
            BinOp::Mod => {
                self.emit_div_zero_check(r, true)?;
                self.builder
                    .build_int_signed_rem(l, r, "mod")
                    .unwrap()
                    .into()
            }
            _ => unreachable!("non-arithmetic operator reached build_int_arithmetic"),
        };
        Ok(value)
    }

    fn build_float_arithmetic(
        &mut self,
        op: &BinOp,
        left: &inkwell::values::BasicValueEnum<'ctx>,
        right: &inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        if *op == BinOp::Mod {
            return Err(HuziError::new_global(
                "Operator '%' requires integer operands",
            ));
        }

        let (l, r) = (left.into_float_value(), right.into_float_value());
        let value = match *op {
            BinOp::Add => self
                .builder
                .build_float_add(l, r, "fadd")
                .unwrap()
                .into(),
            BinOp::Sub => self
                .builder
                .build_float_sub(l, r, "fsub")
                .unwrap()
                .into(),
            BinOp::Mul => self
                .builder
                .build_float_mul(l, r, "fmul")
                .unwrap()
                .into(),
            BinOp::Div => self
                .builder
                .build_float_div(l, r, "fdiv")
                .unwrap()
                .into(),
            _ => unreachable!("non-arithmetic operator reached build_float_arithmetic"),
        };
        Ok(value)
    }

    fn compare_predicates(op: &BinOp) -> (inkwell::IntPredicate, inkwell::FloatPredicate) {
        match op {
            BinOp::Eq => (inkwell::IntPredicate::EQ, inkwell::FloatPredicate::OEQ),
            BinOp::Neq => (inkwell::IntPredicate::NE, inkwell::FloatPredicate::ONE),
            BinOp::Lt => (inkwell::IntPredicate::SLT, inkwell::FloatPredicate::OLT),
            BinOp::Le => (inkwell::IntPredicate::SLE, inkwell::FloatPredicate::OLE),
            BinOp::Gt => (inkwell::IntPredicate::SGT, inkwell::FloatPredicate::OGT),
            BinOp::Ge => (inkwell::IntPredicate::SGE, inkwell::FloatPredicate::OGE),
            _ => unreachable!("non-comparison operator reached compare_predicates"),
        }
    }

    pub(super) fn build_int_or_float_compare(
        &self,
        int_pred: inkwell::IntPredicate,
        float_pred: inkwell::FloatPredicate,
        left: &inkwell::values::BasicValueEnum<'ctx>,
        right: &inkwell::values::BasicValueEnum<'ctx>,
    ) -> inkwell::values::BasicValueEnum<'ctx> {
        if left.is_int_value() {
            self.builder
                .build_int_compare(int_pred, left.into_int_value(), right.into_int_value(), "cmp")
                .unwrap()
                .into()
        } else {
            self.builder
                .build_float_compare(
                    float_pred,
                    left.into_float_value(),
                    right.into_float_value(),
                    "cmp",
                )
                .unwrap()
                .into()
        }
    }

    /// 字符串比较:调用 C `strcmp` 后与 0 比较。支持全部六个比较运算符
    /// (`<` 等按字典序),结果为 `i1`。
    pub(super) fn build_string_compare(
        &mut self,
        op: &BinOp,
        left: &inkwell::values::BasicValueEnum<'ctx>,
        right: &inkwell::values::BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        let strcmp_fn = self
            .module
            .get_function("strcmp")
            .expect("strcmp declared in prelude");
        let cmp = self
            .builder
            .build_call(
                strcmp_fn,
                &[left.into_pointer_value().into(), right.into_pointer_value().into()],
                "str_cmp",
            )
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        let zero = self.context.i32_type().const_int(0, false);
        let (int_pred, _) = Self::compare_predicates(op);
        Ok(self
            .builder
            .build_int_compare(int_pred, cmp, zero, "str_bool")
            .unwrap()
            .into())
    }

    /// 操作数是否为确定大小的数组(变量或结构体字段)。用于在字符串
    /// 比较路径上拦下"数组与数组比较"。
    fn expr_is_array(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Ident(name) => self
                .scope_lookup(name)
                .map(|slot| slot.array_len.is_some())
                .unwrap_or(false),
            Expr::FieldAccess(fa) => self
                .struct_def_of_expr(&fa.base)
                .and_then(|(_, fields)| fields.iter().find(|f| f.name == fa.field))
                .map(|f| matches!(f.ast_ty, Type::Array(..)))
                .unwrap_or(false),
            _ => false,
        }
    }
}
