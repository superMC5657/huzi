use huzi_ast::*;
use huzi_error::HuziError;
use huzi_error::Result;
use std::collections::HashMap;
use inkwell::{
    AddressSpace,
    builder::Builder,
    context::Context,
    module::Module,
    types::BasicType,
    values::{FunctionValue, PointerValue},
};

/// 程序中的全部结构体定义 (AST)。
fn program_struct_definitions(program: &Program) -> Vec<StructDef> {
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
fn program_enum_definitions(program: &Program) -> Vec<EnumDef> {
    program
        .statements
        .iter()
        .filter_map(|s| match &s.node {
            Stmt::Enum(d) => Some(d.clone()),
            _ => None,
        })
        .collect()
}

fn program_type_definitions(program: &Program) -> (Vec<StructDef>, Vec<EnumDef>) {
    (
        program_struct_definitions(program),
        program_enum_definitions(program),
    )
}

/// 程序中的全部函数定义 (AST) 及其定义位置(供调试信息使用)。
fn module_fn_statements(program: &Program) -> Vec<(FnStmt, Span)> {
    program
        .statements
        .iter()
        .filter_map(|s| match &s.node {
            Stmt::Fn(f) => Some((f.clone(), s.span)),
            _ => None,
        })
        .collect()
}

mod aggregates;
mod args;
mod args_utf8;
mod box_nest;
mod box_print;
mod boxed;
mod builtins;
mod builtins_math;
mod builtins_print;
mod builtins_string;
mod builtins_string_util;
mod builtins_io;
mod builtins_file_check;
mod builtins_sys;
mod debuginfo;
mod defer;
mod enum_eq;
mod expr;
mod expr_binary;
mod expr_place;
mod map;
mod map_ops;
mod match_expr;
mod mem_free;
mod runtime;
mod stmt;
mod stmt_branch;
mod stmt_for;
#[cfg(test)]
mod tests;
mod tuples;
mod type_cycles;
mod types;
mod vec;
mod vec_ops;
pub(super) mod generic;


/// A variable slot: `ptr` always holds a pointer whose loaded value has type
/// `ty`. For arrays, `ptr` holds the address of the array data (loaded as a
/// `ptr`), and `elem` records the element type for GEP/indexing. For
/// `Box<T>` variables, `ty` is a plain pointer and `box_inner` records the
/// nested pointee (see `box_nest::BoxNest`) so field access can
/// auto-deref through every layer.
#[derive(Clone, Copy)]
struct VarSlot<'ctx> {
    ptr: PointerValue<'ctx>,
    ty: inkwell::types::BasicTypeEnum<'ctx>,
    elem: Option<inkwell::types::BasicTypeEnum<'ctx>>,
    array_len: Option<u32>,
    mutable: bool,
    box_inner: Option<box_nest::BoxNest<'ctx>>,
}

/// A registered struct field. `ast_ty` keeps the original AST type because
/// array fields decay to bare pointers in LLVM and would lose their element
/// type.
#[derive(Clone)]
struct StructFieldInfo<'ctx> {
    name: String,
    ty: inkwell::types::BasicTypeEnum<'ctx>,
    ast_ty: Type,
}

#[derive(Clone)]
struct EnumVariantInfo<'ctx> {
    name: String,
    /// Discriminant value, equal to the variant's declaration index.
    tag: u32,
    /// LLVM type of the payload (a field struct for multi-payload variants);
    /// None for unit variants.
    payload: Option<inkwell::types::BasicTypeEnum<'ctx>>,
    /// AST payload types (retains array element types, like StructFieldInfo).
    ast_payloads: Vec<Type>,
    /// Index of this variant's payload inside the payload-union struct.
    payload_slot: Option<u32>,
}

#[derive(Clone)]
struct EnumInfo<'ctx> {
    name: String,
    variants: Vec<EnumVariantInfo<'ctx>>,
    /// Data-carrying enums are laid out as { i32 tag, payload union }. Simple
    /// enums are represented directly as their i32 tag (None here).
    llvm: Option<inkwell::types::StructType<'ctx>>,
    payload_union: Option<inkwell::types::StructType<'ctx>>,
}

/// 一个已导入的模块。内置模块(如 math)没有源码,其符号在
/// 编译期由 builtin 调度处理;文件模块携带解析后的 AST。
#[derive(Clone)]
pub struct ModuleCode {
    pub name: String,
    pub program: Option<Program>,
    /// 模块源文件路径(内置模块为 None),用于文件模块的 DIFile。
    pub path: Option<String>,
}

