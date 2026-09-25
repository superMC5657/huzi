//! `for` 语句编译:range 循环与 for-in 遍历的分派与公共助手。
//! 数组/vec/字符串遍历的具体生成在子模块 `iter` 中。

mod iter;

use super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::PointerValue;


impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_for(&mut self, stmt: &ForStmt, span: Span) -> Result<()> {
        match &stmt.source {
            ForSource::Range { .. } => self.compile_for_range(stmt, span),
            ForSource::Array(array) => self.compile_for_array(stmt, array, span),
        }
    }

    /// `for i in start..end`:整数范围循环(与此前行为一致)。
    ///
    /// 归纳变量采用"头部 phi + 体内 alloca"混合形态:循环条件直接比较 phi
    /// 值,尾部 latch 把新值存回 alloca,体内的读写仍走 alloca。dev 管线无
    /// mem2reg,原来头/尾每轮各一次的 `i` load 在 O0 IR 里全是内存访问;
    /// 改后条件比较零 load,每轮只剩尾部一次 load,且各入边的值恒等于
    /// alloca 内容(见 `emit_for_header`),可观察语义与原来逐一对齐。
    /// 体内无 `continue` 时不建中转块,头部 phi 只有预边/latch 两条入边。
    fn compile_for_range(&mut self, stmt: &ForStmt, span: Span) -> Result<()> {
        let i_type = self.context.i32_type();
        let (start, end) = self.compile_for_bounds(stmt)?;

        let function = self.current_function()?;
        // 回边起点必须是当前块:嵌套循环的 pre_block 是外层体中途的开放块,
        // 内层结束后外层语句线性续进内层 after 块(沿用既有嵌套模型)。
        let pre_block = self
            .builder
            .get_insert_block()
            .ok_or_else(|| HuziError::new_global("internal: for loop lost insert block"))?;

        let loop_block = self.context.append_basic_block(function, "for_loop");
        let body_block = self.context.append_basic_block(function, "for_body");
        // 有 continue 才建中转块:无 continue 的常见循环少一个冷块,phi 也少一条入边。
        let need_cont = for_body_may_continue(&stmt.body);
        let cont_block = need_cont.then(|| self.context.append_basic_block(function, "for_cont"));
        let after_block = self.context.append_basic_block(function, "for_after");

        // 分配并初始化循环变量(体内读写/闭包/调试声明仍走该 alloca,不变)。
        let i_alloca = self.build_alloca(i_type.into(), &stmt.var_name)?;
        self.builder.build_store(i_alloca, start).unwrap();
        self.declare_local(&stmt.var_name, i_alloca, i_type.into(), span);

        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();

        let i_phi =
            self.emit_for_header(start, end, pre_block, loop_block, body_block, after_block)?;
        // continue 汇入 for_cont 中转(只重载、不自增,保持既有语义);无
        // continue 时沿用旧目标(循环头),头部 phi 无需第三条入边。
        let continue_target = cont_block.unwrap_or(loop_block);
        self.loop_stack.push((continue_target, after_block));
        let (i_next, latch_block) =
            self.emit_for_body(stmt, i_type, i_alloca, body_block, loop_block)?;
        if let Some(cont) = cont_block {
            let i_cont = self.emit_for_continue(i_type, i_alloca, cont, loop_block)?;
            i_phi.add_incoming(&[(&i_cont, cont)]);
        }
        i_phi.add_incoming(&[(&i_next, latch_block)]);

        self.loop_stack.pop();

        // 循环结束后继续执行后续指令。
        self.builder.position_at_end(after_block);

        Ok(())
    }

    /// 编译范围边界；两者均必须能强制转换为 i32。
    fn compile_for_bounds(
        &mut self,
        stmt: &ForStmt,
    ) -> Result<(inkwell::values::IntValue<'ctx>, inkwell::values::IntValue<'ctx>)> {
        let i_type = self.context.i32_type();

        let ForSource::Range { start, end } = &stmt.source else {
            return Err(HuziError::new_global("internal: not a range for loop"));
        };

        let start = self.compile_expr(start)?;
        let start = match self.coerce_value(i_type.into(), start)? {
            inkwell::values::BasicValueEnum::IntValue(iv) => iv,
            _ => return Err(HuziError::new_global("for loop start must be an integer")),
        };

        let end = self.compile_expr(end)?;
        let end = match self.coerce_value(i_type.into(), end)? {
            inkwell::values::BasicValueEnum::IntValue(iv) => iv,
            _ => return Err(HuziError::new_global("for loop end must be an integer")),
        };

        Ok((start, end))
    }

    /// for-in 单轮绑定:写入循环变量,入新作用域执行循环体后退出。
    pub(super) fn execute_for_in_body(
        &mut self,
        stmt: &ForStmt,
        var_alloca: PointerValue<'ctx>,
        elem: inkwell::values::BasicValueEnum<'ctx>,
        elem_type: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Result<()> {
        self.builder.build_store(var_alloca, elem).unwrap();
        self.push_scope();
        self.scope_insert(
            stmt.var_name.clone(),
            VarSlot {
                ptr: var_alloca,
                ty: elem_type,
                elem: None,
                array_len: None,
                mutable: true,
                box_inner: None,
                map_kind: None,
            },
        );
        self.compile_block(&stmt.body)?;
        self.pop_scope();
        Ok(())
    }

    /// 循环头 BasicBlock:phi 传递归纳变量,每次迭代直接比较 phi(`i < end`),
    /// 消掉原来每轮的条件 load。不变式:各入边的值恒等于当时的 alloca
    /// 内容——预边带刚存入的 `start`、latch 边带刚存入的 `i_next`、continue
    /// 边(如有)带刚重载的内存值;故与原来"load 后比较"的可观察行为一致,
    /// 0..0 空循环与逆序首轮即出。
    fn emit_for_header(
        &mut self,
        start: inkwell::values::IntValue<'ctx>,
        end: inkwell::values::IntValue<'ctx>,
        pre_block: inkwell::basic_block::BasicBlock<'ctx>,
        loop_block: inkwell::basic_block::BasicBlock<'ctx>,
        body_block: inkwell::basic_block::BasicBlock<'ctx>,
        after_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<inkwell::values::PhiValue<'ctx>> {
        self.builder.position_at_end(loop_block);
        // phi 必须为块首指令:此时块为空,直接建在末尾即块首;回边入边在
        // 体与中转块生成后补齐(编译体时才知道 latch 起点)。
        let i_phi = self
            .builder
            .build_phi(start.get_type(), "i_phi")
            .unwrap();
        i_phi.add_incoming(&[(&start, pre_block)]);
        let i_cur = i_phi.as_basic_value().into_int_value();
        let condition = self
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, i_cur, end, "loop_cond")
            .unwrap();
        self.builder
            .build_conditional_branch(condition, body_block, after_block)
            .unwrap();
        Ok(i_phi)
    }

    /// 循环体 BasicBlock:在全新作用域中绑定循环变量(读写走 alloca,不变),
    /// 执行循环体,随后递增 `i` 并跳回循环头。返回 `(i_next, latch 起点)`,
    /// 供头部 phi 补回边入边。
    ///
    /// 递增从内存重载而非 `phi + 1`:体内的 `i = x` 改写必须被下一轮看到,
    /// 与既有 load/add/store 语义一致;头部已走 phi,热路径只剩这一次 load。
    fn emit_for_body(
        &mut self,
        stmt: &ForStmt,
        i_type: inkwell::types::IntType<'ctx>,
        i_alloca: inkwell::values::PointerValue<'ctx>,
        body_block: inkwell::basic_block::BasicBlock<'ctx>,
        loop_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<(
        inkwell::values::IntValue<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
    )> {
        self.builder.position_at_end(body_block);
        self.push_scope();
        self.scope_insert(
            stmt.var_name.clone(),
            VarSlot {
                ptr: i_alloca,
                ty: i_type.into(),
                elem: None,
                array_len: None,
                mutable: true,
                box_inner: None,
                map_kind: None,
            },
        );
        self.compile_block(&stmt.body)?;
        self.pop_scope();

        // latch 起点是体编译后 builder 所在块(直线体即 body_block,以 if
        // 收尾时为其 merge 块);沿用既有"无条件补回边"的线性模型。
        let latch_block = self
            .builder
            .get_insert_block()
            .ok_or_else(|| HuziError::new_global("internal: for loop lost insert block"))?;
        // 在跳回循环条件判断之前递增循环变量(从内存重载,见上)。
        let i = self
            .builder
            .build_load(i_type, i_alloca, "i")
            .unwrap()
            .into_int_value();
        let i_next = self
            .builder
            .build_int_add(i, i_type.const_int(1, false), "i_next")
            .unwrap();
        self.builder.build_store(i_alloca, i_next).unwrap();
        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();
        Ok((i_next, latch_block))
    }

    /// continue 中转块:只把当前内存值重载后带回头部 phi,不自增——与既有
    /// "continue 直接跳条件判断、跳过尾部递增"的语义一致。有 continue 的
    /// 循环其各边先汇到此块,头部 phi 才需要第三条入边。
    fn emit_for_continue(
        &mut self,
        i_type: inkwell::types::IntType<'ctx>,
        i_alloca: inkwell::values::PointerValue<'ctx>,
        cont_block: inkwell::basic_block::BasicBlock<'ctx>,
        loop_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>> {
        self.builder.position_at_end(cont_block);
        let i_cont = self
            .builder
            .build_load(i_type, i_alloca, "i_cont")
            .unwrap()
            .into_int_value();
        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();
        Ok(i_cont)
    }
}

/// 循环体是否可能含有直属于本循环的 `continue`:有则必须建 for_cont 中转块
/// 并给头部 phi 补第三条入边,无则跳过(少一个冷块)。保守判定:嵌套 for/while
/// 体内的 `continue` 归内层循环所有(depth+1 后忽略),其余位置(分支/闭包/
/// 块表达式/内嵌函数体,编译期与本循环共享 loop_stack)一律按本循环处理;
/// 宁可误报(多一个不可达中转块,校验仍过)不可漏报(缺入边则 IR 非法)。
fn for_body_may_continue(body: &Block) -> bool {
    block_may_continue(body, 0)
}

/// 块内是否有 depth 层的 `continue`(0 = 本循环)。
fn block_may_continue(block: &Block, depth: usize) -> bool {
    block
        .statements
        .iter()
        .any(|s| stmt_may_continue(&s.node, depth))
}

fn opt_expr_may_continue(value: &Option<Expr>, depth: usize) -> bool {
    value.as_ref().is_some_and(|e| expr_may_continue(e, depth))
}

/// 语句扫描:显式列出全部变体(不用通配),新增 AST 变体时编译报错,倒逼补扫描。
fn stmt_may_continue(stmt: &Stmt, depth: usize) -> bool {
    match stmt {
        Stmt::Let(l) => opt_expr_may_continue(&l.value, depth),
        Stmt::Expr(e) => expr_may_continue(&e.expr, depth),
        Stmt::Return(r) => opt_expr_may_continue(&r.value, depth),
        Stmt::Block(b) => block_may_continue(b, depth),
        Stmt::Continue => depth == 0,
        Stmt::Break => false,
        Stmt::If(i) => {
            expr_may_continue(&i.condition, depth)
                || block_may_continue(&i.then_branch, depth)
                || i.elif_branches
                    .iter()
                    .any(|(c, b)| expr_may_continue(c, depth) || block_may_continue(b, depth))
                || i.else_branch
                    .as_ref()
                    .is_some_and(|b| block_may_continue(b, depth))
        }
        // 范围边界先于入栈求值(归外层),只有体归内层;while 条件与体都归内层。
        Stmt::For(f) => {
            let source_hit = match &f.source {
                ForSource::Range { start, end } => {
                    expr_may_continue(start, depth) || expr_may_continue(end, depth)
                }
                ForSource::Array(array) => expr_may_continue(array, depth),
            };
            source_hit || block_may_continue(&f.body, depth + 1)
        }
        Stmt::While(w) => {
            expr_may_continue(&w.condition, depth + 1) || block_may_continue(&w.body, depth + 1)
        }
        Stmt::Defer(inner) => stmt_may_continue(&inner.node, depth),
        Stmt::Fn(f) => block_may_continue(&f.body, depth),
        Stmt::Impl(b) => b.methods.iter().any(|m| block_may_continue(&m.body, depth)),
        Stmt::Struct(_) | Stmt::Enum(_) | Stmt::Import(_) | Stmt::Export(_)
        | Stmt::Trait(_) => false,
    }
}

/// 表达式扫描:显式列出全部变体(不用通配),同上。
fn expr_may_continue(expr: &Expr, depth: usize) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Ident(_) | Expr::VecEmpty(_) | Expr::Null => false,
        Expr::Binary(b) => expr_may_continue(&b.left, depth) || expr_may_continue(&b.right, depth),
        Expr::Unary(u) => expr_may_continue(&u.operand, depth),
        Expr::Call(c) => {
            expr_may_continue(&c.callee, depth)
                || c.arguments.iter().any(|a| expr_may_continue(a, depth))
        }
        Expr::Assign(a) => {
            expr_may_continue(&a.target, depth) || expr_may_continue(&a.value, depth)
        }
        Expr::ArrayIndex(i) => {
            expr_may_continue(&i.array, depth) || expr_may_continue(&i.index, depth)
        }
        Expr::ArrayLiteral(items) | Expr::TupleLiteral(items) => {
            items.iter().any(|e| expr_may_continue(e, depth))
        }
        Expr::BoxAlloc(inner) => expr_may_continue(inner, depth),
        Expr::Try(t) => expr_may_continue(&t.inner, depth),
        Expr::FieldAccess(f) => expr_may_continue(&f.base, depth),
        Expr::MethodCall(m) => {
            expr_may_continue(&m.receiver, depth)
                || m.arguments.iter().any(|a| expr_may_continue(a, depth))
        }
        Expr::StructLiteral(s) => s.fields.iter().any(|(_, e)| expr_may_continue(e, depth)),
        Expr::EnumConstruct(e) => e.args.iter().any(|a| expr_may_continue(a, depth)),
        Expr::FString(f) => f.args.iter().any(|a| expr_may_continue(a, depth)),
        Expr::If(i) => {
            expr_may_continue(&i.condition, depth)
                || block_may_continue(&i.then_branch, depth)
                || block_may_continue(&i.else_branch, depth)
        }
        Expr::Match(m) => {
            expr_may_continue(&m.scrutinee, depth)
                || m.arms.iter().any(|arm| {
                    arm.guard.as_ref().is_some_and(|g| expr_may_continue(g, depth))
                        || block_may_continue(&arm.body, depth)
                })
        }
        Expr::Closure(c) => match &c.body {
            ClosureBody::Expr(e) => expr_may_continue(e, depth),
            ClosureBody::Block(b) => block_may_continue(b, depth),
        },
    }
}
