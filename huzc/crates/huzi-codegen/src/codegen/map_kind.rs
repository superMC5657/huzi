//! Map 键值特化(`str->i32`/`str->str`/`i32->i32`)的种类判定与底层 helpers。
//!
//! 复用泛型单态化/修饰名思路:每种特化对应一套条目布局,LLVM 句柄统一为
//! `{ ptr, len, cap }`,条目按种类区分键/值字段类型。AST 层面 `Map` 裸写为
//! 默认 `str->i32`,`Map<K,V>` 显式指定另外两种。

use super::{CodeGen, MapKind};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl MapKind {
    /// 是否为 `str` 类型(含 parser 的 `Named("str")` 与 `Type::Str`)。
    fn is_str_ty(ty: &Type) -> bool {
        matches!(ty, Type::Str) || matches!(ty, Type::Named(n) if n == "str")
    }

    /// 是否为 `i32` 类型(含 parser 的 `Named("i32")` 与 `Type::I32`)。
    fn is_i32_ty(ty: &Type) -> bool {
        matches!(ty, Type::I32 | Type::U32)
            || matches!(ty, Type::Named(n) if n == "i32" || n == "u32")
    }

    /// 是否为 map 族名(`Map`/`HashMap`/`map`)。
    pub(super) fn is_map_name(name: &str) -> bool {
        name == "Map" || name == "HashMap" || name == "map"
    }

    /// 由 AST 类型判定特化种类;不支持的组合返回 None。
    pub(super) fn from_ast(ty: &Type) -> Option<MapKind> {
        match ty {
            Type::Named(n) if Self::is_map_name(n) => Some(MapKind::StrI32),
            Type::Applied(n, args) if Self::is_map_name(n) => {
                if args.len() != 2 {
                    return None;
                }
                let k_str = Self::is_str_ty(&args[0]);
                let k_i32 = Self::is_i32_ty(&args[0]);
                let v_str = Self::is_str_ty(&args[1]);
                let v_i32 = Self::is_i32_ty(&args[1]);
                if k_str && v_i32 {
                    Some(MapKind::StrI32)
                } else if k_str && v_str {
                    Some(MapKind::StrStr)
                } else if k_i32 && v_i32 {
                    Some(MapKind::I32I32)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// 键是否为 `str`(否则为 `i32`)。
    pub(super) fn key_is_str(self) -> bool {
        matches!(self, MapKind::StrI32 | MapKind::StrStr)
    }

    /// 值是否为 `str`(否则为 `i32`)。
    pub(super) fn val_is_str(self) -> bool {
        matches!(self, MapKind::StrStr)
    }

    /// 人类可读的 `K->V` 描述。
    pub(super) fn display(self) -> &'static str {
        match self {
            MapKind::StrI32 => "str->i32",
            MapKind::StrStr => "str->str",
            MapKind::I32I32 => "i32->i32",
        }
    }
}

#[allow(dead_code)]
impl<'ctx> CodeGen<'ctx> {
    /// 条目类型 `{ hash:i32, state:i32, key, klen:i32, val }`,键/值按种类切换。
    pub(super) fn map_entry_type_of(&self, kind: MapKind) -> inkwell::types::StructType<'ctx> {
        let i32_t: inkwell::types::BasicTypeEnum<'ctx> = self.context.i32_type().into();
        let ptr_t: inkwell::types::BasicTypeEnum<'ctx> = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .into();
        let key_t = if kind.key_is_str() { ptr_t } else { i32_t };
        let val_t = if kind.val_is_str() { ptr_t } else { i32_t };
        self.context
            .struct_type(&[i32_t, i32_t, key_t, i32_t, val_t], false)
    }

    /// 键的 LLVM 类型。
    pub(super) fn map_key_llvm(&self, kind: MapKind) -> inkwell::types::BasicTypeEnum<'ctx> {
        if kind.key_is_str() {
            self.context.ptr_type(inkwell::AddressSpace::default()).into()
        } else {
            self.context.i32_type().into()
        }
    }

    /// 值的 LLVM 类型。
    pub(super) fn map_val_llvm(&self, kind: MapKind) -> inkwell::types::BasicTypeEnum<'ctx> {
        if kind.val_is_str() {
            self.context.ptr_type(inkwell::AddressSpace::default()).into()
        } else {
            self.context.i32_type().into()
        }
    }

    /// 由表达式判定 map 种类:变量取槽标记,字段取 AST 声明,默认 `str->i32`。
    pub(super) fn map_kind_of_expr(&self, expr: &Expr) -> Result<MapKind> {
        if let Expr::Ident(name) = expr {
            let slot = self.map_slot_of(name)?;
            if let Some(k) = slot.map_kind {
                return Ok(k);
            }
            return Ok(MapKind::StrI32);
        }
        if let Expr::FieldAccess(fa) = expr {
            if let Some(ty) = self.field_ast_type(&fa.base, &fa.field) {
                if let Some(k) = MapKind::from_ast(&ty) {
                    return Ok(k);
                }
                return Err(HuziError::new_global(format!(
                    "Unsupported Map type '{}'; expected Map, Map<str, str> or Map<i32, i32>",
                    ty
                )));
            }
        }
        Err(HuziError::new_global(
            "Map operation requires a HashMap variable or field (str->i32, str->str or i32->i32)",
        ))
    }

    /// 编译键实参并按种类校验(`str` 须指针,`i32` 须 32 位整数)。
    pub(super) fn compile_map_key(
        &mut self,
        expr: &Expr,
        kind: MapKind,
        fname: &str,
    ) -> Result<BasicValueEnum<'ctx>> {
        let v = self.compile_expr(expr)?;
        if kind.key_is_str() {
            match v {
                BasicValueEnum::PointerValue(_) => Ok(v),
                _ => Err(HuziError::new_global(format!(
                    "{}() key must be str (map is {})",
                    fname,
                    kind.display()
                ))),
            }
        } else {
            match v {
                BasicValueEnum::IntValue(iv) if iv.get_type().get_bit_width() == 32 => Ok(v),
                _ => Err(HuziError::new_global(format!(
                    "{}() key must be i32 (map is {})",
                    fname,
                    kind.display()
                ))),
            }
        }
    }

    /// 编译值实参并按种类校验。
    pub(super) fn compile_map_val(
        &mut self,
        expr: &Expr,
        kind: MapKind,
        fname: &str,
    ) -> Result<BasicValueEnum<'ctx>> {
        let v = self.compile_expr(expr)?;
        if kind.val_is_str() {
            match v {
                BasicValueEnum::PointerValue(_) => Ok(v),
                _ => Err(HuziError::new_global(format!(
                    "{}() value must be str (map is {})",
                    fname,
                    kind.display()
                ))),
            }
        } else {
            match v {
                BasicValueEnum::IntValue(iv) if iv.get_type().get_bit_width() == 32 => Ok(v),
                _ => Err(HuziError::new_global(format!(
                    "{}() value must be i32 (map is {})",
                    fname,
                    kind.display()
                ))),
            }
        }
    }

    /// 按种类求哈希:`str` 走 FNV-1a,`i32` 直接用键值。
    pub(super) fn map_hash_typed(
        &mut self,
        key: BasicValueEnum<'ctx>,
        kind: MapKind,
    ) -> Result<IntValue<'ctx>> {
        if kind.key_is_str() {
            let ptr = match key {
                BasicValueEnum::PointerValue(p) => p,
                _ => return Err(HuziError::new_global("Map str key must be a pointer")),
            };
            let klen = self.str_len_of(ptr, "map_klen")?;
            return self.map_hash(ptr, klen);
        }
        match key {
            BasicValueEnum::IntValue(iv) => Ok(iv),
            _ => Err(HuziError::new_global("Map i32 key must be an integer")),
        }
    }

    /// 按种类判键相等:`str` 用 `strcmp`,`i32` 用整数比较。
    pub(super) fn map_keys_equal(
        &mut self,
        stored: BasicValueEnum<'ctx>,
        query: BasicValueEnum<'ctx>,
        kind: MapKind,
    ) -> IntValue<'ctx> {
        if kind.key_is_str() {
            let a = match stored {
                BasicValueEnum::PointerValue(p) => p,
                _ => return self.context.bool_type().const_int(0, false),
            };
            let b = match query {
                BasicValueEnum::PointerValue(p) => p,
                _ => return self.context.bool_type().const_int(0, false),
            };
            return self.map_key_matches_inner(a, b);
        }
        let a = stored.into_int_value();
        let b = query.into_int_value();
        self.builder
            .build_int_compare(inkwell::IntPredicate::EQ, a, b, "map_keq")
            .unwrap()
    }

    /// 键相等(`str` 路径,供 `map_keys_equal` 与旧探测复用)。
    pub(super) fn map_key_matches_inner(
        &mut self,
        stored: PointerValue<'ctx>,
        key: PointerValue<'ctx>,
    ) -> IntValue<'ctx> {
        let sc = self.module.get_function("strcmp").expect("strcmp in prelude");
        let c = self
            .builder
            .build_call(sc, &[stored.into(), key.into()], "map_cmp")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_int_value();
        self.builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                c,
                self.context.i32_type().const_int(0, false),
                "map_eq",
            )
            .unwrap()
    }

    /// 读条目键字段(按种类返回指针或整数值)。
    pub(super) fn map_load_key(
        &self,
        entry_ty: inkwell::types::StructType<'ctx>,
        data: PointerValue<'ctx>,
        ety: inkwell::types::BasicTypeEnum<'ctx>,
        idx: IntValue<'ctx>,
        kind: MapKind,
    ) -> BasicValueEnum<'ctx> {
        let ep = unsafe { self.builder.build_gep(ety, data, &[idx], "map_kep").unwrap() };
        let kp = self.builder.build_struct_gep(entry_ty, ep, 2, "map_kp").unwrap();
        let key_ty = self.map_key_llvm(kind);
        self.builder.build_load(key_ty, kp, "map_kv").unwrap()
    }

    /// 读条目值字段。
    pub(super) fn map_load_val(
        &self,
        entry_ty: inkwell::types::StructType<'ctx>,
        data: PointerValue<'ctx>,
        ety: inkwell::types::BasicTypeEnum<'ctx>,
        idx: IntValue<'ctx>,
        kind: MapKind,
    ) -> BasicValueEnum<'ctx> {
        let ep = unsafe { self.builder.build_gep(ety, data, &[idx], "map_vep").unwrap() };
        let vp = self.builder.build_struct_gep(entry_ty, ep, 4, "map_vp").unwrap();
        let val_ty = self.map_val_llvm(kind);
        self.builder.build_load(val_ty, vp, "map_vv").unwrap()
    }

    /// 校验 map 字段赋值特化一致;`map_new()` 新建放行(种类由期望决定)。
    pub(super) fn check_map_field_assignable(
        &self,
        value_expr: &Expr,
        expected: &Type,
    ) -> Result<()> {
        let Some(exp_kind) = MapKind::from_ast(expected) else {
            return Ok(());
        };
        // 新建 map 句柄无种类包袱,按期望特化接纳。
        if let Expr::Call(c) = value_expr {
            if let Expr::Ident(n) = &*c.callee {
                if n == "map_new" {
                    return Ok(());
                }
            }
        }
        let actual_kind = match value_expr {
            Expr::Ident(name) => self.scope_lookup(name).and_then(|s| s.map_kind),
            Expr::FieldAccess(fa) => self
                .field_ast_type(&fa.base, &fa.field)
                .and_then(|t| MapKind::from_ast(&t)),
            Expr::Call(c) => match &*c.callee {
                Expr::Ident(fname) => {
                    let key = self.qualify_name(fname);
                    self.fn_return_ast.get(&key).and_then(MapKind::from_ast)
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(act) = actual_kind {
            if act != exp_kind {
                return Err(HuziError::new_global(format!(
                    "Type mismatch: expected Map({}), found Map({})",
                    exp_kind.display(),
                    act.display()
                )));
            }
        }
        Ok(())
    }
}