pub struct CodeGen<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    scopes: Vec<HashMap<String, VarSlot<'ctx>>>,
    functions: HashMap<String, (FunctionValue<'ctx>, Vec<inkwell::types::BasicTypeEnum<'ctx>>)>,
    current_return_type: Option<inkwell::types::BasicTypeEnum<'ctx>>,
    /// 各函数的形参 AST 类型(限定名 -> 参数表),供 `box`/`null` 实参与
    /// `Box<T>` 形参的精确校验(LLVM 层面两者都是指针,无法区分)。
    fn_param_ast: HashMap<String, Vec<Type>>,
    /// 各函数的返回 AST 类型,供表达式求值时判断调用结果是否为 Box。
    fn_return_ast: HashMap<String, Type>,
    /// 当前函数中分配的 Box 局部变量槽 (alloca_ptr, llvm_ty),统一在函数退出时 release。
    box_slots: Vec<(inkwell::values::PointerValue<'ctx>, inkwell::types::BasicTypeEnum<'ctx>)>,
    /// 无返回值函数表(限定名 -> 是否省略返回类型):`fn foo() {...}` 仍按
    /// i32 隐式 `return 0` 生成代码,但其调用值不可用于变量赋值等值位置。
    fn_no_return: HashMap<String, bool>,
    /// 当前函数的 Huzi 声明返回类型,供 `return null` 的位置校验。
    current_return_ast: Option<Type>,
    /// (continue_target, break_target) for each enclosing loop.
    loop_stack: Vec<(inkwell::basic_block::BasicBlock<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)>,
    /// Registered user-defined structs: name -> (LLVM type, ordered fields).
    structs: HashMap<
        String,
        (
            inkwell::types::StructType<'ctx>,
            Vec<StructFieldInfo<'ctx>>,
        ),
    >,
    /// Registered user-defined enums: name -> layout info.
    enums: HashMap<String, EnumInfo<'ctx>>,
    /// 按结构体类型生成的运行时打印机(`huzi_print_struct_<Name>`),供
    /// `print(box)` 的判空递归展开复用,避免编译期内联无限递归。
    struct_printers: HashMap<String, FunctionValue<'ctx>>,
    /// Imported modules, registered via [`CodeGen::add_module`] before compile.
    modules: Vec<ModuleCode>,
    /// 正在编译的模块名;函数注册/查找按 `模块::名` 限定,主程序为 None。
    current_module: Option<String>,
    /// DWARF 调试信息状态(`-g` 时由 debuginfo 模块填充)。
    debug: Option<debuginfo::DebugState<'ctx>>,
    /// 当前函数的 DISubprogram,作为语句行号与变量的 DI scope。
    current_subprogram: Option<inkwell::debug_info::DISubprogram<'ctx>>,
    /// 当前函数延迟执行栈。
    defer_stack: Vec<defer::DeferEntry<'ctx>>,
}
impl<'ctx> CodeGen<'ctx> {
    pub fn new(context: &'ctx Context, name: &str) -> Self {
        let module = context.create_module(name);
        let builder = context.create_builder();

        Self {
            context,
            module,
            builder,
            scopes: Vec::new(),
            functions: HashMap::new(),
            current_return_type: None,
            fn_param_ast: HashMap::new(),
            fn_return_ast: HashMap::new(),
            box_slots: Vec::new(),
            fn_no_return: HashMap::new(),
            current_return_ast: None,
            loop_stack: Vec::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            struct_printers: HashMap::new(),
            modules: Vec::new(),
            current_module: None,
            debug: None,
            current_subprogram: None,
            defer_stack: Vec::new(),
        }
    }

    /// Register an imported module (call before [`CodeGen::compile`]).
    /// 内置模块传 `None`,文件模块传入其解析后的 AST。
    pub fn add_module(&mut self, name: &str, program: Option<&Program>, path: Option<&str>) {
        if self.modules.iter().any(|m| m.name == name) {
            return;
        }
        self.modules.push(ModuleCode {
            name: name.to_string(),
            program: program.cloned(),
            path: path.map(|p| p.to_string()),
        });
    }

    /// 编译模块内代码时,函数按 `模块::名` 限定;主程序代码原样返回。
    fn qualify_name(&self, name: &str) -> String {
        match &self.current_module {
            Some(m) => format!("{}::{}", m, name),
            None => name.to_string(),
        }
    }

    pub fn compile(&mut self, program: &Program) -> Result<()> {
        let monomorphized = generic::monomorphize_all(program, &mut self.modules)?;
        let program = &monomorphized;

        self.prelude()?;

        // 模块先注册类型与函数签名,主程序才能引用模块符号。
        let modules = self.modules.clone();
        for m in &modules {
            self.current_module = Some(m.name.clone());
            if let Some(prog) = &m.program {
                self.register_module_types(prog)?;
            }
        }
        self.current_module = None;

        let fn_stmts = self.register_program_types(program)?;
        self.declare_fn_signatures(&fn_stmts)?;

        // 模块函数体先于主程序编译,函数已注册,互相可见。
        for m in &modules {
            if let Some(prog) = &m.program {
                self.current_module = Some(m.name.clone());
                self.use_debug_file(m.path.as_deref());
                for (fn_stmt, span) in module_fn_statements(prog) {
                    self.compile_fn(&fn_stmt, span)?;
                }
                self.current_module = None;
            }
        }
        self.use_debug_file(None);

        for (fn_stmt, span) in &fn_stmts {
            self.compile_fn(fn_stmt, *span)?;
        }

        self.compile_top_level(program, &fn_stmts)?;

        self.finalize_debug_info();
        Ok(())
    }

