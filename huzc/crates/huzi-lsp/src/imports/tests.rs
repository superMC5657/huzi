//! 跳转单测(纯搬移自 `imports.rs`,与实现同级锁定)。

use super::*;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// 相对路径试探的纯逻辑:点分名 -> 子路径(供单测锁定映射规则)。
fn import_to_relpath(import_name: &str) -> PathBuf {
    let rel: PathBuf = import_name.split('.').collect();
    rel.with_extension("hz")
}

/// 并行安全的唯一临时目录。
fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "huzi_lsp_{tag}_{}_{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(dir.join("mods")).expect("create temp dirs");
    dir
}

#[test]
fn dotted_name_maps_to_nested_hz_path() {
    // Given: 点分 import 名
    // When: 转相对路径
    // Then: 段转子目录并加 .hz 后缀
    assert_eq!(
        import_to_relpath("mods.helpers"),
        Path::new("mods/helpers.hz")
    );
    assert_eq!(import_to_relpath("math"), Path::new("math.hz"));
}

#[test]
fn import_line_resolves_to_companion_file() {
    // Given: 临时目录下 main.hz 与 mods/helpers.hz 并存
    let dir = unique_dir("import");
    let main = dir.join("main.hz");
    std::fs::write(&main, "import mods.helpers\n").unwrap();
    std::fs::write(
        dir.join("mods/helpers.hz"),
        "fn add(a: i32, b: i32) -> i32 {\n return a + b\n}\n",
    )
    .unwrap();
    let uri = Uri::from_file_path(&main).expect("file uri");
    // When: 解析 import 名
    let target = resolve_import_uri(&uri, "mods.helpers");
    // Then: 命中 mods/helpers.hz
    assert_eq!(
        target.expect("resolved").to_file_path().expect("path").into_owned(),
        dir.join("mods/helpers.hz")
    );
}

#[test]
fn import_jump_returns_file_head() {
    // Given: 同上布局,光标落在 import 行词上
    let dir = unique_dir("jump");
    let main = dir.join("main.hz");
    let text = "import mods.helpers\nhelpers::add(1, 2)\n";
    std::fs::write(&main, text).unwrap();
    std::fs::write(
        dir.join("mods/helpers.hz"),
        "fn add(a: i32, b: i32) -> i32 {\n return a + b\n}\n",
    )
    .unwrap();
    let uri = Uri::from_file_path(&main).expect("file uri");
    // When: 请求 import 行定义(行 0,列 10 落在 helpers 段)
    let loc = import_jump_location(
        text,
        &uri,
        Position {
            line: 0,
            character: 10,
        },
    )
    .expect("import jump");
    // Then: 跳到目标文件头(0 行 0 列)
    assert_eq!(loc.range.start.line, 0);
    assert_eq!(loc.range.start.character, 0);
    assert_eq!(
        loc.uri.to_file_path().expect("path").into_owned(),
        dir.join("mods/helpers.hz")
    );
}

#[test]
fn module_fn_jump_lands_on_fn_symbol() {
    // Given: 主文件经 helpers::add 调用模块函数
    let dir = unique_dir("modfn");
    let main = dir.join("main.hz");
    let text = "import mods.helpers\nlet s = helpers::add(3, 4)\n";
    std::fs::write(&main, text).unwrap();
    std::fs::write(
        dir.join("mods/helpers.hz"),
        "fn add(a: i32, b: i32) -> i32 {\n return a + b\n}\n",
    )
    .unwrap();
    let uri = Uri::from_file_path(&main).expect("file uri");
    // When: 在 helpers::add 处请求定义(行 1,列 20 落在 add)
    let loc = module_fn_location(
        text,
        &uri,
        Position {
            line: 1,
            character: 20,
        },
    )
    .expect("module fn jump");
    // Then: 落到模块文件首行 fn 定义处
    assert_eq!(
        loc.uri.to_file_path().expect("path").into_owned(),
        dir.join("mods/helpers.hz")
    );
    assert_eq!(loc.range.start.line, 0);
}

#[test]
fn missing_module_falls_back_to_none() {
    // Given: import 指向不存在的文件
    let dir = unique_dir("missing");
    let main = dir.join("main.hz");
    std::fs::write(&main, "import no.such\n").unwrap();
    let uri = Uri::from_file_path(&main).expect("file uri");
    // When: 请求跳转
    // Then: 返回 None(调用方回退同文件),不抛错
    assert!(resolve_import_uri(&uri, "no.such").is_none());
    assert!(import_jump_location(
        "import no.such\n",
        &uri,
        Position {
            line: 0,
            character: 8,
        }
    )
    .is_none());
}

