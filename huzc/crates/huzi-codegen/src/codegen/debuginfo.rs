//! DWARF 调试信息生成(`-g` 时启用):编译单元、函数子程序、语句行号
//! 位置与变量 `llvm.dbg.declare`。`debug` 为 `None` 时所有方法 no-op,
//! 不产生任何元数据,IR 与未启用时完全一致。

use super::CodeGen;
use huzi_ast::Span;
use inkwell::debug_info::{
    AsDIScope, DebugInfoBuilder, DIFlags, DIFlagsConstants, DIFile, DISubprogram,
    DIType, DWARFEmissionKind, DWARFSourceLanguage,
};
use inkwell::types::BasicTypeEnum;
use inkwell::values::PointerValue;
use inkwell::AddressSpace;
use std::collections::HashMap;

/// llvm-sys 未提供 DW_ATE_* 具名常量,这里使用 DWARF 标准取值。
const DW_ATE_BOOLEAN: u32 = 0x02;
const DW_ATE_FLOAT: u32 = 0x04;
const DW_ATE_SIGNED: u32 = 0x05;
const DW_ATE_UTF: u32 = 0x08;

/// `-g` 启用后的调试状态:DIBuilder、编译单元与 DIFile 缓存。
pub(super) struct DebugState<'ctx> {
    builder: DebugInfoBuilder<'ctx>,
    compile_unit: inkwell::debug_info::DICompileUnit<'ctx>,
    main_file: DIFile<'ctx>,
    /// 当前编译文件的 DIFile(主程序或文件模块)。
    current_file: DIFile<'ctx>,
    /// DIFile 按 "目录\0文件名" 缓存,避免重复创建。
    files: HashMap<String, DIFile<'ctx>>,
}

/// 拆分源文件路径为 (文件名, 目录),供 DIFile 使用。
fn split_source_path(path: &str) -> (String, String) {
    let p = std::path::Path::new(path);
    let filename = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let directory = p
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    (filename, directory)
}

/// 计算一个 LLVM 类型的 (字节大小, 字节对齐),与 x86_64 通用布局一致:
/// 整型按位宽,浮点 4/8,指针 8,复合类型按成员排布并加填充。
fn llvm_size_align(ty: BasicTypeEnum) -> (u64, u32) {
    match ty {
        BasicTypeEnum::IntType(t) => {
            let size = (t.get_bit_width() as u64).div_ceil(8);
            (size, size.min(8) as u32)
        }
        BasicTypeEnum::FloatType(t) => {
            let size = t.size_of().get_zero_extended_constant().unwrap_or(8);
            (size, size as u32)
        }
        BasicTypeEnum::PointerType(_) => (8, 8),
        BasicTypeEnum::ArrayType(t) => {
            let (elem_size, elem_align) = llvm_size_align(t.get_element_type());
            (elem_size * t.len() as u64, elem_align)
        }
        BasicTypeEnum::VectorType(_) => (16, 16),
        BasicTypeEnum::StructType(t) => {
            let mut offset = 0u64;
            let mut align = 1u32;
            for field in t.get_field_types() {
                let (size, field_align) = llvm_size_align(field);
                align = align.max(field_align);
                offset = offset.div_ceil(field_align as u64) * field_align as u64 + size;
            }
            (offset, align)
        }
    }
}

/// 结构体/元组成员的字节偏移(按 LLVM 默认布局)。
fn struct_member_offsets(fields: &[BasicTypeEnum]) -> Vec<u64> {
    let mut offsets = Vec::with_capacity(fields.len());
    let mut offset = 0u64;
    for field in fields {
        let (size, align) = llvm_size_align(*field);
        offset = offset.div_ceil(align as u64) * align as u64;
        offsets.push(offset);
        offset += size;
    }
    offsets
}

