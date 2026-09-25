//! Box 赋值相容校验:`box(..)`/`null` 与期望 AST 类型的层数比对。

use super::super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    /// 编译前校验 `box(..)`/`null` 与期望 AST 类型相容。LLVM 层面
    /// `Box<Node>` 与 `Box<Other>` 都是指针,此处做层数 + 结构名比对补位。
    /// 嵌套层数不一致(如 `box(box(..))` 进 `Box<Node>` 槽)同样报错。
    pub(in super::super) fn check_box_assignable(&self, value_expr: &Expr, expected: &Type) -> Result<()> {
        match value_expr {
            Expr::Null => {
                if !Self::is_box_or_weak_ast(expected) {
                    return Err(HuziError::new_global(format!(
                        "null can only be assigned to a Box<T> or weak Box<T> slot (found '{}'); add a `: Box<...>` annotation or assign to a Box field/parameter",
                        expected
                    )));
                }
                Ok(())
            }
            Expr::BoxAlloc(inner) => {
                if !Self::is_box_or_weak_ast(expected) {
                    return Err(HuziError::new_global(format!(
                        "Cannot assign a Box value to non-Box type '{}'; use a `: Box<...>` slot",
                        expected
                    )));
                }
                let target_expected = match expected {
                    Type::Weak(inner_box) => inner_box.as_ref(),
                    _ => expected,
                };
                self.check_box_nest_match(inner, target_expected)
            }
            _ => Ok(()),
        }
    }

    /// `box(E)` 实际形状与期望 `Box` 形状的层数 + 结构名比对。
    /// 内容不可推导(如函数调用结果)时跳过,交由 LLVM 层决定。
    fn check_box_nest_match(&self, inner: &Expr, expected: &Type) -> Result<()> {
        let (edepth, ename) = Self::box_ast_shape(expected).ok_or_else(|| {
            HuziError::new_global(format!(
                "Cannot assign a Box value to non-Box type '{}'; use a `: Box<...>` slot",
                expected
            ))
        })?;
        let Some((adepth, aname)) = self.box_content_shape(inner) else {
            return Ok(());
        };
        if adepth != edepth || aname != ename {
            return Err(HuziError::new_global(format!(
                "Box type mismatch: value holds '{}', but the slot expects '{}'",
                Self::display_box_nest(adepth, &aname),
                expected
            )));
        }
        Ok(())
    }
}