#[test]
fn import_jump_on_blank_returns_none() {
    // Given: import 行但光标落在空白处(L1 锁定:空白不跳)
    let dir = unique_dir("blank");
    let main = dir.join("main.hz");
    let text = "import mods.helpers\n";
    std::fs::write(&main, text).unwrap();
    std::fs::write(
        dir.join("mods/helpers.hz"),
        "fn add(a: i32, b: i32) -> i32 {\n return a + b\n}\n",
    )
    .unwrap();
    let uri = Uri::from_file_path(&main).expect("file uri");
    // When: 光标落在 `import` 后的空格处(行 0,列 6)
    // Then: 返回 None,不抛错
    assert!(import_jump_location(
        text,
        &uri,
        Position {
            line: 0,
            character: 6,
        }
    )
    .is_none());
}

#[test]
fn module_fn_jump_outside_double_colon_returns_none() {
    // Given: 普通调用行(L1 锁定:非 `A::b` 处不跳)
    let dir = unique_dir("outside");
    let main = dir.join("main.hz");
    let text = "import mods.helpers\nlet s = 1\n";
    std::fs::write(&main, text).unwrap();
    std::fs::write(
        dir.join("mods/helpers.hz"),
        "fn add(a: i32, b: i32) -> i32 {\n return a + b\n}\n",
    )
    .unwrap();
    let uri = Uri::from_file_path(&main).expect("file uri");
    // When: 光标落在无双冒号的行(行 1,列 5)
    // Then: 返回 None,不抛错
    assert!(module_fn_location(
        text,
        &uri,
        Position {
            line: 1,
            character: 5,
        }
    )
    .is_none());
}

#[test]
fn invalid_dotted_import_returns_none() {
    // Given: 非法点分名(L1 锁定:非法名不跳)
    let dir = unique_dir("invalid");
    let main = dir.join("main.hz");
    std::fs::write(&main, "import foo..bar\n").unwrap();
    let uri = Uri::from_file_path(&main).expect("file uri");
    // When: 请求跳转
    // Then: 返回 None,不抛错
    assert!(import_jump_location(
        "import foo..bar\n",
        &uri,
        Position {
            line: 0,
            character: 8,
        }
    )
    .is_none());
}

#[test]
fn std_stem_layout_jumps_to_fn_symbol() {
    // Given: 仿 huzi-src 的 std/json/lib.hz 布局(L3)
    let dir = unique_dir("stdjump");
    let main = dir.join("main.hz");
    let text = "import std.json\nlet s = json::get(\"a\", \"b\")\n";
    std::fs::write(&main, text).unwrap();
    let stem = dir.join("huzi-src").join("std").join("json");
    std::fs::create_dir_all(&stem).unwrap();
    std::fs::write(
        stem.join("lib.hz"),
        "fn get(s: str, path: str) -> (bool, str) {\n return (false, \"\")\n}\n",
    )
    .unwrap();
    let uri = Uri::from_file_path(&main).expect("file uri");
    // When: 解析 import 并在 json::get 处请求定义
    let target = resolve_import_uri(&uri, "std.json").expect("std hit");
    assert_eq!(
        target.to_file_path().expect("path").into_owned(),
        stem.join("lib.hz")
    );
    let loc = module_fn_location(
        text,
        &uri,
        Position {
            line: 1,
            character: 16,
        },
    )
    .expect("fn jump");
    // Then: 落到 lib.hz 首行 fn 定义处
    assert_eq!(
        loc.uri.to_file_path().expect("path").into_owned(),
        stem.join("lib.hz")
    );
    assert_eq!(loc.range.start.line, 0);
}

#[test]
fn std_struct_symbol_jump_prefers_type() {
    // Given: 模块内结构体定义(L3 类型次选)
    let dir = unique_dir("stdtype");
    let main = dir.join("main.hz");
    let text = "import mods.shapes\nlet p = shapes::Point\n";
    std::fs::write(&main, text).unwrap();
    std::fs::write(
        dir.join("mods/shapes.hz"),
        "struct Point { x: i32, y: i32 }\n",
    )
    .unwrap();
    let uri = Uri::from_file_path(&main).expect("file uri");
    // When: 在 shapes::Point 处请求定义
    let loc = module_fn_location(
        text,
        &uri,
        Position {
            line: 1,
            character: 20,
        },
    )
    .expect("type jump");
    // Then: 落到模块文件结构体定义处
    assert_eq!(
        loc.uri.to_file_path().expect("path").into_owned(),
        dir.join("mods/shapes.hz")
    );
    assert_eq!(loc.range.start.line, 0);
}
