use super::*;

#[test]
fn test_fmt_idempotent_basic() {
    let src = r#"fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    let mut x = 10
    let y = 20
    let z = add(x, y)
    print("sum =", z)
    return 0
}
"#;
    let formatted = format_source(src).expect("format should succeed");
    let formatted_again = format_source(&formatted).expect("second format should succeed");
    assert_eq!(formatted, formatted_again, "Formatting must be idempotent");
}

#[test]
fn test_fmt_idempotent_struct_and_enum() {
    let src = r#"struct Point {
    x: i32,
    y: i32,
}

enum Shape {
    Circle(f64),
    Rect(i32, i32),
}

fn main() -> i32 {
    let p = Point { x: 1, y: 2 }
    let s = Shape::Circle(3.14)
    return 0
}
"#;
    let formatted = format_source(src).expect("format should succeed");
    let formatted_again = format_source(&formatted).expect("second format should succeed");
    assert_eq!(formatted, formatted_again, "Formatting must be idempotent");
}

#[test]
fn test_fmt_idempotent_control_flow_and_defer() {
    let src = r#"fn compute(n: i32) -> i32 {
    defer print("cleanup compute")
    if n > 10 {
        return n * 2
    } elif n > 5 {
        return n + 5
    } else {
        return 0
    }
}

fn main() -> i32 {
    let mut i = 0
    while i < 3 {
        defer print("loop defer")
        i = i + 1
    }
    for k in 0..5 {
        print(k)
    }
    return compute(i)
}
"#;
    let formatted = format_source(src).expect("format should succeed");
    let formatted_again = format_source(&formatted).expect("second format should succeed");
    assert_eq!(formatted, formatted_again, "Formatting must be idempotent");
}

#[test]
fn test_fmt_operator_precedence() {
    let src = r#"fn main() -> i32 {
    let a = 1 + 2 * 3
    let b = (1 + 2) * 3
    let c = a > 0 && b < 10 || a == b
    let d = -(a + b)
    return 0
}
"#;
    let formatted = format_source(src).expect("format should succeed");
    let formatted_again = format_source(&formatted).expect("second format should succeed");
    assert_eq!(formatted, formatted_again, "Formatting must be idempotent");
}

#[test]
fn test_fmt_if_expr() {
    let src = r#"fn main() -> i32 {
    let grade = if avg >= 90 {
        "A"
    } elif avg >= 80 {
        "B"
    } elif avg >= 70 {
        "C"
    } elif avg >= 60 {
        "D"
    } else {
        "F"
    }
    return 0
}
"#;
    let formatted = format_source(src).expect("format should succeed");
    println!("FORMATTED:\n{}", formatted);
    let formatted_again = format_source(&formatted).expect("second format should succeed");
    assert_eq!(formatted, formatted_again);
}
