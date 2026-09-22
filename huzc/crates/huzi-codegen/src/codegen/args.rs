use super::CodeGen;
use huzi_ast::Expr;
use huzi_error::{HuziError, Result};
use inkwell::module::Linkage;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue};
use inkwell::{AddressSpace, IntPredicate};

impl<'ctx> CodeGen<'ctx> {
    /// Declare the module-level state backing the arg/EOF builtins:
    /// `huzi_argc`/`huzi_argv` are captured from the C `main` entry
    /// signature, `huzi_eof` is set by the read_* builtins when stdin
    /// returns EOF, and `huzi_empty_str` backs out-of-range `arg(i)`.
    pub(super) fn declare_arg_support(&mut self) {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        let argc = self
            .module
            .add_global(i32_type, Some(AddressSpace::default()), "huzi_argc");
        argc.set_linkage(Linkage::Private);
        argc.set_initializer(&i32_type.const_int(0, false));

        let argv = self
            .module
            .add_global(ptr_type, Some(AddressSpace::default()), "huzi_argv");
        argv.set_linkage(Linkage::Private);
        argv.set_initializer(&ptr_type.const_null());

        let eof = self
            .module
            .add_global(i32_type, Some(AddressSpace::default()), "huzi_eof");
        eof.set_linkage(Linkage::Private);
        eof.set_initializer(&i32_type.const_int(0, false));

        let empty_str_type = self.context.i8_type().array_type(1);
        let empty_str = self
            .module
            .add_global(empty_str_type, Some(AddressSpace::default()), "huzi_empty_str");
        empty_str.set_linkage(Linkage::Private);
        empty_str.set_initializer(&empty_str_type.const_zero());
    }

    /// Store the hidden `main(argc, argv)` parameters into the globals the
    /// arg builtins read from. Called at the top of the entry block. On
    /// Windows the stored ANSI values are then replaced with UTF-8 strings
    /// converted from the Unicode command line (see `args_utf8`).
    pub(super) fn store_main_args(&mut self, function: FunctionValue<'ctx>) {
        let argc_global = self.arg_global("huzi_argc");
        let argv_global = self.arg_global("huzi_argv");
        let argc = function.get_nth_param(0).unwrap().into_int_value();
        let argv = function.get_nth_param(1).unwrap().into_pointer_value();
        self.builder.build_store(argc_global, argc).unwrap();
        self.builder.build_store(argv_global, argv).unwrap();
        if cfg!(windows) {
            self.refresh_windows_argv_utf8();
        }
    }

    /// OR the given `eof_hit` condition (i1) into the sticky EOF flag.
    pub(super) fn mark_eof_flag(&mut self, eof_hit: IntValue<'ctx>) {
        let eof_global = self.arg_global("huzi_eof");
        let i32_type = self.context.i32_type();
        let old = self
            .builder
            .build_load(i32_type, eof_global, "eof_old")
            .unwrap()
            .into_int_value();
        let hit = self
            .builder
            .build_int_z_extend(eof_hit, i32_type, "eof_hit_i32")
            .unwrap();
        let flag = self.builder.build_or(old, hit, "eof_flag").unwrap();
        self.builder.build_store(eof_global, flag).unwrap();
    }

