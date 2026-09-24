//! 结构体/数据枚举/元组的 DICompositeType(自 `debuginfo.rs` 纯搬移,零逻辑变化)。

use super::{CodeGen, DW_ATE_SIGNED, llvm_size_align, struct_member_offsets};
use inkwell::debug_info::{AsDIScope, DIFlags, DIFlagsConstants, DIType};
use inkwell::types::BasicTypeEnum;

impl<'ctx> CodeGen<'ctx> {
    /// 结构体/数据枚举/元组的 DICompositeType,成员偏移按 LLVM 布局计算。
    pub(super) fn di_struct_type(
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
}