impl<'ctx> CodeGen<'ctx> {
    /// 启用 DWARF 调试信息。必须在 [`CodeGen::compile`] 之前调用,
    /// `source_path` 是主程序源文件(编译单元)。
    pub fn enable_debug_info(&mut self, source_path: &str) {
        let (filename, directory) = split_source_path(source_path);
        let (builder, compile_unit) = self.module.create_debug_info_builder(
            true,
            DWARFSourceLanguage::C,
            &filename,
            &directory,
            "huzc",
            false, // is_optimized:调试模式强制 -O0
            "",    // 编译命令行 flags
            0,     // 运行时版本 (runtime_version)
            "",    // split 名称 (split_name)
            DWARFEmissionKind::Full,
            0,     // dwo id
            false, // 拆分调试内联 (split_debug_inlining)
            false, // 性能剖析调试信息 (debug_info_for_profiling)
            "",    // sysroot
            "",    // sdk
        );
        let main_file = builder.create_file(&filename, &directory);
        self.debug = Some(DebugState {
            builder,
            compile_unit,
            main_file,
            current_file: main_file,
            files: HashMap::new(),
        });
        // llc 只在模块带 "Debug Info Version" 标志时才输出调试节
        // (DIBuilder 不会自动添加),这里补上当前 LLVM 的版本号。
        let version = self.context.i32_type().const_int(
            inkwell::debug_info::debug_metadata_version() as u64,
            false,
        );
        self.module.add_basic_value_flag(
            "Debug Info Version",
            inkwell::module::FlagBehavior::Warning,
            version,
        );
    }

    /// 切换当前编译文件的 DIFile:主程序传 `None`,文件模块传其路径。
    pub(super) fn use_debug_file(&mut self, path: Option<&str>) {
        let Some(state) = self.debug.as_mut() else {
            return;
        };
        let file = match path {
            None => state.main_file,
            Some(p) => {
                let (filename, directory) = split_source_path(p);
                let key = format!("{}\0{}", directory, filename);
                match state.files.get(&key) {
                    Some(file) => *file,
                    None => {
                        let file = state.builder.create_file(&filename, &directory);
                        state.files.insert(key, file);
                        file
                    }
                }
            }
        };
        state.current_file = file;
    }

    /// 把当前语句的源码位置设为 builder 的 debug location,
    /// 之后生成的指令都归属该行。
    pub(super) fn set_current_debug_span(&mut self, span: Span) {
        let (Some(state), Some(sp)) = (self.debug.as_ref(), self.current_subprogram) else {
            return;
        };
        let scope = sp.as_debug_info_scope();
        let loc = state.builder.create_debug_location(
            self.context,
            span.start_line() as u32,
            span.start_column() as u32,
            scope,
            None,
        );
        self.builder.set_current_debug_location(loc);
    }

    /// 清除 builder 的 debug location(进入新函数时避免归属错乱)。
    pub(super) fn clear_debug_location(&mut self) {
        if self.debug.is_some() {
            self.builder.unset_current_debug_location();
        }
    }

    /// 为函数创建 DISubprogram(供 `add_function` 挂载)。
    /// 类型无法映射的参数会导致整个子程序放弃(返回 None)。
    pub(super) fn create_subprogram(
        &self,
        name: &str,
        line: u32,
        param_types: &[BasicTypeEnum<'ctx>],
        return_type: BasicTypeEnum<'ctx>,
    ) -> Option<DISubprogram<'ctx>> {
        let state = self.debug.as_ref()?;
        let file = state.current_file;
        let mut param_dis = Vec::with_capacity(param_types.len());
        for ty in param_types {
            param_dis.push(self.di_type(*ty)?);
        }
        let ret_di = self.di_type(return_type)?;
        let ty = state
            .builder
            .create_subroutine_type(file, Some(ret_di), &param_dis, DIFlags::ZERO);
        let scope = state.compile_unit.as_debug_info_scope();
        Some(state.builder.create_function(
            scope,
            name,
            None,
            file,
            line,
            ty,
            false, // 是否单元局部 (is_local_to_unit)
            true,  // is_definition
            line,  // 作用域起始行 (scope_line)
            DIFlags::ZERO,
            false, // is_optimized
        ))
    }

    /// 为局部变量(参数之外的 alloca)挂 `llvm.dbg.declare`。
    pub(super) fn declare_local(
        &mut self,
        name: &str,
        ptr: PointerValue<'ctx>,
        ty: BasicTypeEnum<'ctx>,
        span: Span,
    ) {
        let (Some(state), Some(sp)) = (self.debug.as_ref(), self.current_subprogram) else {
            return;
        };
        let Some(di) = self.di_type(ty) else {
            return;
        };
        let file = state.current_file;
        let line = span.start_line() as u32;
        let scope = sp.as_debug_info_scope();
        let var = state
            .builder
            .create_auto_variable(scope, name, file, line, di, true, DIFlags::ZERO, 0);
        let loc = state.builder.create_debug_location(
            self.context,
            line,
            span.start_column() as u32,
            scope,
            None,
        );
        self.insert_debug_declare(state, ptr, Some(var), loc);
    }

