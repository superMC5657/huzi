//! Vec 动态数组:`let v = vec(1, 2, 3)` + `push(v, x)` + `v[i]`/`v[i] = x` + `len(v)` + `for x in v`。
//!
//! 表示:变量槽内存放匿名结构体值 `{ data: ptr, len: i32, cap: i32 }`,data 指向堆
//! (首个 push 即按需 `realloc` 翻倍,程序结束前不释放,与既有堆字符串一致)。
//! 槽判定:定长数组槽 ty 为指针、元组槽 elem 为空,只有 vec 槽同时满足
//! "ty 为结构体 + elem 标记元素类型"(array_len 恒为 None)。
//! v1 约束:`vec()` 至少 1 个元素(类型由首元素推导);vec 变量整体不可
//! print/to_string(请打印元素);for-in 进入前一次性读取长度。

use super::{CodeGen, VarSlot};
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

/// vec 运行期三件套:堆数据指针 + 当前长度 + 容量。
struct VecParts<'ctx> {
    data: PointerValue<'ctx>,
    len: IntValue<'ctx>,
    cap: IntValue<'ctx>,
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
    fn vec_slot_of(&self, name: &str) -> Result<VarSlot<'ctx>> {
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
    fn vec_struct_type(&self) -> inkwell::types::StructType<'ctx> {
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
    fn load_vec_parts(&mut self, slot: &VarSlot<'ctx>) -> Result<VecParts<'ctx>> {
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
    fn store_vec_parts(
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
    fn elem_bytes_i32(
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
                "vec() requires at least 1 element (element type is inferred from the first)",
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
        let data = self
            .builder
            .build_array_malloc(elem_type, len_val, "vec_data")
            .map_err(|_| HuziError::new_global("Failed to allocate vec storage"))?;
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
        self.builder.build_store(alloca, vec_val).unwrap();
        self.scope_insert(
            stmt.name.clone(),
            VarSlot {
                ptr: alloca,
                ty: vec_ty,
                elem: Some(elem_type),
                array_len: None,
                mutable: stmt.mutable,
            },
        );
        self.declare_local(&stmt.name, alloca, vec_ty, span);
        Ok(())
    }

    /// `push(v, x)` — 需 `let mut` 变量;满时容量翻倍(realloc),返回整数 0
    /// (与 srand/sleep_ms 一致,仅为兼容表达式位置)。
    pub(super) fn compile_vec_push(
        &mut self,
        arguments: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 2 {
            return Err(HuziError::new_global("push() requires exactly 2 arguments (vec, value)"));
        }
        let name = match &arguments[0] {
            Expr::Ident(name) => name.clone(),
            _ => {
                return Err(HuziError::new_global(
                    "push() first argument must be a vec variable",
                ))
            }
        };
        let slot = self.vec_slot_of(&name)?;
        self.ensure_mutable(&arguments[0])?;
        let elem_type = slot.elem.unwrap();
        let value = self.compile_expr(&arguments[1])?;
        let value = self.coerce_value(elem_type, value)?;

        let vec_ty = self.vec_struct_type();
        let parts = self.load_vec_parts(&slot)?;
        let i32_type = self.context.i32_type();

        // len == cap 时翻倍扩容;grow 块内直接回写新 (data, cap),
        // append 块重载合并(内存即合并点,无需 phi)。
        let function = self.current_function()?;
        let grow_block = self.context.append_basic_block(function, "vec_grow");
        let append_block = self.context.append_basic_block(function, "vec_append");
        let full = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, parts.len, parts.cap, "vec_full")
            .unwrap();
        self.builder
            .build_conditional_branch(full, grow_block, append_block)
            .unwrap();

        self.builder.position_at_end(grow_block);
        let new_cap = self
            .builder
            .build_int_mul(parts.cap, i32_type.const_int(2, false), "vec_new_cap")
            .unwrap();
        let elem_bytes = self.elem_bytes_i32(elem_type)?;
        let new_bytes = self
            .builder
            .build_int_mul(new_cap, elem_bytes, "vec_new_bytes")
            .unwrap();
        let realloc_fn = self.module.get_function("realloc").expect("realloc in prelude");
        let new_data = self
            .builder
            .build_call(realloc_fn, &[parts.data.into(), new_bytes.into()], "vec_realloc")
            .unwrap()
            .try_as_basic_value()
            .unwrap_left()
            .into_pointer_value();
        self.store_vec_parts(&slot, vec_ty, &VecParts {
            data: new_data,
            len: parts.len,
            cap: new_cap,
        });
        self.builder
            .build_unconditional_branch(append_block)
            .unwrap();

        // 写入新元素并回写 (data, len+1, cap)。
        self.builder.position_at_end(append_block);
        let now = self.load_vec_parts(&slot)?;
        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, now.data, &[now.len], "vec_push_ptr")
                .unwrap()
        };
        self.builder.build_store(elem_ptr, value).unwrap();
        let new_len = self
            .builder
            .build_int_add(now.len, i32_type.const_int(1, false), "vec_new_len")
            .unwrap();
        self.store_vec_parts(&slot, vec_ty, &VecParts {
            data: now.data,
            len: new_len,
            cap: now.cap,
        });
        Ok(i32_type.const_int(0, false).into())
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

    /// `for x in v`:循环变量逐轮绑定当前元素值;进入前一次性读取长度。
    pub(super) fn compile_for_vec(
        &mut self,
        stmt: &ForStmt,
        name: &str,
        span: Span,
    ) -> Result<()> {
        let slot = self.vec_slot_of(name)?;
        let elem_type = slot.elem.unwrap();
        let parts = self.load_vec_parts(&slot)?;
        let (data, len) = (parts.data, parts.len);

        let function = self.current_function()?;
        let i_type = self.context.i32_type();
        let loop_block = self.context.append_basic_block(function, "vec_for_loop");
        let body_block = self.context.append_basic_block(function, "vec_for_body");
        let after_block = self.context.append_basic_block(function, "vec_for_after");
        self.loop_stack.push((loop_block, after_block));

        let idx_alloca = self.build_alloca(i_type.into(), "vec_for_idx")?;
        self.builder
            .build_store(idx_alloca, i_type.const_int(0, false))
            .unwrap();
        let var_alloca = self.build_alloca(elem_type, &stmt.var_name)?;
        self.declare_local(&stmt.var_name, var_alloca, elem_type, span);

        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();

        self.builder.position_at_end(loop_block);
        let idx = self
            .builder
            .build_load(i_type, idx_alloca, "vec_for_i")
            .unwrap()
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, idx, len, "vec_for_cond")
            .unwrap();
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .unwrap();

        self.builder.position_at_end(body_block);
        let idx = self
            .builder
            .build_load(i_type, idx_alloca, "vec_for_i")
            .unwrap()
            .into_int_value();
        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, data, &[idx], "vec_for_elem_ptr")
                .unwrap()
        };
        let elem = self
            .builder
            .build_load(elem_type, elem_ptr, "vec_for_elem")
            .unwrap();
        self.builder.build_store(var_alloca, elem).unwrap();
        self.push_scope();
        self.scope_insert(
            stmt.var_name.clone(),
            VarSlot {
                ptr: var_alloca,
                ty: elem_type,
                elem: None,
                array_len: None,
                mutable: true,
            },
        );
        self.compile_block(&stmt.body)?;
        self.pop_scope();
        let idx = self
            .builder
            .build_load(i_type, idx_alloca, "vec_for_i")
            .unwrap()
            .into_int_value();
        let next = self
            .builder
            .build_int_add(idx, i_type.const_int(1, false), "vec_for_next")
            .unwrap();
        self.builder.build_store(idx_alloca, next).unwrap();
        self.builder
            .build_unconditional_branch(loop_block)
            .unwrap();

        self.loop_stack.pop();
        self.builder.position_at_end(after_block);
        Ok(())
    }
}
