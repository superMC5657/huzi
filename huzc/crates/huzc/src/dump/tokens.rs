use huzc::cli::Args;
use huzi_lexer::{Lexer, SpannedToken, Token};

use crate::die;
use crate::pipeline::front::read_source;

/// `--dump-tokens` 的 token 种类：与 hzlex（examples/hzlex/src/lexer.hz）一致。
/// 取值仅为 kw | ident | int | float | string | char | punct | eof。
fn dump_token_kind(token: &Token) -> &'static str {
    match token {
        Token::Fn
        | Token::Struct
        | Token::Enum
        | Token::Match
        | Token::Let
        | Token::Mut
        | Token::If
        | Token::Else
        | Token::Elif
        | Token::For
        | Token::In
        | Token::While
        | Token::Return
        | Token::Import
        | Token::Export
        | Token::Break
        | Token::Continue
        | Token::Defer
        | Token::Trait
        | Token::Impl
        | Token::True
        | Token::False
        | Token::Weak => "kw",
        Token::Ident(_) => "ident",
        Token::Int(_) => "int",
        Token::Float(_) => "float",
        Token::String(_) | Token::FString(_) => "string",
        Token::Char(_) => "char",
        Token::Eof => "eof",
        _ => "punct",
    }
}

/// `--dump-tokens` 的 token 文本：与 hzlex 的 texts 平行数组一致。
/// kw/punct 为源码拼写；ident 为值显示（即源码拼写）；int/float 由调用方
/// 经源码切片取源码拼写（见 `format_tokens`），此处仅为取不到切片时的
/// 回退（值显示）；string 为解转义后原文
/// （去除首尾引号，f-string 去掉前缀 f）；char 为码点十进制；eof 为空串。
fn dump_token_text(token: &Token) -> String {
    match token {
        Token::Fn => "fn".to_string(),
        Token::Struct => "struct".to_string(),
        Token::Enum => "enum".to_string(),
        Token::Match => "match".to_string(),
        Token::Let => "let".to_string(),
        Token::Mut => "mut".to_string(),
        Token::If => "if".to_string(),
        Token::Else => "else".to_string(),
        Token::Elif => "elif".to_string(),
        Token::For => "for".to_string(),
        Token::In => "in".to_string(),
        Token::While => "while".to_string(),
        Token::Return => "return".to_string(),
        Token::Import => "import".to_string(),
        Token::Export => "export".to_string(),
        Token::Break => "break".to_string(),
        Token::Continue => "continue".to_string(),
        Token::Defer => "defer".to_string(),
        Token::Trait => "trait".to_string(),
        Token::Impl => "impl".to_string(),
        Token::True => "true".to_string(),
        Token::False => "false".to_string(),
        Token::Weak => "weak".to_string(),
        Token::Ident(s) | Token::String(s) | Token::FString(s) => s.clone(),
        Token::Int(n) => n.to_string(),
        Token::Float(n) => n.to_string(),
        Token::Char(c) => (*c as u32).to_string(),
        Token::Eof => String::new(),
        Token::Plus => "+".to_string(),
        Token::PlusEq => "+=".to_string(),
        Token::Minus => "-".to_string(),
        Token::MinusEq => "-=".to_string(),
        Token::Star => "*".to_string(),
        Token::StarEq => "*=".to_string(),
        Token::Slash => "/".to_string(),
        Token::SlashEq => "/=".to_string(),
        Token::Percent => "%".to_string(),
        Token::PercentEq => "%=".to_string(),
        Token::Equal => "=".to_string(),
        Token::EqualEqual => "==".to_string(),
        Token::Bang => "!".to_string(),
        Token::BangEqual => "!=".to_string(),
        Token::Less => "<".to_string(),
        Token::LessEqual => "<=".to_string(),
        Token::Greater => ">".to_string(),
        Token::GreaterEqual => ">=".to_string(),
        Token::AmpAmp => "&&".to_string(),
        Token::BarBar => "||".to_string(),
        Token::Pipe => "|".to_string(),
        Token::Question => "?".to_string(),
        Token::LParen => "(".to_string(),
        Token::RParen => ")".to_string(),
        Token::LBrace => "{".to_string(),
        Token::RBrace => "}".to_string(),
        Token::LBracket => "[".to_string(),
        Token::RBracket => "]".to_string(),
        Token::Comma => ",".to_string(),
        Token::Colon => ":".to_string(),
        Token::PathSep => "::".to_string(),
        Token::Semi => ";".to_string(),
        Token::Arrow => "->".to_string(),
        Token::FatArrow => "=>".to_string(),
        Token::Dot => ".".to_string(),
        Token::DotDot => "..".to_string(),
    }
}

