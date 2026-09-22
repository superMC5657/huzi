use huzi_ast::Type;

/// 将类型名称修饰为合法标识符字符串。
pub fn mangle_type(ty: &Type) -> String {
    match ty {
        Type::I32 => "i32".to_string(),
        Type::I64 => "i64".to_string(),
        Type::U32 => "u32".to_string(),
        Type::U64 => "u64".to_string(),
        Type::F32 => "f32".to_string(),
        Type::F64 => "f64".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Str => "str".to_string(),
        Type::Char => "char".to_string(),
        Type::Unit => "unit".to_string(),
        Type::Named(s) | Type::Generic(s) => s.clone(),
        Type::Box(inner) => format!("Box__{}", mangle_type(inner)),
        Type::Applied(name, args) => {
            let mangled_args: Vec<_> = args.iter().map(mangle_type).collect();
            format!("{}__{}", name, mangled_args.join("_"))
        }
        Type::Array(elem, size) => format!("arr__{}_{}", mangle_type(elem), size),
        Type::Tuple(elems) => {
            let mangled_elems: Vec<_> = elems.iter().map(mangle_type).collect();
            format!("tuple__{}", mangled_elems.join("_"))
        }
    }
}

/// 将名称与类型实参列表修饰为单态化符号名。
pub fn mangle_name(base: &str, type_args: &[Type]) -> String {
    if type_args.is_empty() {
        base.to_string()
    } else {
        let mangled_args: Vec<_> = type_args.iter().map(mangle_type).collect();
        format!("{}__{}", base, mangled_args.join("_"))
    }
}
