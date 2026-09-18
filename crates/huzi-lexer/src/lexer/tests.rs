use super::*;
use crate::token::Token;

fn tokenize(src: &str) -> Vec<SpannedToken> {
    Lexer::new(src.to_string())
        .tokenize()
        .unwrap_or_else(|e| panic!("unexpected lex error: {}", e))
}

#[test]
fn keywords_identifiers_and_integers() {
    let tokens = tokenize("let x = 42");
    assert_eq!(
        tokens,
        vec![
            SpannedToken { token: Token::Let, line: 1, column: 1 },
            SpannedToken { token: Token::Ident("x".to_string()), line: 1, column: 5 },
            SpannedToken { token: Token::Equal, line: 1, column: 7 },
            SpannedToken { token: Token::Int(42), line: 1, column: 9 },
            SpannedToken { token: Token::Eof, line: 1, column: 11 },
        ]
    );
}

#[test]
fn float_string_and_char_literals() {
    let tokens = tokenize("2.5 \"hi\" 'a'");
    assert_eq!(tokens[0].token, Token::Float(2.5));
    assert_eq!(tokens[1].token, Token::String("hi".to_string()));
    assert_eq!(tokens[2].token, Token::Char('a'));
}

#[test]
fn comparison_and_logic_operators() {
    let tokens = tokenize("== != <= >= && ||");
    assert_eq!(tokens[0].token, Token::EqualEqual);
    assert_eq!(tokens[1].token, Token::BangEqual);
    assert_eq!(tokens[2].token, Token::LessEqual);
    assert_eq!(tokens[3].token, Token::GreaterEqual);
    assert_eq!(tokens[4].token, Token::AmpAmp);
    assert_eq!(tokens[5].token, Token::BarBar);
}

#[test]
fn spans_track_line_and_column() {
    let tokens = tokenize("let\n  y");
    assert_eq!(tokens[0].token, Token::Let);
    assert_eq!((tokens[0].line, tokens[0].column), (1, 1));
    assert_eq!(tokens[1].token, Token::Ident("y".to_string()));
    assert_eq!((tokens[1].line, tokens[1].column), (2, 3));
}

#[test]
fn line_and_hash_comments_are_skipped() {
    let tokens = tokenize("// leading note\nx # trailing note\n");
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].token, Token::Ident("x".to_string()));
    assert_eq!(tokens[0].line, 2);
}

#[test]
fn illegal_character_reports_position() {
    let err = Lexer::new("let x = @".to_string())
        .tokenize()
        .expect_err("'@' must be rejected");
    assert_eq!((err.line(), err.column()), (1, 9));
}
