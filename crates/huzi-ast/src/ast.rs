use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Str,
    Char,
    Unit,
    Named(String),
    Array(Box<Type>, usize), // Array<ElementType, Size>
    Tuple(Vec<Type>),
    /// 堆分配智能指针:`Box<Node>`(具名结构体)或嵌套 `Box<Box<Node>>`
    /// (每层仍是指针,最内层须为具名结构体);`Box<i32>` 等非结构体
    /// 直接包装与 `vec` 字段类型不受支持。
    Box(Box<Type>),
    Generic(String),
    Applied(String, Vec<Type>),
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::I32 => write!(f, "i32"),
            Type::I64 => write!(f, "i64"),
            Type::U32 => write!(f, "u32"),
            Type::U64 => write!(f, "u64"),
            Type::F32 => write!(f, "f32"),
            Type::F64 => write!(f, "f64"),
            Type::Bool => write!(f, "bool"),
            Type::Str => write!(f, "str"),
            Type::Char => write!(f, "char"),
            Type::Unit => write!(f, "()"),
            Type::Named(name) => write!(f, "{}", name),
            Type::Array(elem_type, size) => write!(f, "[{}; {}]", elem_type, size),
            Type::Tuple(elems) => {
                write!(f, "(")?;
                for (i, elem) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", elem)?;
                }
                write!(f, ")")
            }
            Type::Box(inner) => write!(f, "Box<{}>", inner),
            Type::Generic(name) => write!(f, "{}", name),
            Type::Applied(name, args) => {
                write!(f, "{}<", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ">")
            }
        }
    }
}

/// 源码位置:起止区间(1-based 行列号),与 lexer 的 `SpannedToken` 对齐。
/// `line`/`column` 为起始位置(保留旧单点构造兼容),
/// `end_line`/`end_column` 为结束位置(词法上取末 token 列 +1,无精确
/// 末位置时退化为起始位置,保证 `end >= start`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl Span {
    /// 单点构造:结束位置退化为起始位置(零宽区间)。
    pub fn new(line: usize, column: usize) -> Self {
        Self {
            line,
            column,
            end_line: line,
            end_column: column,
        }
    }

    /// 区间构造:结束位置小于起始位置时钳制为起始位置,
    /// 保证区间永不倒置(`end >= start`)。
    pub fn new_range(
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
    ) -> Self {
        if (end_line, end_column) < (start_line, start_column) {
            Self {
                line: start_line,
                column: start_column,
                end_line: start_line,
                end_column: start_column,
            }
        } else {
            Self {
                line: start_line,
                column: start_column,
                end_line,
                end_column,
            }
        }
    }

    /// 起始行(兼容 accessor,供 codegen DWARF 行号使用)。
    pub fn start_line(&self) -> usize {
        self.line
    }

    /// 起始列(兼容 accessor,供 codegen DWARF 列号使用)。
    pub fn start_column(&self) -> usize {
        self.column
    }

    /// 是否为零宽(单点)区间。
    pub fn is_empty(&self) -> bool {
        (self.line, self.column) == (self.end_line, self.end_column)
    }
}

/// 携带源码位置的 AST 节点包裹。语句级粒度即可满足
/// 调试行号/断点/单步的需求。
#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    /// 旧单点构造(向后兼容):结束位置退化为起始位置。
    pub fn new(node: T, line: usize, column: usize) -> Self {
        Self {
            node,
            span: Span::new(line, column),
        }
    }

    /// 区间构造:记录语句起止位置。
    pub fn with_range(
        node: T,
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
    ) -> Self {
        Self {
            node,
            span: Span::new_range(start_line, start_column, end_line, end_column),
        }
    }

    /// 沿用已有区间(elif 折叠等合成节点透传外层区间)。
    pub fn with_span(node: T, span: Span) -> Self {
        Self { node, span }
    }
}

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
    Expr(ExprStmt),
    Return(ReturnStmt),
    Break,
    Continue,
    Block(Block),
    If(IfStmt),
    For(ForStmt),
    While(WhileStmt),
    Defer(Box<Spanned<Stmt>>),
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
    pub variants: Vec<EnumVariant>,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    /// Payload types: `Red` has none, `Ok(i32)` has one,
    /// `Pair(i32, str)` has several.
    pub payloads: Vec<Type>,
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
    /// `for i in start..end`
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

#[derive(Debug, Clone)]
pub enum Expr {
    Literal(Literal),
    Ident(String),
    Binary(BinaryExpr),
    Unary(UnaryExpr),
    Call(CallExpr),
    Assign(AssignExpr),
    ArrayIndex(ArrayIndexExpr),
    ArrayLiteral(Vec<Expr>),
    TupleLiteral(Vec<Expr>),
    /// 空 vec 构造:`vec<T>()`(元素类型由尖括号显式指定,零长)。
    VecEmpty(Type),
    /// 堆分配构造:`box(expr)`(求值后 malloc 存入,返回 `Box<T>`)。
    BoxAlloc(Box<Expr>),
    /// 空指针字面量:`null`(只能出现在 `Box<T>` 期望位置)。
    Null,
    If(IfExpr),
    FieldAccess(FieldAccessExpr),
    StructLiteral(StructLiteralExpr),
    EnumConstruct(EnumConstructExpr),
    Match(MatchExpr),
}

/// Enum variant construction: `Color::Red` or `Result::Ok(42)`
#[derive(Debug, Clone)]
pub struct EnumConstructExpr {
    pub enum_name: String,
    pub variant: String,
    pub args: Vec<Expr>,
}

/// `match scrutinee { pattern => body, ... }` used as an expression.
#[derive(Debug, Clone)]
pub struct MatchExpr {
    pub scrutinee: Box<Expr>,
    pub arms: Vec<MatchArm>,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    /// `Enum::Variant`, `Enum::Variant(x)` or `Enum::Variant(x, y)` — binds
    /// the payload fields to variables inside the arm body.
    Variant {
        enum_name: String,
        variant: String,
        bindings: Vec<String>,
    },
    /// `_` — matches anything.
    Wildcard,
}

#[derive(Debug, Clone)]
pub struct FieldAccessExpr {
    pub base: Box<Expr>,
    pub field: String,
}

/// Struct instantiation: `Point { x: 1, y: 2 }` or `Pair<i32, str> { key: 1, val: "a" }`
#[derive(Debug, Clone)]
pub struct StructLiteralExpr {
    pub name: String,
    pub fields: Vec<(String, Expr)>,
    pub type_args: Vec<Type>,
}

/// If used as an expression: `let m = if cond { a } else { b }`
#[derive(Debug, Clone)]
pub struct IfExpr {
    pub condition: Box<Expr>,
    pub then_branch: Block,
    pub else_branch: Block,
}

#[derive(Debug, Clone)]
pub struct ArrayIndexExpr {
    pub array: Box<Expr>,
    pub index: Box<Expr>,
}

#[derive(Debug, Clone)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Char(char),
}

#[derive(Debug, Clone)]
pub struct BinaryExpr {
    pub left: Box<Expr>,
    pub operator: BinOp,
    pub right: Box<Expr>,
}

#[derive(Debug, Clone)]
pub struct UnaryExpr {
    pub operator: UnOp,
    pub operand: Box<Expr>,
}

#[derive(Debug, Clone)]
pub struct CallExpr {
    pub callee: Box<Expr>,
    pub arguments: Vec<Expr>,
    pub type_args: Vec<Type>,
}

#[derive(Debug, Clone)]
pub struct AssignExpr {
    pub target: Box<Expr>,
    pub value: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}
