//! 表达式直系子表达式只读遍历(审计/补全用,不含遍历器类型)。
//!
//! `for_each_child_expr` 对每个 `Expr` 变体产出其直系子 `Expr`
//! (如 `Binary` 产出左右操作数);`Block` 体与闭包作用域边界不下钻:
//! `If`/`Match` 只产出条件/判别式与守卫,分支块由调用方按语句遍历;
//! `Closure` 产出空(作用域隔离,调用方须另建 `known` 集)。

use super::expr::Expr;

/// 对 `expr` 的每个直系子表达式调用 `f`;只读,不分配。
pub fn for_each_child_expr(expr: &Expr, f: &mut impl FnMut(&Expr)) {
    match expr {
        Expr::Binary(b) => {
            f(&b.left);
            f(&b.right);
        }
        Expr::Unary(u) => f(&u.operand),
        Expr::Call(c) => {
            f(&c.callee);
            for a in &c.arguments {
                f(a);
            }
        }
        Expr::Assign(a) => {
            f(&a.target);
            f(&a.value);
        }
        Expr::ArrayIndex(a) => {
            f(&a.array);
            f(&a.index);
        }
        Expr::ArrayLiteral(es) | Expr::TupleLiteral(es) => {
            for e in es {
                f(e);
            }
        }
        Expr::BoxAlloc(inner) => f(inner),
        Expr::Try(t) => f(&t.inner),
        Expr::FieldAccess(fa) => f(&fa.base),
        Expr::StructLiteral(sl) => {
            for (_, e) in &sl.fields {
                f(e);
            }
        }
        Expr::EnumConstruct(ec) => {
            for a in &ec.args {
                f(a);
            }
        }
        Expr::MethodCall(m) => {
            f(&m.receiver);
            for a in &m.arguments {
                f(a);
            }
        }
        Expr::FString(fs) => {
            for a in &fs.args {
                f(a);
            }
        }
        // 条件/判别式与守卫是直系子表达式;分支块由调用方遍历。
        Expr::If(i) => f(&i.condition),
        Expr::Match(m) => {
            f(&m.scrutinee);
            for arm in &m.arms {
                if let Some(g) = &arm.guard {
                    f(g);
                }
            }
        }
        // 叶节点/作用域边界:无直系子表达式。
        Expr::Literal(_)
        | Expr::Ident(_)
        | Expr::VecEmpty(_)
        | Expr::Null
        | Expr::Closure(_) => {}
    }
}
