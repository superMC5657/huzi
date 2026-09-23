use super::stmt::Block;
use super::types::Type;

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
    MethodCall(MethodCallExpr),
    /// 后缀 `?`:`expr?` — Result<T> 解包,失败时从当前函数提前返回。
    Try(TryExpr),
    /// 闭包/匿名函数表达式：`|x, y| expr`、`|x| { ... }`、`|| expr`
    Closure(ClosureExpr),
}

/// 闭包表达式定义
#[derive(Debug, Clone)]
pub struct ClosureExpr {
    pub params: Vec<ClosureParam>,
    pub return_type: Option<Type>,
    pub body: ClosureBody,
}

/// 闭包形参定义
#[derive(Debug, Clone)]
pub struct ClosureParam {
    pub name: String,
    pub param_type: Option<Type>,
}

/// 闭包函数体（单表达式或完整语句块）
#[derive(Debug, Clone)]
pub enum ClosureBody {
    Expr(Box<Expr>),
    Block(Block),
}

/// `expr?` — 成功取 `value` 字段,失败提前返回整个 Result 值。
#[derive(Debug, Clone)]
pub struct TryExpr {
    pub inner: Box<Expr>,
}

#[derive(Debug, Clone)]
pub struct MethodCallExpr {
    pub receiver: Box<Expr>,
    pub method: String,
    pub arguments: Vec<Expr>,
}

/// 枚举变体构造：`Color::Red`、`Result::Ok(42)` 或 `Result<i32, str>::Ok(42)`
#[derive(Debug, Clone)]
pub struct EnumConstructExpr {
    pub enum_name: String,
    pub variant: String,
    pub args: Vec<Expr>,
    pub type_args: Vec<Type>,
}

/// 作为表达式使用的 match 表达式：`match scrutinee { pattern => body, ... }`
#[derive(Debug, Clone)]
pub struct MatchExpr {
    pub scrutinee: Box<Expr>,
    pub arms: Vec<MatchArm>,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    /// `Enum::Variant`、`Enum::Variant(x)` 或 `Enum::Variant(x, y)` — 将
    /// 负载字段绑定到分支体内的局部变量。
    Variant {
        enum_name: String,
        variant: String,
        bindings: Vec<String>,
    },
    /// 字面量模式（如 `0`, `42`, `-1`, `true`, `'a'`, `"hello"`）
    Literal(Literal),
    /// 变量绑定模式（如 `x`、`val`）
    Variable(String),
    /// `_` — 通配符，匹配任意值。
    Wildcard,
}

#[derive(Debug, Clone)]
pub struct FieldAccessExpr {
    pub base: Box<Expr>,
    pub field: String,
}

/// 结构体实例化：`Point { x: 1, y: 2 }` 或 `Pair<i32, str> { key: 1, val: "a" }`
#[derive(Debug, Clone)]
pub struct StructLiteralExpr {
    pub name: String,
    pub fields: Vec<(String, Expr)>,
    pub type_args: Vec<Type>,
}

/// 作为表达式使用的 if：`let m = if cond { a } else { b }`
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
    /// 前缀解引用:`*b` 穿透全部 Box 层直达最内层值(空指针逐层校验)。
    Deref,
}
