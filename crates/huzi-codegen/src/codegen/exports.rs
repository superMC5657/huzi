//! 模块符号重导出 (Re-export / Symbol Aliasing)。
//!
//! 支持 `export calc`, `export calc::*`, `export calc::add` 等导出声明，
//! 在编译期直接建立符号别名映射，避免运行时产生冗余的包装函数嵌套开销。

use super::{CodeGen, ModuleCode};
use huzi_ast::Stmt;
use huzi_error::Result;

impl<'ctx> CodeGen<'ctx> {
    /// 阶段 1: 在类型与函数签名注册后，重导出类型 (struct/enum) 与 AST 签名。
    pub(super) fn apply_module_export_signatures(&mut self, modules: &[ModuleCode]) -> Result<()> {
        for m in modules {
            let Some(prog) = &m.program else { continue };
            let pkg_prefix = &m.name;

            for stmt in &prog.statements {
                let Stmt::Export(exp) = &stmt.node else { continue };

                if exp.is_wildcard {
                    // e.g. export calc::*
                    let target_prefix = format!("{}::", exp.path);

                    let matching_structs: Vec<(String, String)> = self
                        .structs
                        .keys()
                        .filter(|k| k.starts_with(&target_prefix))
                        .map(|k| {
                            let item_name = k.strip_prefix(&target_prefix).unwrap();
                            (k.clone(), format!("{}::{}", pkg_prefix, item_name))
                        })
                        .collect();
                    for (src_key, alias_key) in matching_structs {
                        if let Some(st) = self.structs.get(&src_key).cloned() {
                            self.structs.insert(alias_key, st);
                        }
                    }

                    let matching_enums: Vec<(String, String)> = self
                        .enums
                        .keys()
                        .filter(|k| k.starts_with(&target_prefix))
                        .map(|k| {
                            let item_name = k.strip_prefix(&target_prefix).unwrap();
                            (k.clone(), format!("{}::{}", pkg_prefix, item_name))
                        })
                        .collect();
                    for (src_key, alias_key) in matching_enums {
                        if let Some(e) = self.enums.get(&src_key).cloned() {
                            self.enums.insert(alias_key, e);
                        }
                    }
                } else if exp.path.contains("::") {
                    // e.g. export calc::MyStruct
                    let src_key = &exp.path;
                    let item_name = exp.path.rsplit("::").next().unwrap_or(&exp.path);
                    let alias_key = format!("{}::{}", pkg_prefix, item_name);

                    if let Some(st) = self.structs.get(src_key).cloned() {
                        self.structs.insert(alias_key.clone(), st);
                    }
                    if let Some(e) = self.enums.get(src_key).cloned() {
                        self.enums.insert(alias_key, e);
                    }
                } else {
                    // e.g. export calc
                    let target_prefix = format!("{}::", exp.path);

                    let matching_structs: Vec<(String, String)> = self
                        .structs
                        .keys()
                        .filter(|k| k.starts_with(&target_prefix))
                        .map(|k| {
                            let sub_path = k.strip_prefix(&target_prefix).unwrap();
                            (k.clone(), format!("{}::{}::{}", pkg_prefix, exp.path, sub_path))
                        })
                        .collect();
                    for (src_key, alias_key) in matching_structs {
                        if let Some(st) = self.structs.get(&src_key).cloned() {
                            self.structs.insert(alias_key, st);
                        }
                    }

                    let matching_enums: Vec<(String, String)> = self
                        .enums
                        .keys()
                        .filter(|k| k.starts_with(&target_prefix))
                        .map(|k| {
                            let sub_path = k.strip_prefix(&target_prefix).unwrap();
                            (k.clone(), format!("{}::{}::{}", pkg_prefix, exp.path, sub_path))
                        })
                        .collect();
                    for (src_key, alias_key) in matching_enums {
                        if let Some(e) = self.enums.get(&src_key).cloned() {
                            self.enums.insert(alias_key, e);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// 阶段 2: 在模块函数体编译完成后，为 LLVM FunctionValue 与 AST 签名建立别名映射。
    pub(super) fn apply_module_export_functions(&mut self, modules: &[ModuleCode]) -> Result<()> {
        for m in modules {
            let Some(prog) = &m.program else { continue };
            let pkg_prefix = &m.name;

            for stmt in &prog.statements {
                let Stmt::Export(exp) = &stmt.node else { continue };

                if exp.is_wildcard {
                    // e.g. export calc::*
                    let target_prefix = format!("{}::", exp.path);

                    let matching_fns: Vec<(String, String)> = self
                        .functions
                        .keys()
                        .filter(|k| k.starts_with(&target_prefix))
                        .map(|k| {
                            let item_name = k.strip_prefix(&target_prefix).unwrap();
                            (k.clone(), format!("{}::{}", pkg_prefix, item_name))
                        })
                        .collect();

                    for (src_key, alias_key) in matching_fns {
                        if let Some(fn_val) = self.functions.get(&src_key).cloned() {
                            self.functions.insert(alias_key.clone(), fn_val);
                        }
                        if let Some(ast) = self.fn_param_ast.get(&src_key).cloned() {
                            self.fn_param_ast.insert(alias_key.clone(), ast);
                        }
                        if let Some(ret) = self.fn_return_ast.get(&src_key).cloned() {
                            self.fn_return_ast.insert(alias_key.clone(), ret);
                        }
                        if let Some(&no_ret) = self.fn_no_return.get(&src_key) {
                            self.fn_no_return.insert(alias_key, no_ret);
                        }
                    }
                } else if exp.path.contains("::") {
                    // e.g. export calc::add
                    let src_key = &exp.path;
                    let item_name = exp.path.rsplit("::").next().unwrap_or(&exp.path);
                    let alias_key = format!("{}::{}", pkg_prefix, item_name);

                    if let Some(fn_val) = self.functions.get(src_key).cloned() {
                        self.functions.insert(alias_key.clone(), fn_val);
                    }
                    if let Some(ast) = self.fn_param_ast.get(src_key).cloned() {
                        self.fn_param_ast.insert(alias_key.clone(), ast);
                    }
                    if let Some(ret) = self.fn_return_ast.get(src_key).cloned() {
                        self.fn_return_ast.insert(alias_key.clone(), ret);
                    }
                    if let Some(&no_ret) = self.fn_no_return.get(src_key) {
                        self.fn_no_return.insert(alias_key, no_ret);
                    }
                } else {
                    // e.g. export calc
                    // Re-export submodule hierarchy: calc::add -> my_math::calc::add
                    let target_prefix = format!("{}::", exp.path);

                    let matching_fns: Vec<(String, String)> = self
                        .functions
                        .keys()
                        .filter(|k| k.starts_with(&target_prefix))
                        .map(|k| {
                            let sub_path = k.strip_prefix(&target_prefix).unwrap();
                            (k.clone(), format!("{}::{}::{}", pkg_prefix, exp.path, sub_path))
                        })
                        .collect();

                    for (src_key, alias_key) in matching_fns {
                        if let Some(fn_val) = self.functions.get(&src_key).cloned() {
                            self.functions.insert(alias_key.clone(), fn_val);
                        }
                        if let Some(ast) = self.fn_param_ast.get(&src_key).cloned() {
                            self.fn_param_ast.insert(alias_key.clone(), ast);
                        }
                        if let Some(ret) = self.fn_return_ast.get(&src_key).cloned() {
                            self.fn_return_ast.insert(alias_key.clone(), ret);
                        }
                        if let Some(&no_ret) = self.fn_no_return.get(&src_key) {
                            self.fn_no_return.insert(alias_key, no_ret);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
