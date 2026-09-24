//! Vec 动态数组:`let v = vec(1, 2, 3)` / `let v = vec<T>()` + `push(v, x)` +
//! 支持索引读写 `v[i]`/`v[i] = x`、获取长度 `len(v)`、迭代遍历 `for x in v` 以及打印输出 `print(v)`。
//! 增删操作 `pop`/`remove`/`insert`/`clear` 见 `vec_ops.rs`(同模块分文件,
//! 保持本文件 500 行以内)。
//!
//! 表示:变量槽内存放匿名结构体值 `{ data: ptr, len: i32, cap: i32 }`,data 指向堆
//! (首个 push 即按需 `realloc` 翻倍,程序结束前不释放,与既有堆字符串一致)。
//! 槽判定:定长数组槽 ty 为指针、元组槽 elem 为空,只有 vec 槽同时满足
//! "ty 为结构体 + elem 标记元素类型"(array_len 恒为 None)。
//! 空 vec 由 `vec<T>()` 构造(data 为 null,len/cap 为 0,首次 push 分配初始容量);
//! 裸 `vec()` 无类型可推导,直接报编译错误引导写 `vec<T>()`。
//! 打印格式为 `[e1, e2, ...]`(空 vec 为 `[]`),元素复用自身打印逻辑。

mod push;

use super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

/// vec 运行期三件套:堆数据指针 + 当前长度 + 容量。
pub(super) struct VecParts<'ctx> {
    pub(super) data: PointerValue<'ctx>,
    pub(super) len: IntValue<'ctx>,
    pub(super) cap: IntValue<'ctx>,
}

impl<'ctx> CodeGen<'ctx> {
    /// AST 层面判断是否为 `vec(...)` 构造调用(供 `let` 分发,不生成指令)。
    pub(super) fn is_vec_ctor(call: &CallExpr) -> bool {
        matches!(&*call.callee, Expr::Ident(name) if name == "vec")
    }

    /// 变量槽是否为 vec(结构体类型 + elem 标记元素类型)。
    pub(super) fn is_vec_slot(slot: &VarSlot<'_>) -> bool {
        slot.ty.is_struct_type() && slot.elem.is_some()
    }

