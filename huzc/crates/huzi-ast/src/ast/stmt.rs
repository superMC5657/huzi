use super::expr::Expr;
use super::types::{Spanned, Type};

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Spanned<Stmt>>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let(LetStmt),
    Struct(StructDef),
    Enum(EnumDef),
    Fn(FnStmt),
    Import(ImportStmt),
    Export(ExportStmt),
    Expr(ExprStmt),
    Return(ReturnStmt),
    Break,
    Continue,
    Block(Block),
    If(IfStmt),
    For(ForStmt),
    While(WhileStmt),
    Defer(Box<Spanned<Stmt>>),
    Trait(TraitDef),
    Impl(ImplBlock),
}

#[derive(Debug, Clone)]
pub struct LetStmt {
    pub name: String,
    pub mutable: bool,
    pub type_annotation: Option<Type>,
    pub value: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct FnStmt {
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<FnParam>,
    pub return_type: Option<Type>,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub struct FnParam {
    pub name: String,
    pub param_type: Type,
}

/// `import math` — 导入一个模块。内置模块(如 math)由编译器提供;
/// 文件模块解析为导入文件同目录(或工作目录)下的 `<路径>.hz`,
/// 点分名(`mods.helpers`)对应子路径 `mods/helpers.hz`,符号绑定为末段名。
#[derive(Debug, Clone)]
pub struct ImportStmt {
    pub name: String,
}

/// 导出声明，例如：`export calc`, `export calc::*`, `export calc::add`
#[derive(Debug, Clone, PartialEq)]
pub struct ExportStmt {
    pub path: String,
    pub is_wildcard: bool,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<StructField>,
}

#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub field_type: Type,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub variants: Vec<EnumVariant>,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    /// 负载类型：`Red` 无负载，`Ok(i32)` 携带一个负载，
    /// `Pair(i32, str)` 携带多个负载。
    pub payloads: Vec<Type>,
}

#[derive(Debug, Clone)]
pub struct TraitDef {
    pub name: String,
    pub methods: Vec<TraitMethodDef>,
}

#[derive(Debug, Clone)]
pub struct TraitMethodDef {
    pub name: String,
    pub has_self: bool,
    pub params: Vec<FnParam>,
    pub return_type: Option<Type>,
}

#[derive(Debug, Clone)]
pub struct ImplBlock {
    pub trait_name: Option<String>,
    pub target_type: String,
    pub methods: Vec<FnStmt>,
}

#[derive(Debug, Clone)]
pub struct ExprStmt {
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub struct ReturnStmt {
    pub value: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub statements: Vec<Spanned<Stmt>>,
}

#[derive(Debug, Clone)]
pub struct IfStmt {
    pub condition: Expr,
    pub then_branch: Block,
    pub elif_branches: Vec<(Expr, Block)>,
    pub else_branch: Option<Block>,
}

/// `for` 循环的迭代来源:整数范围或数组。
#[derive(Debug, Clone)]
pub enum ForSource {
    /// 范围迭代：`for i in start..end`
    Range { start: Expr, end: Expr },
    /// `for x in arr`(数组变量/结构体字段,长度编译期已知)
    Array(Expr),
}

#[derive(Debug, Clone)]
pub struct ForStmt {
    pub var_name: String,
    pub source: ForSource,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub struct WhileStmt {
    pub condition: Expr,
    pub body: Block,
}