/// 将 token 序列格式化为 hzlex 兼容的 dump 文本。
/// 每行格式为 `{line}:{col} {kind} {text}`（单空格分隔，含 eof 行；
/// eof 行 text 为空故行尾带一个空格），行尾统一 `\n`。
/// int/float 的 text 取源码拼写（逐字节一致，如 `007`/`0.0` 不归一化），
/// 经行列号反推源码切片得到；反推失败时回退为值显示。
fn format_tokens(tokens: &[SpannedToken], source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let line_starts = build_line_starts(&chars);
    let mut out = String::new();
    for st in tokens {
        let kind = dump_token_kind(&st.token);
        let text = match &st.token {
            Token::Int(_) => number_lexeme(&chars, &line_starts, st.line, st.column, false)
                .unwrap_or_else(|| dump_token_text(&st.token)),
            Token::Float(_) => number_lexeme(&chars, &line_starts, st.line, st.column, true)
                .unwrap_or_else(|| dump_token_text(&st.token)),
            _ => dump_token_text(&st.token),
        };
        out.push_str(&format!("{}:{} {} {}\n", st.line, st.column, kind, text));
    }
    out
}

/// 按 char 计的行首索引表：`line_starts[i]` 为第 `i+1` 行首个字符的下标。
/// 与词法器口径一致（`\n` 换行，`\r` 只占一列不换行）。
fn build_line_starts(chars: &[char]) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// 从源码切片取数字字面量的源码拼写：int 取前导 `[0-9]+`，float 取
/// `[0-9]+.[0-9]+`。行列号为词法器的 1 基 char 口径；越界或无数字时
/// 返回 `None` 由调用方回退为值显示。
fn number_lexeme(
    chars: &[char],
    line_starts: &[usize],
    line: usize,
    column: usize,
    is_float: bool,
) -> Option<String> {
    if line == 0 || column == 0 || line > line_starts.len() {
        return None;
    }
    let mut idx = line_starts[line - 1] + (column - 1);
    if idx >= chars.len() {
        return None;
    }
    let start = idx;
    while idx < chars.len() && chars[idx].is_ascii_digit() {
        idx += 1;
    }
    if is_float {
        if idx >= chars.len() || chars[idx] != '.' {
            return None;
        }
        idx += 1;
        let frac_start = idx;
        while idx < chars.len() && chars[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx == frac_start {
            return None;
        }
    }
    if idx == start {
        return None;
    }
    Some(chars[start..idx].iter().collect())
}

/// 执行 `--dump-tokens -i <file>`：词法分析并逐行打印后直接返回。
/// 词法失败沿用现有错误路径（stderr 渲染 + 非零退出），不做错误恢复。
pub(crate) fn run_dump_tokens(args: &Args) {
    let input = match &args.input {
        Some(i) => i,
        None => {
            eprintln!("error: '--dump-tokens' requires '--input <file.hz>'");
            std::process::exit(1);
        }
    };
    let source = read_source(input);
    let tokens = Lexer::new(source.clone())
        .tokenize()
        .unwrap_or_else(|e| die(huzi_error::render(&e, &source, "Lex error")));
    print!("{}", format_tokens(&tokens, &source));
}

#[cfg(test)]
mod dump_tokens_tests {
    use super::*;

    /// dump 行与 hzlex 输出一致：覆盖 kw/ident/float/转义 string/range 点。
    /// 期望行即 hzlex `lex()` 对同一源码的 kinds/texts/lines/cols
    /// 按 `行:列 类型 文本` 打印的结果（逐字节一致，eof 行尾带空格）。
    #[test]
    fn dump_matches_hzlex_format() {
        let src = "let s = \"a\\tb\"; let f = 3.14; let r = 1..2".to_string();
        let tokens = Lexer::new(src.clone())
            .tokenize()
            .expect("test snippet must lex cleanly");
        let got = format_tokens(&tokens, &src);
        let tab = '\t';
        let expected = format!(
            "1:1 kw let\n1:5 ident s\n1:7 punct =\n1:9 string a{tab}b\n1:15 punct ;\n1:17 kw let\n1:21 ident f\n1:23 punct =\n1:25 float 3.14\n1:29 punct ;\n1:31 kw let\n1:35 ident r\n1:37 punct =\n1:39 int 1\n1:40 punct ..\n1:42 int 2\n1:43 eof \n"
        );
        assert_eq!(got, expected);
    }

    /// 数字拼写逐字节一致：前导零与小数尾零不做值归一化。
    /// `007` 不得归一为 `7`，`0.0`/`144.0`/`2.0` 不得归一为 `0`/`144`/`2`。
    #[test]
    fn dump_preserves_numeric_spelling() {
        let src = "let a = 007; let b = 0.0; let c = 144.0; let d = 2.0".to_string();
        let tokens = Lexer::new(src.clone())
            .tokenize()
            .expect("test snippet must lex cleanly");
        let got = format_tokens(&tokens, &src);
        let expected = "1:1 kw let\n1:5 ident a\n1:7 punct =\n1:9 int 007\n1:12 punct ;\n1:14 kw let\n1:18 ident b\n1:20 punct =\n1:22 float 0.0\n1:25 punct ;\n1:27 kw let\n1:31 ident c\n1:33 punct =\n1:35 float 144.0\n1:40 punct ;\n1:42 kw let\n1:46 ident d\n1:48 punct =\n1:50 float 2.0\n1:53 eof \n";
        assert_eq!(got, expected);
    }
}
