//! stditems 单测:入口探查与补全项(纯搬移自 `stditems.rs`,与实现同级锁定)。

use super::*;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tower_lsp_server::ls_types::Uri;

use crate::analysis::parse_and_collect;

/// 并行安全的唯一临时目录。
fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "huzi_lsp_std_{tag}_{}_{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(dir.join("mods")).expect("create temp dirs");
    dir
}

#[test]
fn lib_entry_wins_over_conventional_candidates() {
    // Given: stem 目录下 huzi.toml 指定 custom_entry.hz,且另有 lib.hz
    let dir = unique_dir("libentry");
    let stem = dir.join("mods").join("greeter");
    std::fs::create_dir_all(&stem).unwrap();
    std::fs::write(
        stem.join("huzi.toml"),
        "[package]\nname = \"greeter\"\nlib_entry = \"custom_entry.hz\"\n",
    )
    .unwrap();
    std::fs::write(stem.join("custom_entry.hz"), "fn hi() -> i32 {\n return 1\n}\n")
        .unwrap();
    std::fs::write(stem.join("lib.hz"), "fn other() -> i32 {\n return 2\n}\n").unwrap();
    // When: 解析 mods.greeter
    let hit = resolve_in_roots("mods.greeter", &[dir.clone()])
        .expect("lib_entry hit");
    // Then: 命中 custom_entry.hz 而非 lib.hz
    assert_eq!(hit, stem.join("custom_entry.hz"));
}

#[test]
fn stem_lib_fallback_without_toml() {        // Given: 无 toml 的 stem 目录,仅有 lib.hz
    let dir = unique_dir("stemlib");
    let stem = dir.join("mods").join("plain");
    std::fs::create_dir_all(&stem).unwrap();
    std::fs::write(stem.join("lib.hz"), "fn go() -> i32 {\n return 1\n}\n").unwrap();
    // When: 解析 mods.plain
    let hit =
        resolve_in_roots("mods.plain", &[dir.clone()]).expect("stem hit");
    // Then: 命中 lib.hz
    assert_eq!(hit, stem.join("lib.hz"));
}

#[test]
fn current_file_dir_root_resolves_sibling_module() {
    // Given: main.hz 与 mods/helpers.hz 并存
    let dir = unique_dir("curdir");
    let main = dir.join("main.hz");
    std::fs::write(&main, "import mods.helpers\n").unwrap();
    std::fs::write(
        dir.join("mods/helpers.hz"),
        "fn add(a: i32, b: i32) -> i32 {\n return a + b\n}\n",
    )
    .unwrap();
    let uri = Uri::from_file_path(&main).expect("file uri");
    // When: 经当前文件目录根解析
    let hit = resolve_in_roots(
        "mods.helpers",
        &module_file_roots(Some(&uri)),
    )
    .expect("hit");
    // Then: 命中同目录模块文件
    assert_eq!(hit, dir.join("mods/helpers.hz"));
}

#[test]
fn bind_completion_reads_module_symbols() {
    // Given: 主文件 import mods.helpers,模块含 add/Point
    let dir = unique_dir("bind");
    std::fs::write(
        dir.join("mods/helpers.hz"),
        "struct Point { x: i32 }\nfn add(a: i32, b: i32) -> i32 {\n return a + b\n}\n",
    )
    .unwrap();
    let text = "import mods.helpers\nhelpers::";
    // When: 取 helpers:: 补全(注入临时根)
    let items = bind_items_with_roots(text, "helpers", "", &[dir]);
    let labels: Vec<&str> =
        items.iter().map(|i| i.label.as_str()).collect();
    // Then: 含 add 与 Point,不含局部变量
    assert!(labels.contains(&"add"), "{labels:?}");
    assert!(labels.contains(&"Point"), "{labels:?}");
}

#[test]
fn bind_completion_includes_target_impl() {
    // Given: import std.json,目标含 Value/parse + 固有 stringify + trait load
    let dir = unique_dir("bindimpl");
    std::fs::create_dir_all(dir.join("std")).unwrap();
    std::fs::write(dir.join("std/json.hz"), "struct Value { x: i32 }\nfn parse(s: str) -> i32 {\n return 1\n}\nimpl Value {\n fn stringify(self: Value) -> str {\n return \"v\"\n }\n}\ntrait Loader {\n fn load(self) -> i32\n}\n").unwrap();
    // When: 取 json:: 补全(注入临时根)
    let items = bind_items_with_roots("import std.json\njson::", "json", "", &[dir]);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    // Then: fn/类型外另含固有与 trait 方法
    assert!(labels.contains(&"Value"), "{labels:?}");
    assert!(labels.contains(&"parse"), "{labels:?}");
    assert!(labels.contains(&"stringify"), "{labels:?}");
    assert!(labels.contains(&"load"), "{labels:?}");
}

#[test]
fn bind_without_import_returns_empty() {
    // Given: 无 import 的文件
    // When: 取未知绑定补全
    let items = bind_items_with_roots("let x = 1\nfoo::", "foo", "", &[]);
    // Then: 空表,不抛错
    assert!(items.is_empty());
}

#[test]
fn std_prefix_lists_exported_modules() {
    // Given: 仿 std/lib.hz 的 export 表
    let dir = unique_dir("stdlib");
    let lib = dir.join("lib.hz");
    std::fs::write(
        &lib,
        "export json\nexport json::*\nexport csv\nexport csv::*\n",
    )
    .unwrap();
    // When: 取空前缀与 `j` 前缀
    let all = std_prefix_items_with_lib(&lib, "");
    let filtered = std_prefix_items_with_lib(&lib, "j");
    // Then: 去重含 json/csv;j 前缀仅 json
    let labels: Vec<&str> =
        all.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"json"), "{labels:?}");
    assert!(labels.contains(&"csv"), "{labels:?}");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].label, "json");
}

#[test]
fn impl_and_trait_tables_complete_members() {
    // Given: 含 Point/impl 与 Printable trait 的程序
    let text = "struct Point { x: i32, y: i32 }\ntrait Printable {\n fn show(self) -> str\n}\nimpl Printable for Point {\n fn show(self: Point) -> str {\n return \"p\"\n }\n}\n";
    let (program, _symbols) = parse_and_collect(text);
    // When: 取 Point 的 impl 方法与 Printable 的 trait 方法
    let impl_items = impl_method_items(&program, "Point", "");
    let trait_items = trait_method_items(&program, "Printable", "");
    // Then: 均含 show;无关类型为空
    assert!(impl_items.iter().any(|i| i.label == "show"));
    assert!(trait_items.iter().any(|i| i.label == "show"));
    assert!(impl_method_items(&program, "Other", "").is_empty());
    assert!(trait_method_items(&program, "Missing", "").is_empty());
}
