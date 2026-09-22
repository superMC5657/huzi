use huzi_ast::*;
use huzi_error::HuziError;
use huzi_error::Result;
use std::collections::HashMap;
use inkwell::{
    builder::Builder,
    context::Context,
    module::Module,
    values::{FunctionValue, PointerValue},
};

mod program_compile;
mod qname;
use program_compile::*;

mod aggregates;
mod exports;
mod args;
mod args_utf8;
mod box_nest;
mod box_deref;
mod box_print;
mod boxed;
mod builtins;
mod builtins_bytes;
mod builtins_chan;
mod builtins_math;
mod builtins_print;
mod builtins_string;
mod builtins_string_util;
mod builtins_parse;
mod builtins_io;
mod builtins_file_check;
mod builtins_sys;
mod builtins_net;
mod builtins_thread;
mod debuginfo;
mod defer;
mod enum_eq;
mod expr;
mod expr_binary;
mod expr_place;
mod map;
mod map_kind;
mod map_rehash;
mod map_ops;
mod map_keys;
mod match_expr;
mod mem_free;
mod runtime;
mod stmt;
mod stmt_branch;
mod stmt_for;
mod try_op;
#[cfg(test)]
mod tests;
mod tuples;
mod type_cycles;
mod types;
mod vec;
mod vec_ops;
pub(super) mod generic;
pub(super) mod trait_;


/// Map 键值特化种类:当前支持三种单态化形态。
/// 复用泛型单态化/修饰名思路,每种对应一套条目布局。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum MapKind {
    /// `str -> i32`(默认,裸 `Map` 即此种)。
    StrI32,
    /// 映射形态 `str -> str`。
    StrStr,
    /// `i32 -> i32`。
    I32I32,
}

/// 变量槽：`ptr` 始终持有一个指针，其加载后的值类型为 `ty`。
/// 对于数组，`ptr` 持有数组数据的地址（作为指针加载），`elem`
/// 记录元素类型以供 GEP/索引。对于 `Box<T>` 变量，`ty` 为裸指针，
/// `box_inner` 记录嵌套指向的目标类型（参见 `box_nest::BoxNest`），
/// 从而支持跨任意层级的自动解引用字段访问。
#[derive(Clone, Copy)]
struct VarSlot<'ctx> {
    ptr: PointerValue<'ctx>,
    ty: inkwell::types::BasicTypeEnum<'ctx>,
    elem: Option<inkwell::types::BasicTypeEnum<'ctx>>,
    array_len: Option<u32>,
    mutable: bool,
    box_inner: Option<box_nest::BoxNest<'ctx>>,
    /// Map 槽的键值特化(非 map 为 None)。
    map_kind: Option<MapKind>,
}

/// 已注册的结构体字段。`ast_ty` 保留原始 AST 类型，因为数组字段
/// 在 LLVM 中会退化为裸指针而丢失其元素类型。
#[derive(Clone)]
struct StructFieldInfo<'ctx> {
    name: String,
    ty: inkwell::types::BasicTypeEnum<'ctx>,
    ast_ty: Type,
}

#[derive(Clone)]
struct EnumVariantInfo<'ctx> {
    name: String,
    /// 判别码值，等于该变体的声明索引。
    tag: u32,
    /// 变体负载的 LLVM 类型（多负载变体为字段结构体）；单元变体为 None。
    payload: Option<inkwell::types::BasicTypeEnum<'ctx>>,
    /// AST 负载类型（保留数组元素类型，与 StructFieldInfo 一致）。
    ast_payloads: Vec<Type>,
    /// 该变体负载在负载联合体结构体中的索引。
    payload_slot: Option<u32>,
}

