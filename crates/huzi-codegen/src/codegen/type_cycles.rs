use super::CodeGen;
use huzi_ast::*;
use huzi_error::{HuziError, Result};
use std::collections::HashMap;

impl<'ctx> CodeGen<'ctx> {
    /// Reject by-value reference cycles (A -> B -> A) among struct and enum
    /// definitions, which have no finite layout. Array fields decay to
    /// pointers so they cannot form one.
    pub(super) fn check_type_cycles(&self, structs: &[StructDef], enums: &[EnumDef]) -> Result<()> {
        let mut names: Vec<&str> = structs.iter().map(|d| d.name.as_str()).collect();
        names.extend(enums.iter().map(|d| d.name.as_str()));

        let mut refs: HashMap<&str, Vec<&str>> = HashMap::new();
        for def in structs {
            let field_types: Vec<&str> = def
                .fields
                .iter()
                .filter_map(|f| match &f.field_type {
                    Type::Named(n) if names.contains(&n.as_str()) => Some(n.as_str()),
                    _ => None,
                })
                .collect();
            refs.insert(def.name.as_str(), field_types);
        }
        for def in enums {
            let payload_types: Vec<&str> = def
                .variants
                .iter()
                .flat_map(|v| v.payloads.iter())
                .filter_map(|t| match t {
                    Type::Named(n) if names.contains(&n.as_str()) => Some(n.as_str()),
                    _ => None,
                })
                .collect();
            refs.insert(def.name.as_str(), payload_types);
        }

        fn has_cycle(node: &str, refs: &HashMap<&str, Vec<&str>>, path: &mut Vec<String>) -> bool {
            if path.iter().any(|n| n == node) {
                return true;
            }
            if let Some(children) = refs.get(node) {
                path.push(node.to_string());
                for child in children {
                    if has_cycle(child, refs, path) {
                        return true;
                    }
                }
                path.pop();
            }
            false
        }

        for def in structs.iter().map(|d| &d.name).chain(enums.iter().map(|d| &d.name)) {
            if has_cycle(def, &refs, &mut Vec::new()) {
                return Err(HuziError::new_global(format!(
                    "Type '{}' is part of a by-value reference cycle",
                    def
                )));
            }
        }

        Ok(())
    }
}
