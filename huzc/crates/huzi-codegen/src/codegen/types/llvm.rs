use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::AddressSpace;

impl<'ctx> super::CodeGen<'ctx> {
    pub(in super::super) fn type_to_llvm(&self, ty: &Type) -> Result<inkwell::types::BasicTypeEnum<'ctx>> {
        match ty {
            Type::I32 | Type::U32 => Ok(self.context.i32_type().into()),
            Type::I64 | Type::U64 => Ok(self.context.i64_type().into()),
            Type::F32 => Ok(self.context.f32_type().into()),
            Type::F64 => Ok(self.context.f64_type().into()),
            Type::Bool => Ok(self.context.bool_type().into()),
            Type::Char => Ok(self.context.i8_type().into()),
            Type::Str => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            Type::Unit => Ok(self.context.i32_type().into()),
            Type::Fn(_, _) => Ok(self.closure_struct_type().into()),
            // 数组退化为指针（LLVM 不透明指针使两者等价）；元素类型在 VarSlot 中跟踪。
            Type::Array(_, _) => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            // `Box<T>` 降阶为指向 T 的 LLVM 存储的裸指针；因此持有 Box 字段的结构体
            // 具有固定大小，且通过 Box 形成的任意引用循环均合法（参见 check_type_cycles）。
            // 嵌套的 `Box<Box<..>>` 同样是裸指针：每层堆单元持有下一层的指针，
            // 因而内存布局保持扁平。
            // T 为具名结构体或标量（`i32`/`i64`/`f64`/`bool`/`str`，及 `u32`/`u64`/`f32`/`char`）；
            // 拒绝其他复合类型。
            Type::Box(inner) => {
                // 嵌套层直接放行(指针套指针);单层须为结构体或标量。
                if matches!(&**inner, Type::Box(_)) {
                    self.type_to_llvm(inner)?;
                    return Ok(self.context.ptr_type(AddressSpace::default()).into());
                }
                let inner_ty = self.type_to_llvm(inner)?;
                if self.is_box_pointee(inner_ty) || Self::is_box_scalar(inner, inner_ty) {
                    return Ok(self.context.ptr_type(AddressSpace::default()).into());
                }
                Err(HuziError::new_global(format!(
                    "Box<T> requires a named struct or scalar type (i32/i64/f64/bool/str) (found '{}')",
                    inner
                )))
            }
            Type::Weak(inner) => {
                self.type_to_llvm(inner)?;
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            // 元组是字面量结构体：LLVM 按结构比对它们，因此两个 `(i32, str)` 元组类型始终相等。
            Type::Tuple(elems) => {
                let mut field_types = Vec::with_capacity(elems.len());
                for elem in elems {
                    field_types.push(self.type_to_llvm(elem)?);
                }
                Ok(self.context.struct_type(&field_types, false).into())
            }
            Type::Named(name) => match name.as_str() {
                "i32" | "u32" => Ok(self.context.i32_type().into()),
                "i64" | "u64" => Ok(self.context.i64_type().into()),
                "f32" => Ok(self.context.f32_type().into()),
                "f64" => Ok(self.context.f64_type().into()),
                "bool" => Ok(self.context.bool_type().into()),
                "char" => Ok(self.context.i8_type().into()),
                "str" => Ok(self.context.ptr_type(AddressSpace::default()).into()),
                "map" | "Map" | "HashMap" => Ok(self.vec_struct_type().into()),
                other => {
                    if let Some((st, _)) = self.structs.get(other) {
                        return Ok((*st).into());
                    }
                    if let Some(info) = self.enums.get(other) {
                        // 简单枚举为其 i32 tag；带数据的枚举为带标签的结构体。
                        return Ok(match info.llvm {
                            Some(st) => st.into(),
                            None => self.context.i32_type().into(),
                        });
                    }
                    Err(HuziError::new_global(format!("Unsupported type: {}", other)))
                }
            },
            Type::Generic(g) => Err(HuziError::new_global(format!(
                "Unresolved generic type parameter '{}'",
                g
            ))),
            Type::Applied(name, args) => {
                if name == "vec" || name == "map" || name == "Map" || name == "HashMap" {
                    return Ok(self.vec_struct_type().into());
                }
                let mangled = crate::codegen::generic::mangle_name(name, args);
                if let Some((st, _)) = self.structs.get(&mangled) {
                    return Ok((*st).into());
                }
                Err(HuziError::new_global(format!("Unsupported type: {}", name)))
            }
        }
    }
}