    /// `arg_count() -> i32`: number of command-line arguments (argv[0] included).
    pub(super) fn compile_arg_count(&mut self) -> Result<BasicValueEnum<'ctx>> {
        let argc = self.load_argc()?;
        Ok(argc.into())
    }

    /// `arg(i) -> str`: the i-th command-line argument, or an empty string
    /// when the index is negative or past the end. The returned pointer
    /// aliases argv storage; it is not copied.
    pub(super) fn compile_arg(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("arg() requires exactly 1 argument"));
        }
        let idx_value = self.compile_expr(&arguments[0])?;
        let i32_type = self.context.i32_type();
        let idx = match idx_value {
            BasicValueEnum::IntValue(iv) => match iv.get_type().get_bit_width() {
                32 => iv,
                w if w < 32 => self
                    .builder
                    .build_int_s_extend(iv, i32_type, "arg_idx")
                    .unwrap(),
                _ => self.builder.build_int_truncate(iv, i32_type, "arg_idx").unwrap(),
            },
            _ => return Err(HuziError::new_global("arg() requires an integer index")),
        };

        let argc = self.load_argc()?;
        let ge_zero = self
            .builder
            .build_int_compare(IntPredicate::SGE, idx, i32_type.const_int(0, false), "arg_ge_zero")
            .unwrap();
        let lt_argc = self
            .builder
            .build_int_compare(IntPredicate::SLT, idx, argc, "arg_lt_argc")
            .unwrap();
        let in_range = self
            .builder
            .build_and(ge_zero, lt_argc, "arg_in_range")
            .unwrap();

        let function = self.current_function()?;
        let ok_block = self.context.append_basic_block(function, "arg_ok");
        let out_block = self.context.append_basic_block(function, "arg_out");
        let merge_block = self.context.append_basic_block(function, "arg_merge");
        self.builder
            .build_conditional_branch(in_range, ok_block, out_block)
            .unwrap();

        self.builder.position_at_end(ok_block);
        let argv = self.load_argv()?;
        let slot = unsafe {
            self.builder
                .build_gep(self.context.ptr_type(AddressSpace::default()), argv, &[idx], "arg_slot")
                .unwrap()
        };
        let arg_ptr = self
            .builder
            .build_load(self.context.ptr_type(AddressSpace::default()), slot, "arg_ptr")
            .unwrap();
        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        self.builder.position_at_end(out_block);
        let empty_ptr = self.arg_global("huzi_empty_str");
        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        self.builder.position_at_end(merge_block);
        let phi = self
            .builder
            .build_phi(self.context.ptr_type(AddressSpace::default()), "arg_value")
            .unwrap();
        phi.add_incoming(&[(&arg_ptr, ok_block), (&empty_ptr, out_block)]);
        Ok(phi.as_basic_value())
    }

    /// `arg_ok(i) -> bool`: whether index `i` names a valid argv entry
    /// (`0 <= i < arg_count()`). Use it to probe before `arg(i)` instead
    /// of guessing from an empty string.
    pub(super) fn compile_arg_ok(&mut self, arguments: &[Expr]) -> Result<BasicValueEnum<'ctx>> {
        if arguments.len() != 1 {
            return Err(HuziError::new_global("arg_ok() requires exactly 1 argument"));
        }
        let idx = self.coerce_arg_index(&arguments[0])?;
        Ok(self.arg_idx_in_range(idx)?.into())
    }

    /// Coerce an `arg(i)`/`arg_ok(i)` index expression to i32.
    fn coerce_arg_index(&mut self, expr: &Expr) -> Result<IntValue<'ctx>> {
        let i32_type = self.context.i32_type();
        match self.compile_expr(expr)? {
            BasicValueEnum::IntValue(iv) => match iv.get_type().get_bit_width() {
                32 => Ok(iv),
                w if w < 32 => Ok(self
                    .builder
                    .build_int_s_extend(iv, i32_type, "arg_idx")
                    .unwrap()),
                _ => Ok(self.builder.build_int_truncate(iv, i32_type, "arg_idx").unwrap()),
            },
            _ => Err(HuziError::new_global("arg_ok() requires an integer index")),
        }
    }

    /// `0 <= idx < argc` as an i1 value (negative indices fail the
    /// signed lower-bound check, matching `arg(i)` out-of-range rules).
    fn arg_idx_in_range(&mut self, idx: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        let i32_type = self.context.i32_type();
        let argc = self.load_argc()?;
        let ge_zero = self
            .builder
            .build_int_compare(IntPredicate::SGE, idx, i32_type.const_int(0, false), "arg_ge_zero")
            .unwrap();
        let lt_argc = self
            .builder
            .build_int_compare(IntPredicate::SLT, idx, argc, "arg_lt_argc")
            .unwrap();
        Ok(self.builder.build_and(ge_zero, lt_argc, "arg_in_range").unwrap())
    }

    /// `is_eof() -> bool`: whether a previous read_* hit end of stdin.
    pub(super) fn compile_is_eof(&mut self) -> Result<BasicValueEnum<'ctx>> {
        let eof_global = self.arg_global("huzi_eof");
        let flag = self
            .builder
            .build_load(self.context.i32_type(), eof_global, "eof_flag")
            .unwrap()
            .into_int_value();
        let is_set = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                flag,
                self.context.i32_type().const_int(0, false),
                "eof_set",
            )
            .unwrap();
        Ok(is_set.into())
    }

    fn arg_global(&self, name: &str) -> inkwell::values::PointerValue<'ctx> {
        self.module
            .get_global(name)
            .expect("arg support globals are declared in prelude")
            .as_pointer_value()
    }

    fn load_argc(&self) -> Result<IntValue<'ctx>> {
        let argc_global = self.arg_global("huzi_argc");
        let argc = self
            .builder
            .build_load(self.context.i32_type(), argc_global, "argc")
            .unwrap();
        Ok(argc.into_int_value())
    }

    fn load_argv(&self) -> Result<inkwell::values::PointerValue<'ctx>> {
        let argv_global = self.arg_global("huzi_argv");
        let argv = self
            .builder
            .build_load(self.context.ptr_type(AddressSpace::default()), argv_global, "argv")
            .unwrap();
        Ok(argv.into_pointer_value())
    }
}