    /// 按名查找 vec 变量槽;非 vec 或未定义时报友好错误。
    pub(super) fn vec_slot_of(&self, name: &str) -> Result<VarSlot<'ctx>> {
        let slot = self
            .scope_lookup(name)
            .ok_or_else(|| self.unknown_variable_error(name))?;
        if !Self::is_vec_slot(&slot) {
            return Err(HuziError::new_global(format!(
                "'{}' is not a vec (use vec(...) to create one)",
                name
            )));
        }
        Ok(slot)
    }

    /// vec 匿名结构体类型 `{ data: ptr, len: i32, cap: i32 }`(data 为非类型化指针)。
    pub(super) fn vec_struct_type(&self) -> inkwell::types::StructType<'ctx> {
        self.context.struct_type(
            &[
                self.context.ptr_type(inkwell::AddressSpace::default()).into(),
                self.context.i32_type().into(),
                self.context.i32_type().into(),
            ],
            false,
        )
    }

    /// 装载 vec 变量的结构体值,并拆出 (data, len, cap)。
    pub(super) fn load_vec_parts(&mut self, slot: &VarSlot<'ctx>) -> Result<VecParts<'ctx>> {
        let vec_val = self
            .builder
            .build_load(slot.ty, slot.ptr, "vec_load")
            .unwrap()
            .into_struct_value();
        let data = self
            .builder
            .build_extract_value(vec_val, 0, "vec_data")
            .unwrap()
            .into_pointer_value();
        let len = self
            .builder
            .build_extract_value(vec_val, 1, "vec_len")
            .unwrap()
            .into_int_value();
        let cap = self
            .builder
            .build_extract_value(vec_val, 2, "vec_cap")
            .unwrap()
            .into_int_value();
        Ok(VecParts { data, len, cap })
    }

    /// 回写 vec 结构体的 (data, len, cap) 三个字段。
    pub(super) fn store_vec_parts(
        &mut self,
        slot: &VarSlot<'ctx>,
        vec_ty: inkwell::types::StructType<'ctx>,
        parts: &VecParts<'ctx>,
    ) {
        let data_ptr = self
            .builder
            .build_struct_gep(vec_ty, slot.ptr, 0, "vec_field_data")
            .unwrap();
        self.builder.build_store(data_ptr, parts.data).unwrap();
        let len_ptr = self
            .builder
            .build_struct_gep(vec_ty, slot.ptr, 1, "vec_field_len")
            .unwrap();
        self.builder.build_store(len_ptr, parts.len).unwrap();
        let cap_ptr = self
            .builder
            .build_struct_gep(vec_ty, slot.ptr, 2, "vec_field_cap")
            .unwrap();
        self.builder.build_store(cap_ptr, parts.cap).unwrap();
    }

    /// 元素类型的字节数(归一化为 i32,供 malloc/realloc 的总字节计算)。
    pub(super) fn elem_bytes_i32(
        &self,
        elem: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let size_opt = match elem {
            inkwell::types::BasicTypeEnum::ArrayType(t) => t.size_of(),
            inkwell::types::BasicTypeEnum::FloatType(t) => Some(t.size_of()),
            inkwell::types::BasicTypeEnum::IntType(t) => Some(t.size_of()),
            inkwell::types::BasicTypeEnum::PointerType(t) => Some(t.size_of()),
            inkwell::types::BasicTypeEnum::StructType(t) => t.size_of(),
            inkwell::types::BasicTypeEnum::VectorType(t) => t.size_of(),
        };
        let size = size_opt
            .ok_or_else(|| HuziError::new_global("Cannot determine vec element size"))?;
        let i32_type = self.context.i32_type();
        Ok(match size.get_type().get_bit_width() {
            32 => size,
            64 => self.builder.build_int_truncate(size, i32_type, "bytes").unwrap(),
            _ => self.builder.build_int_z_extend(size, i32_type, "bytes").unwrap(),
        })
    }

    /// 编译构造参数并协调到首元素类型,返回(协调后值列表, 元素类型)。
    fn vec_elements(
        &mut self,
        arguments: &[Expr],
    ) -> Result<(Vec<BasicValueEnum<'ctx>>, inkwell::types::BasicTypeEnum<'ctx>)> {
        if arguments.is_empty() {
            return Err(HuziError::new_global(
                "vec() requires at least 1 element; for an empty vec, write vec<T>() (e.g. vec<i32>())",
            ));
        }
        let mut values = Vec::with_capacity(arguments.len());
        for arg in arguments {
            values.push(self.compile_expr(arg)?);
        }
        let elem_type = values[0].get_type();
        let mut coerced = Vec::with_capacity(values.len());
        for val in values {
            coerced.push(self.coerce_value(elem_type, val)?);
        }
        Ok((coerced, elem_type))
    }

    /// 堆上分配并存入元素,组装 `{ data, len, cap=len }` 结构体值。
    fn vec_assemble(
        &mut self,
        coerced: Vec<BasicValueEnum<'ctx>>,
        elem_type: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        let i32_type = self.context.i32_type();
        let len_val = i32_type.const_int(coerced.len() as u64, false);
        let data = self.alloc_vec_buffer(elem_type, len_val, "vec_data")?;
        for (i, val) in coerced.into_iter().enumerate() {
            let idx = i32_type.const_int(i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(elem_type, data, &[idx], "vec_elem")
                    .unwrap()
            };
            self.builder.build_store(elem_ptr, val).unwrap();
        }

        let vec_ty = self.vec_struct_type();
        let tmp = self.build_alloca(vec_ty.into(), "vec_tmp")?;
        let data_ptr = self
            .builder
            .build_struct_gep(vec_ty, tmp, 0, "vec_tmp_data")
            .unwrap();
        self.builder.build_store(data_ptr, data).unwrap();
        let len_ptr = self
            .builder
            .build_struct_gep(vec_ty, tmp, 1, "vec_tmp_len")
            .unwrap();
        self.builder.build_store(len_ptr, len_val).unwrap();
        let cap_ptr = self
            .builder
            .build_struct_gep(vec_ty, tmp, 2, "vec_tmp_cap")
            .unwrap();
        self.builder.build_store(cap_ptr, len_val).unwrap();
        Ok(self.builder.build_load(vec_ty, tmp, "vec_val").unwrap())
    }

    /// 零长组装 `{ null, 0, 0 }`(data 从不解引用,len 为 0 时循环零次)。
    pub(super) fn vec_assemble_empty(
        &mut self,
        _elem_type: inkwell::types::BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        let i32_type = self.context.i32_type();
        let zero = i32_type.const_int(0, false);
        let vec_ty = self.vec_struct_type();
        let tmp = self.build_alloca(vec_ty.into(), "vec_tmp")?;
        let data_ptr = self
            .builder
            .build_struct_gep(vec_ty, tmp, 0, "vec_tmp_data")
            .unwrap();
        let null_ptr = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null();
        self.builder.build_store(data_ptr, null_ptr).unwrap();
        let len_ptr = self
            .builder
            .build_struct_gep(vec_ty, tmp, 1, "vec_tmp_len")
            .unwrap();
        self.builder.build_store(len_ptr, zero).unwrap();
        let cap_ptr = self
            .builder
            .build_struct_gep(vec_ty, tmp, 2, "vec_tmp_cap")
            .unwrap();
        self.builder.build_store(cap_ptr, zero).unwrap();
        Ok(self.builder.build_load(vec_ty, tmp, "vec_val").unwrap())
    }

    /// `vec<T>()` 表达式位置的值(元素类型由尖括号指定,调用方负责存槽)。
    pub(super) fn compile_vec_empty_value(
        &mut self,
        elem_ast_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>> {
        let elem_type = self.type_to_llvm(elem_ast_ty)?;
        self.vec_assemble_empty(elem_type)
    }

    /// `let v = vec<T>()` — 零长存槽,elem 标记尖括号指定的元素类型。
    /// 同非空构造,不接受 `let` 类型标注(类型已由 `vec<T>` 给出)。
    pub(super) fn compile_let_vec_empty(
        &mut self,
        stmt: &LetStmt,
        elem_ast_ty: &Type,
        span: Span,
    ) -> Result<()> {
        if stmt.type_annotation.is_some() {
            return Err(HuziError::new_global(
                "vec<T>() already specifies its element type; remove the `let` type annotation",
            ));
        }
        let elem_type = self.type_to_llvm(elem_ast_ty)?;
        let vec_val = self.vec_assemble_empty(elem_type)?;
        let vec_ty = vec_val.get_type();
        let alloca = self.build_alloca(vec_ty, &stmt.name)?;
        self.register_droppable(alloca, vec_ty, super::drop::DropKind::Vec);
        self.builder.build_store(alloca, vec_val).unwrap();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: vec_ty,
                elem: Some(elem_type),
                array_len: None,
                mutable: stmt.mutable,
                box_inner: None,
                map_kind: None,
            },
        );
        self.declare_local(&stmt.name, alloca, vec_ty, span);
        Ok(())
    }

    /// `vec(e1, e2, ...)` — 元素类型由首元素推导,其余元素自动协调;
    /// 返回栈上组装的结构体值,调用方(`let`)负责存入变量槽。
    pub(super) fn compile_vec_ctor(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        let (coerced, elem_type) = self.vec_elements(arguments)?;
        self.vec_assemble(coerced, elem_type)
    }

    /// `let v = vec(...)` — 结构体值存入变量槽,elem 标记元素类型。
    /// 不接受类型标注(元素类型由构造参数推导)。
    pub(super) fn compile_let_vec(
        &mut self,
        stmt: &LetStmt,
        arguments: &[Expr],
        span: Span,
    ) -> Result<()> {
        if stmt.type_annotation.is_some() {
            return Err(HuziError::new_global(
                "vec() infers its element type from the arguments; remove the type annotation",
            ));
        }
        let (coerced, elem_type) = self.vec_elements(arguments)?;
        let vec_val = self.vec_assemble(coerced, elem_type)?;
        let vec_ty = vec_val.get_type();
        let alloca = self.build_alloca(vec_ty, &stmt.name)?;
        self.register_droppable(alloca, vec_ty, super::drop::DropKind::Vec);
        self.builder.build_store(alloca, vec_val).unwrap();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: vec_ty,
                elem: Some(elem_type),
                array_len: None,
                mutable: stmt.mutable,
                box_inner: None,
                map_kind: None,
            },
        );
        self.declare_local(&stmt.name, alloca, vec_ty, span);
        Ok(())
    }

    /// vec 下标读 `v[i]`(调用方已确认数组表达式为 vec 变量)。
    pub(super) fn compile_vec_index_load(
        &mut self,
        name: &str,
        index_expr: &Expr,
    ) -> Result<BasicValueEnum<'ctx>> {
        let slot = self.vec_slot_of(name)?;
        let elem_type = slot.elem.unwrap();
        let parts = self.load_vec_parts(&slot)?;
        let (data, len) = (parts.data, parts.len);
        let index_val = self.compile_expr(index_expr)?;
        let index_i32 = self.coerce_index(index_val)?;
        self.emit_vec_bounds_check(len, index_i32)?;
        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, data, &[index_i32], "vec_elem_ptr")
                .unwrap()
        };
        Ok(self.builder.build_load(elem_type, elem_ptr, "vec_elem").unwrap())
    }

    /// vec 下标写 `v[i] = x` 的元素指针(调用方负责 coerce + store)。
    pub(super) fn vec_index_ptr(
        &mut self,
        name: &str,
        index_expr: &Expr,
    ) -> Result<(PointerValue<'ctx>, inkwell::types::BasicTypeEnum<'ctx>)> {
        let slot = self.vec_slot_of(name)?;
        let elem_type = slot.elem.unwrap();
        let parts = self.load_vec_parts(&slot)?;
        let (data, len) = (parts.data, parts.len);
        let index_val = self.compile_expr(index_expr)?;
        let index_i32 = self.coerce_index(index_val)?;
        self.emit_vec_bounds_check(len, index_i32)?;
        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, data, &[index_i32], "vec_elem_ptr")
                .unwrap()
        };
        Ok((elem_ptr, elem_type))
    }

    /// vec 动态越界检查:无符号比较,负下标自然落入失败分支。
    pub(super) fn emit_vec_bounds_check(
        &mut self,
        len: IntValue<'ctx>,
        index_i32: IntValue<'ctx>,
    ) -> Result<()> {
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, index_i32, len, "vec_idx_ok")
            .unwrap();
        self.emit_runtime_check(cond, "Runtime error: vec index out of bounds\n\0", &[])
    }

    /// `len(v)` 的动态长度。
    pub(super) fn vec_len_value(&mut self, name: &str) -> Result<BasicValueEnum<'ctx>> {
        let slot = self.vec_slot_of(name)?;
        let len = self.load_vec_parts(&slot)?.len;
        Ok(len.into())
    }

}