    /// Register all top-level struct/enum definitions before anything else
    /// so function signatures and field types can reference them. Returns
    /// the collected function definitions.
    fn register_program_types(&mut self, program: &Program) -> Result<Vec<(FnStmt, Span)>> {
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
    fn register_module_types(&mut self, program: &Program) -> Result<()> {
        let (struct_defs, enum_defs) = program_type_definitions(program);
        self.register_type_definitions(&struct_defs, &enum_defs)?;
        for (fn_stmt, span) in module_fn_statements(program) {
            self.compile_fn_signature(&fn_stmt, span)?;
        }
        Ok(())
    }

    /// 结构体/枚举注册的公共阶段:循环检查 + 名称占位 + 字段/载荷解析。
    fn register_type_definitions(
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

    fn declare_fn_signatures(&mut self, fn_stmts: &[(FnStmt, Span)]) -> Result<()> {
        for (fn_stmt, span) in fn_stmts {
            self.compile_fn_signature(fn_stmt, *span)?;
        }
        Ok(())
    }

    /// Top-level statements must live in a `main` function; synthesize one
    /// if the program only has top-level code.
    fn compile_top_level(&mut self, program: &Program, fn_stmts: &[(FnStmt, Span)]) -> Result<()> {
        let has_main = fn_stmts.iter().any(|(f, _)| f.name == "main");
        let top_level: Vec<&Spanned<Stmt>> = program
            .statements
            .iter()
            .filter(|s| {
                !matches!(
                    &s.node,
                    Stmt::Fn(_) | Stmt::Struct(_) | Stmt::Enum(_) | Stmt::Import(_)
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
        self.scopes = vec![HashMap::new()];

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

    fn compile_fn_signature(&mut self, stmt: &FnStmt, span: Span) -> Result<()> {
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

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn scope_insert(&mut self, name: String, slot: VarSlot<'ctx>) {
        self.scopes
            .last_mut()
            .expect("scope stack is never empty")
            .insert(name, slot);
    }

    fn scope_lookup(&self, name: &str) -> Option<VarSlot<'ctx>> {
        for scope in self.scopes.iter().rev() {
            if let Some(slot) = scope.get(name) {
                return Some(*slot);
            }
        }
        None
    }

    fn current_function(&self) -> Result<FunctionValue<'ctx>> {
        self.builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .ok_or_else(|| HuziError::new_global("No current function"))
    }

    // ==================== Diagnostics ====================

    /// 收集当前可见的全部名字(函数、结构体、枚举、作用域变量),
    /// 用于未知名字的 "did you mean" 建议。
    fn visible_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.functions.keys().map(|s| s.as_str()).collect();
        names.extend(self.structs.keys().map(|s| s.as_str()));
        names.extend(self.enums.keys().map(|s| s.as_str()));
        for scope in &self.scopes {
            names.extend(scope.keys().map(|s| s.as_str()));
        }
        names
    }

    /// 构造 "Unknown variable" 错误,附最接近名字的修复建议。
    pub(super) fn unknown_variable_error(&self, name: &str) -> HuziError {
        let mut message = format!("Unknown variable: {}", name);
        if let Some(hint) = huzi_error::did_you_mean(name, self.visible_names()) {
            message.push_str(&format!("\n  help: {}", hint));
        }
        HuziError::new_global(message)
    }

    /// 构造 "Unknown function" 错误,附最接近名字的修复建议。
    pub(super) fn unknown_function_error(&self, name: &str) -> HuziError {
        let mut message = format!("Unknown function: {}", name);
        if let Some(hint) = huzi_error::did_you_mean(name, self.visible_names()) {
            message.push_str(&format!("\n  help: {}", hint));
        }
        HuziError::new_global(message)
    }

    /// True if the current insert block has no terminator yet.
    fn at_open_end(&self) -> bool {
        self.builder
            .get_insert_block()
            .map(|b| b.get_terminator().is_none())
            .unwrap_or(false)
    }

    // ==================== Statements ====================
}

impl<'ctx> CodeGen<'ctx> {
    pub fn print_llvm_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn verify(&self) -> bool {
        self.module.verify().is_ok()
    }

    pub fn write_ir_to_file(&self, path: &str) -> std::result::Result<(), std::io::Error> {
        std::fs::write(path, self.module.print_to_string().to_string())
    }
}
