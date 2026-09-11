use super::{CodeGen, VarSlot};
use inkwell::AddressSpace;
use inkwell::types::BasicType;
use std::collections::HashMap;
use huzi_ast::*;
use huzi_error::{HuziError, Result};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn compile_stmt(&mut self, stmt: &Stmt, span: Span) -> Result<()> {
        // 之后生成的指令都归属该语句所在行(未启用调试时为 no-op)。
        self.set_current_debug_span(span);
        match stmt {
            Stmt::Let(let_stmt) => self.compile_let(let_stmt, span),
            Stmt::Struct(_) => Err(HuziError::new_global(
                "Struct definitions are only allowed at the top level",
            )),
            Stmt::Enum(_) => Err(HuziError::new_global(
                "Enum definitions are only allowed at the top level",
            )),
            Stmt::Fn(fn_stmt) => self.compile_fn(fn_stmt, span),
            // import 在加载阶段已处理,编译期不再出现
            Stmt::Import(_) => Ok(()),
            Stmt::Expr(expr_stmt) => {
                self.compile_expr(&expr_stmt.expr)?;
                Ok(())
            }
            Stmt::Return(return_stmt) => self.compile_return(return_stmt),
            Stmt::Break => self.compile_break(),
            Stmt::Continue => self.compile_continue(),
            Stmt::Block(block) => self.compile_block(block),
            Stmt::If(if_stmt) => self.compile_if(if_stmt, span),
            Stmt::For(for_stmt) => self.compile_for(for_stmt, span),
            Stmt::While(while_stmt) => self.compile_while(while_stmt),
        }
    }

    pub(super) fn compile_let(&mut self, stmt: &LetStmt, span: Span) -> Result<()> {
        match &stmt.value {
            Some(Expr::ArrayLiteral(elements)) => self.compile_let_array(stmt, elements, span),
            Some(Expr::TupleLiteral(elements)) => self.compile_let_tuple(stmt, elements, span),
            Some(value_expr) => self.compile_let_with_value(stmt, value_expr, span),
            None => self.compile_let_uninitialized(stmt, span),
        }
    }

    /// `let name = [a, b, c]` — build a fixed-size array and store its
    /// address in a pointer slot so loading the variable yields the array
    /// address.
    fn compile_let_array(&mut self, stmt: &LetStmt, elements: &[Expr], span: Span) -> Result<()> {
        if elements.is_empty() {
            return Err(HuziError::new_global("Empty array literal not supported"));
        }

        let mut values = Vec::with_capacity(elements.len());
        for e in elements {
            values.push(self.compile_expr(e)?);
        }

        let elem_type = values[0].get_type();
        let array_type = elem_type.array_type(values.len() as u32);
        let array_ptr = self.build_alloca(array_type.into(), &stmt.name)?;

        for (i, val) in values.iter().enumerate() {
            let val = self.coerce_value(elem_type, *val)?;
            let index = self.context.i32_type().const_int(i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(elem_type, array_ptr, &[index], "arr_elem")
                    .unwrap()
            };
            self.builder.build_store(elem_ptr, val).unwrap();
        }

        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let slot_ptr = self.build_alloca(ptr_ty.into(), &format!("{}.ptr", stmt.name))?;
        self.builder.build_store(slot_ptr, array_ptr).unwrap();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: slot_ptr,
                ty: ptr_ty.into(),
                elem: Some(elem_type),
                array_len: Some(values.len() as u32),
                mutable: stmt.mutable,
            },
        );
        self.declare_local(&stmt.name, slot_ptr, ptr_ty.into(), span);
        Ok(())
    }

    /// `let name[: T] = value`.
    fn compile_let_with_value(&mut self, stmt: &LetStmt, value_expr: &Expr, span: Span) -> Result<()> {
        let mut value = self.compile_expr(value_expr)?;

        let var_type = match &stmt.type_annotation {
            Some(t) => {
                let ty = self.type_to_llvm(t)?;
                value = self.coerce_value(ty, value)?;
                ty
            }
            None => value.get_type(),
        };

        let alloca = self.build_alloca(var_type, &stmt.name)?;
        self.builder.build_store(alloca, value).unwrap();

        // Pointers to strings support char indexing.
        let elem = if var_type.is_pointer_type() {
            Some(self.context.i8_type().into())
        } else {
            None
        };

        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: var_type,
                elem,
                array_len: None,
                mutable: stmt.mutable,
            },
        );
        self.declare_local(&stmt.name, alloca, var_type, span);
        Ok(())
    }

    /// `let name: T;` — declaration without initializer, zero-initialized.
    fn compile_let_uninitialized(&mut self, stmt: &LetStmt, span: Span) -> Result<()> {
        // Requires a type annotation.
        let ty = match &stmt.type_annotation {
            Some(t) => self.type_to_llvm(t)?,
            None => {
                return Err(HuziError::new_global(format!(
                    "Variable '{}' declared without a value or a type annotation",
                    stmt.name
                )))
            }
        };
        let alloca = self.build_alloca(ty, &stmt.name)?;
        self.builder.build_store(alloca, ty.const_zero()).unwrap();

        let elem = if ty.is_pointer_type() {
            Some(self.context.i8_type().into())
        } else {
            None
        };

        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty,
                elem,
                array_len: None,
                mutable: stmt.mutable,
            },
        );
        self.declare_local(&stmt.name, alloca, ty, span);
        Ok(())
    }

    pub(super) fn compile_fn(&mut self, stmt: &FnStmt, span: Span) -> Result<()> {
        let (function, _) = self
            .functions
            .get(&self.qualify_name(&stmt.name))
            .cloned()
            .ok_or_else(|| HuziError::new_global(format!("Unknown function: {}", stmt.name)))?;

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        self.current_subprogram = function.get_subprogram();
        self.clear_debug_location();

        // The program entry point sets up UTF-8 console output first.
        if stmt.name == "main" {
            self.emit_console_utf8_setup();
            // A parameterless Huzi `fn main` is compiled with the C
            // `main(argc, argv)` signature; capture the hidden params.
            if stmt.params.is_empty() && function.count_params() == 2 {
                self.store_main_args(function);
            }
        }

        let return_type = match &stmt.return_type {
            Some(t) => self.type_to_llvm(t)?,
            None => self.context.i32_type().into(),
        };
        self.current_return_type = Some(return_type);
        self.scopes = vec![HashMap::new()];

        for (i, param) in stmt.params.iter().enumerate() {
            let arg = function.get_nth_param(i as u32).unwrap();
            let arg_type = arg.get_type();

            let alloca = self.build_alloca(arg_type, &param.name)?;
            self.builder.build_store(alloca, arg).unwrap();
            self.declare_param(&param.name, alloca, arg_type, i as u32 + 1, span.line as u32);

            // Arrays decay to pointers; remember the element type for indexing.
            let elem = match &param.param_type {
                Type::Array(elem_ty, _) => Some(self.type_to_llvm(elem_ty)?),
                // `s: str` parses as Named("str") (parse_type keeps builtin
                // names as Named), so both forms need char-index metadata.
                Type::Str => Some(self.context.i8_type().into()),
                Type::Named(n) if n == "str" => Some(self.context.i8_type().into()),
                _ => None,
            };

            self.scopes.last_mut().unwrap().insert(
                param.name.clone(),
                VarSlot {
                    ptr: alloca,
                    ty: arg_type,
                    elem,
                    array_len: match &param.param_type {
                        Type::Array(_, size) => Some(*size as u32),
                        _ => None,
                    },
                    mutable: true,
                },
            );
        }

        self.compile_block(&stmt.body)?;

        // Functions without an explicit return fall through with a zero value
        // of the declared return type.
        if self.at_open_end() {
            self.builder
                .build_return(Some(&return_type.const_zero()))
                .unwrap();
        }

        self.current_return_type = None;

        Ok(())
    }

    pub(super) fn compile_return(&mut self, stmt: &ReturnStmt) -> Result<()> {
        let ret_type = self
            .current_return_type
            .unwrap_or_else(|| self.context.i32_type().into());

        match &stmt.value {
            Some(value) => {
                let value = self.compile_expr(value)?;
                let value = self.coerce_value(ret_type, value)?;
                self.builder.build_return(Some(&value)).unwrap();
            }
            None => {
                self.builder
                    .build_return(Some(&ret_type.const_zero()))
                    .unwrap();
            }
        }
        Ok(())
    }

    pub(super) fn compile_block(&mut self, block: &Block) -> Result<()> {
        self.push_scope();
        for stmt in &block.statements {
            self.compile_stmt(&stmt.node, stmt.span)?;
        }
        self.pop_scope();
        Ok(())
    }

    /// Compile a block as an expression: the block's value is the value of its
    /// last expression statement.
    pub(super) fn compile_block_value(&mut self, block: &Block) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        self.push_scope();
        let mut last: Option<inkwell::values::BasicValueEnum<'ctx>> = None;
        for stmt in &block.statements {
            match &stmt.node {
                Stmt::Expr(es) => last = Some(self.compile_expr(&es.expr)?),
                other => self.compile_stmt(other, stmt.span)?,
            }
        }
        self.pop_scope();
        last.ok_or_else(|| HuziError::new_global("Block used as an expression must end with a value"))
    }

}