#[derive(Clone)]
struct EnumInfo<'ctx> {
    name: String,
    variants: Vec<EnumVariantInfo<'ctx>>,
    /// 携带数据的枚举布局为 { i32 tag, payload_union }。
    /// 简单枚举直接表示为其 i32 tag（此处为 None）。
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
    /// 局部变量名 → 已知的 AST 类型(由 let 的值静态推导),
    /// 供 `r.1` 这类元组字段访问推断元素类型。
    local_ast: HashMap<String, Type>,
    /// 当前函数中分配的 Box 局部变量槽 (alloca_ptr, llvm_ty),统一在函数退出时 release。
    box_slots: Vec<(inkwell::values::PointerValue<'ctx>, inkwell::types::BasicTypeEnum<'ctx>)>,
    /// 无返回值函数表(限定名 -> 是否省略返回类型):`fn foo() {...}` 仍按
    /// i32 隐式 `return 0` 生成代码,但其调用值不可用于变量赋值等值位置。
    fn_no_return: HashMap<String, bool>,
    /// 当前函数的 Huzi 声明返回类型,供 `return null` 的位置校验。
    current_return_ast: Option<Type>,
    /// 每个外层循环的 (continue 目标 BasicBlock, break 目标 BasicBlock)。
    loop_stack: Vec<(inkwell::basic_block::BasicBlock<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)>,
    /// 已注册的用户定义结构体：名称 -> (LLVM 类型, 顺序字段列表)。
    structs: HashMap<
        String,
        (
            inkwell::types::StructType<'ctx>,
            Vec<StructFieldInfo<'ctx>>,
        ),
    >,
    /// 已注册的用户定义枚举：名称 -> 布局信息。
    enums: HashMap<String, EnumInfo<'ctx>>,
    /// 按结构体类型生成的运行时打印机(`huzi_print_struct_<Name>`),供
    /// `print(box)` 的判空递归展开复用,避免编译期内联无限递归。
    struct_printers: HashMap<String, FunctionValue<'ctx>>,
    /// 已导入的模块，在编译前通过 [`CodeGen::add_module`] 注册。
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
            local_ast: HashMap::new(),
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

    /// 注册已导入模块（在 [`CodeGen::compile`] 之前调用）。
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

    /// 编译模块内代码时,函数按 `模块::名` 限定;已限定名或主程序代码原样返回。
    fn qualify_name(&self, name: &str) -> String {
        if name.contains("::") {
            return name.to_string();
        }
        match &self.current_module {
            Some(m) => qname::qualified(m, name),
            None => name.to_string(),
        }
    }

    pub fn compile(&mut self, program: &Program) -> Result<()> {
        let monomorphized = generic::monomorphize_all(program, &mut self.modules)?;
        let desugared = trait_::desugar_traits(&monomorphized, &mut self.modules)?;
        let program = &desugared;

        self.prelude()?;

        // 类型注册统一先行:模块与主程序(含泛型单态化产物,如
        // Result__i32)的全部具名类型都注册完成后,才编译函数签名——
        // 任一侧的签名都可能引用另一侧的具名类型。
        let modules = self.modules.clone();
        for m in &modules {
            self.current_module = Some(m.name.clone());
            if let Some(prog) = &m.program {
                self.register_module_type_definitions(prog)?;
            }
        }
        self.current_module = None;

        let fn_stmts = self.register_program_types(program)?;

        for m in &modules {
            self.current_module = Some(m.name.clone());
            if let Some(prog) = &m.program {
                self.register_module_fn_signatures(prog)?;
            }
        }
        self.current_module = None;
        self.apply_module_export_signatures(&modules)?;
        self.declare_fn_signatures(&fn_stmts)?;

        // 模块函数体先于主程序编译,函数已注册,互相可见。
        // 泛型模板同样跳过:与主程序一致,只编译单态化产物。
        for m in &modules {
            if let Some(prog) = &m.program {
                self.current_module = Some(m.name.clone());
                self.use_debug_file(m.path.as_deref());
                for (fn_stmt, span) in module_fn_statements(prog) {
                    if fn_stmt.type_params.is_empty() {
                        self.compile_fn(&fn_stmt, span)?;
                    }
                }
                self.current_module = None;
            }
        }
        self.use_debug_file(None);
        self.apply_module_export_functions(&modules)?;

        for (fn_stmt, span) in &fn_stmts {
            self.compile_fn(fn_stmt, *span)?;
        }

        self.compile_top_level(program, &fn_stmts)?;

        self.finalize_debug_info();
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

    /// 当当前插入 BasicBlock 尚未包含终结指令（Terminator）时返回 true。
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

    /// 校验并返回 LLVM 原始错误信息(verify 失败时供 CLI 打印细节)。
    pub fn verify_detailed(&self) -> std::result::Result<(), String> {
        self.module.verify().map_err(|e| e.to_string())
    }

    pub fn write_ir_to_file(&self, path: &str) -> std::result::Result<(), std::io::Error> {
        std::fs::write(path, self.module.print_to_string().to_string())
    }
}