    /// 为函数参数挂 `llvm.dbg.declare`(`arg_no` 从 1 开始)。
    pub(super) fn declare_param(
        &mut self,
        name: &str,
        ptr: PointerValue<'ctx>,
        ty: BasicTypeEnum<'ctx>,
        arg_no: u32,
        line: u32,
    ) {
        let (Some(state), Some(sp)) = (self.debug.as_ref(), self.current_subprogram) else {
            return;
        };
        let Some(di) = self.di_type(ty) else {
            return;
        };
        let file = state.current_file;
        let scope = sp.as_debug_info_scope();
        let var = state.builder.create_parameter_variable(
            scope, name, arg_no, file, line, di, true, DIFlags::ZERO,
        );
        let loc = state
            .builder
            .create_debug_location(self.context, line, 0, scope, None);
        self.insert_debug_declare(state, ptr, Some(var), loc);
    }

    /// 在 entry 块末尾(有终止指令时插到终止指令之前)插入 dbg.declare。
    fn insert_debug_declare(
        &self,
        state: &DebugState<'ctx>,
        ptr: PointerValue<'ctx>,
        var: Option<inkwell::debug_info::DILocalVariable<'ctx>>,
        loc: inkwell::debug_info::DILocation<'ctx>,
    ) {
        let entry = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .and_then(|f| f.get_first_basic_block());
        let Some(entry) = entry else {
            return;
        };
        match entry.get_terminator() {
            Some(term) => {
                state
                    .builder
                    .insert_declare_before_instruction(ptr, var, None, loc, term);
            }
            None => {
                state.builder.insert_declare_at_end(ptr, var, None, loc, entry);
            }
        }
    }

    /// 解析 DI 类型:整型/浮点映射为基本类型,指针映射为 char*,
    /// 结构体/枚举/元组映射为复合类型。无法表达时返回 None。
    fn di_type(&self, ty: BasicTypeEnum<'ctx>) -> Option<DIType<'ctx>> {
        match ty {
            BasicTypeEnum::IntType(t) => {
                let (name, encoding) = match t.get_bit_width() {
                    1 => ("bool", DW_ATE_BOOLEAN),
                    8 => ("char", DW_ATE_UTF),
                    64 => ("i64", DW_ATE_SIGNED),
                    _ => ("i32", DW_ATE_SIGNED),
                };
                self.di_basic_type(name, t.get_bit_width() as u64, encoding)
            }
            BasicTypeEnum::FloatType(t) => {
                if t == self.context.f64_type() {
                    self.di_basic_type("f64", 64, DW_ATE_FLOAT)
                } else {
                    self.di_basic_type("f32", 32, DW_ATE_FLOAT)
                }
            }
            BasicTypeEnum::PointerType(_) => {
                let pointee = self.di_basic_type("char", 8, DW_ATE_UTF)?;
                let state = self.debug.as_ref()?;
                Some(
                    state
                        .builder
                        .create_pointer_type("", pointee, 64, 8, AddressSpace::default())
                        .as_type(),
                )
            }
            BasicTypeEnum::StructType(st) => self.di_struct_type(st),
            BasicTypeEnum::ArrayType(_) => None,
            BasicTypeEnum::VectorType(_) => None,
        }
    }

    fn di_basic_type(&self, name: &str, bits: u64, encoding: u32) -> Option<DIType<'ctx>> {
        let state = self.debug.as_ref()?;
        state
            .builder
            .create_basic_type(name, bits, encoding, DIFlags::ZERO)
            .ok()
            .map(|t| t.as_type())
    }

    /// 结构体/数据枚举/元组的 DICompositeType,成员偏移按 LLVM 布局计算。
    fn di_struct_type(
        &self,
        st: inkwell::types::StructType<'ctx>,
    ) -> Option<DIType<'ctx>> {
        let state = self.debug.as_ref()?;
        let file = state.current_file;
        let scope = state.compile_unit.as_debug_info_scope();

