use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;
use inkwell::types::BasicType;

/// 程序中的全部结构体定义 (AST)。
pub(super) fn program_struct_definitions(program: &Program) -> Vec<StructDef> {
    program
        .statements
        .iter()
        .filter_map(|s| match &s.node {
            Stmt::Struct(d) => Some(d.clone()),
            _ => None,
        })
        .collect()
}

/// 程序中的全部枚举定义 (AST)。
pub(super) fn program_enum_definitions(program: &Program) -> Vec<EnumDef> {
    program
        .statements
        .iter()
        .filter_map(|s| match &s.node {
            Stmt::Enum(d) => Some(d.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn program_type_definitions(program: &Program) -> (Vec<StructDef>, Vec<EnumDef>) {
    (
        program_struct_definitions(program),
        program_enum_definitions(program),
    )
}

/// 程序中的全部函数定义 (AST) 及其定义位置(供调试信息使用)。
pub(super) fn module_fn_statements(program: &Program) -> Vec<(FnStmt, Span)> {
    program
        .statements
        .iter()
        .filter_map(|s| match &s.node {
            Stmt::Fn(f) => Some((f.clone(), s.span)),
            _ => None,
        })
        .collect()
}

impl<'ctx> CodeGen<'ctx> {
    /// Register all top-level struct/enum definitions before anything else
    /// so function signatures and field types can reference them. Returns
    /// the collected function definitions.
    pub(super) fn register_program_types(&mut self, program: &Program) -> Result<Vec<(FnStmt, Span)>> {
        let (struct_defs, enum_defs) = program_type_definitions(program);
        self.register_type_definitions(&struct_defs, &enum_defs)?;

        Ok(program
            .statements
            .iter()
            .filter_map(|s| match &s.node {
                Stmt::Fn(f) => Some((f.clone(), s.span)),
                _ => None,
            })
            .collect())
    }

    /// Register a module file's struct/enum definitions and function
    /// signatures. Called with `current_module` set, so signatures are
    /// registered under qualified names.
    pub(super) fn register_module_types(&mut self, program: &Program) -> Result<()> {
        let (struct_defs, enum_defs) = program_type_definitions(program);
        self.register_type_definitions(&struct_defs, &enum_defs)?;
        for (fn_stmt, span) in module_fn_statements(program) {
            self.compile_fn_signature(&fn_stmt, span)?;
        }
        Ok(())
    }

    /// 结构体/枚举注册的公共阶段:循环检查 + 名称占位 + 字段/载荷解析。
    pub(super) fn register_type_definitions(
        &mut self,
        struct_defs: &[StructDef],
        enum_defs: &[EnumDef],
    ) -> Result<()> {
        self.check_type_cycles(struct_defs, enum_defs)?;
        self.register_struct_names(struct_defs)?;
        self.register_enum_names(enum_defs)?;
        self.resolve_struct_bodies(struct_defs)?;
        self.resolve_enum_bodies(enum_defs)?;
        Ok(())
    }

    pub(super) fn declare_fn_signatures(&mut self, fn_stmts: &[(FnStmt, Span)]) -> Result<()> {
        for (fn_stmt, span) in fn_stmts {
            self.compile_fn_signature(fn_stmt, *span)?;
        }
        Ok(())
    }

    /// Top-level statements must live in a `main` function; synthesize one
    /// if the program only has top-level code.
    pub(super) fn compile_top_level(&mut self, program: &Program, fn_stmts: &[(FnStmt, Span)]) -> Result<()> {
        let has_main = fn_stmts.iter().any(|(f, _)| f.name == "main");
        let top_level: Vec<&Spanned<Stmt>> = program
            .statements
            .iter()
            .filter(|s| {
                !matches!(
                    &s.node,
                    Stmt::Fn(_) | Stmt::Struct(_) | Stmt::Enum(_) | Stmt::Import(_) | Stmt::Trait(_) | Stmt::Impl(_)
                )
            })
            .collect();

        if has_main {
            if !top_level.is_empty() {
                return Err(HuziError::new_global(
                    "Cannot mix top-level statements with `fn main`; move the top-level code into a function",
                ));
            }
            return Ok(());
        }

        if top_level.is_empty() {
            return Err(HuziError::new_global(
                "No `fn main` found; define `fn main() -> i32 { ... }` or write top-level statements",
            ));
        }

        // The C runtime calls `main(argc, argv)`; capture both into globals
        // so the arg()/arg_count() builtins can read them.
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let main_type = i32_type.fn_type(&[i32_type.into(), ptr_type.into()], false);
        let line = top_level
            .first()
            .map(|s| s.span.start_line() as u32)
            .unwrap_or(1);
        let sp = self.create_subprogram(
            "main",
            line,
            &[i32_type.into(), ptr_type.into()],
            i32_type.into(),
        );
        let main_fn = self.module.add_function("main", main_type, None);
        if let Some(sp) = sp {
            main_fn.set_subprogram(sp);
        }
        self.functions.insert("main".to_string(), (main_fn, vec![]));
        self.current_subprogram = main_fn.get_subprogram();

        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);
        self.clear_debug_location();
        self.emit_console_utf8_setup();
        self.store_main_args(main_fn);
        self.current_return_type = Some(self.context.i32_type().into());
        self.scopes = vec![std::collections::HashMap::new()];

        for stmt in &top_level {
            self.compile_stmt(&stmt.node, stmt.span)?;
        }

        if self.at_open_end() {
            self.builder
                .build_return(Some(&self.context.i32_type().const_int(0, false)))
                .unwrap();
        }

        Ok(())
    }

    pub(super) fn compile_fn_signature(&mut self, stmt: &FnStmt, span: Span) -> Result<()> {
        let qualified_name = self.qualify_name(&stmt.name);
        // `fn main` 仍须显式 `-> i32`(C 入口约定);其它函数可省略返回类型。
        if qualified_name == "main" {
            let ok = match &stmt.return_type {
                Some(t) => self.type_to_llvm(t)? == self.context.i32_type().into(),
                None => false,
            };
            if !ok {
                return Err(HuziError::new_global(
                    "fn main must declare `-> i32` as its return type",
                ));
            }
        }
        if self.functions.contains_key(&qualified_name) {
            return Err(HuziError::new_global(format!(
                "Duplicate function definition: {}",
                qualified_name
            )));
        }

        let param_llvm_types: Vec<inkwell::types::BasicTypeEnum<'ctx>> = stmt
            .params
            .iter()
            .map(|p| self.type_to_llvm(&p.param_type))
            .collect::<Result<Vec<_>>>()?;
        // The entry point is compiled with the C `main(argc, argv)` signature
        // so the arg builtins can capture them; Huzi-level `fn main()` stays
        // parameterless.
        let param_llvm_types = if qualified_name == "main" && param_llvm_types.is_empty() {
            vec![
                self.context.i32_type().into(),
                self.context.ptr_type(AddressSpace::default()).into(),
            ]
        } else {
            param_llvm_types
        };
        let param_types: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
            param_llvm_types.iter().map(|t| (*t).into()).collect();

        let return_type = match &stmt.return_type {
            Some(t) => self.type_to_llvm(t)?,
            None => self.context.i32_type().into(),
        };
        let fn_type = return_type.fn_type(&param_types, false);

        let sp =
            self.create_subprogram(&qualified_name, span.start_line() as u32, &param_llvm_types, return_type);
        let function = self.module.add_function(&qualified_name, fn_type, None);
        if let Some(sp) = sp {
            function.set_subprogram(sp);
        }
        self.functions
            .insert(qualified_name.clone(), (function, param_llvm_types));
        // 无返回值标记并行记录,供值位置的调用校验(`let x = foo()` 拒绝)。
        self.fn_no_return.insert(qualified_name.clone(), stmt.return_type.is_none());
        // 形参 AST 类型并行记录,供调用点 `box`/`null` 实参校验。
        self.fn_param_ast.insert(
            qualified_name.clone(),
            stmt.params.iter().map(|p| p.param_type.clone()).collect(),
        );
        // 返回 AST 类型并行记录,供表达式求值时判断调用结果是否为 Box。
        if let Some(ret_ty) = &stmt.return_type {
            self.fn_return_ast.insert(qualified_name, ret_ty.clone());
        }

        Ok(())
    }
}
