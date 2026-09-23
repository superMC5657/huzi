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
    Array(Box<Type>, usize), // 定长数组: [元素类型; 长度]
    Tuple(Vec<Type>),
    /// 堆分配智能指针:`Box<Node>`(具名结构体)、`Box<i32>` 等
    /// 基础类型(`i32`/`i64`/`f64`/`bool`/`str`)或嵌套
    /// `Box<Box<T>>`(每层仍是指针,最内层为结构体或基础类型);
    /// `vec` 字段类型不受支持。
    Box(Box<Type>),
    Generic(String),
    Applied(String, Vec<Type>),
    /// 一等函数类型: `fn(T1, T2) -> Ret`
    Fn(Vec<Type>, Box<Type>),
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
            Type::Fn(params, ret) => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
        }
    }
}

impl Type {
    /// 将类型树中出现的 `Self` 替换为具体的类型名。
    pub fn substitute_self(&self, target: &str) -> Type {
        match self {
            Type::Named(name) if name == "Self" => Type::Named(target.to_string()),
            Type::Box(inner) => Type::Box(Box::new(inner.substitute_self(target))),
            Type::Array(elem, len) => Type::Array(Box::new(elem.substitute_self(target)), *len),
            Type::Tuple(elems) => {
                Type::Tuple(elems.iter().map(|e| e.substitute_self(target)).collect())
            }
            Type::Applied(name, args) => {
                let n = if name == "Self" {
                    target.to_string()
                } else {
                    name.clone()
                };
                Type::Applied(n, args.iter().map(|a| a.substitute_self(target)).collect())
            }
            Type::Fn(params, ret) => Type::Fn(
                params.iter().map(|p| p.substitute_self(target)).collect(),
                Box::new(ret.substitute_self(target)),
            ),
            other => other.clone(),
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