        // 数据携带枚举布局为 { i32 tag, payload union }。
        if let Some(info) = self.enum_data_by_type(st.into()) {
            let name = info.name.clone();
            let payload_union = info.payload_union?;
            let payloads: Vec<BasicTypeEnum<'ctx>> = info
                .variants
                .iter()
                .filter_map(|v| v.payload)
                .collect();
            let members: Vec<DIType<'ctx>> = payloads
                .iter()
                .map(|p| self.di_type(*p))
                .collect::<Option<Vec<_>>>()?;
            let (usize_, ualign) = llvm_size_align(payload_union.into());
            let union_di = state.builder.create_union_type(
                scope,
                &format!("{}.payload", name),
                file,
                0,
                usize_ * 8,
                ualign,
                DIFlags::ZERO,
                &members,
                0,
                "",
            );
            let tag_di = self.di_basic_type("i32", 32, DW_ATE_SIGNED)?;
            let (size, align) = llvm_size_align(st.into());
            let di = state.builder.create_struct_type(
                scope,
                &name,
                file,
                0,
                size * 8,
                align,
                DIFlags::ZERO,
                None,
                &[tag_di, union_di.as_type()],
                0,
                None,
                &format!("huzi.{}.enum", name),
            );
            return Some(di.as_type());
        }

        // 已注册结构体或匿名元组。
        let fields: Vec<BasicTypeEnum<'ctx>> = (0..st.count_fields())
            .map(|i| st.get_field_type_at_index(i).unwrap())
            .collect();
        let (name, member_names, unique_id) = self.describe_struct(st, &fields);
        let offsets = struct_member_offsets(&fields);
        let mut members = Vec::with_capacity(fields.len());
        for (i, field) in fields.iter().enumerate() {
            let field_di = self.di_type(*field)?;
            let (size, align) = llvm_size_align(*field);
            let member = state.builder.create_member_type(
                scope,
                &member_names[i],
                file,
                0,
                size * 8,
                align,
                offsets[i] * 8,
                DIFlags::ZERO,
                field_di,
            );
            members.push(member.as_type());
        }
        let (size, align) = llvm_size_align(st.into());
        let di = state.builder.create_struct_type(
            scope,
            &name,
            file,
            0,
            size * 8,
            align,
            DIFlags::ZERO,
            None,
            &members,
            0,
            None,
            &unique_id,
        );
        Some(di.as_type())
    }

    /// 结构体的 (DI 名, 成员名, 唯一 id):已注册结构体用原名,
    /// 其余视为元组,成员名 ".0"、".1"…,id 由成员宽度签名生成。
    fn describe_struct(
        &self,
        st: inkwell::types::StructType<'ctx>,
        fields: &[BasicTypeEnum<'ctx>],
    ) -> (String, Vec<String>, String) {
        if let Some((name, info)) = self
            .structs
            .iter()
            .find(|(_, (s, _))| *s == st)
            .map(|(n, (_, f))| (n.clone(), f))
        {
            let member_names: Vec<String> = info.iter().map(|f| f.name.clone()).collect();
            let unique_id = format!("huzi.{}", name);
            return (name, member_names, unique_id);
        }
        let signature: Vec<String> = fields
            .iter()
            .map(|f| match f {
                BasicTypeEnum::IntType(t) => format!("i{}", t.get_bit_width()),
                BasicTypeEnum::FloatType(_) => {
                    format!("f{}", llvm_size_align(*f).0 * 8)
                }
                BasicTypeEnum::PointerType(_) => "p".to_string(),
                BasicTypeEnum::StructType(_) => "s".to_string(),
                BasicTypeEnum::ArrayType(_) => "a".to_string(),
                BasicTypeEnum::VectorType(_) => "v".to_string(),
            })
            .collect();
        let member_names: Vec<String> = (0..fields.len()).map(|i| format!(".{}", i)).collect();
        (
            String::new(),
            member_names,
            format!("huzi.tuple.{}", signature.join("_")),
        )
    }

    /// 编译收尾:解析 DIBuilder 的未完成元数据。
    pub(super) fn finalize_debug_info(&mut self) {
        if let Some(state) = &self.debug {
            state.builder.finalize();
        }
    }}
