# Huzi 标准库源码（预留）

本目录 `huzi-src/` 规划用于存放以 Huzi 语言自身编写的高级标准库模块源码（Self-hosted Standard Library）。

## 定位与说明

当前 Huzi 核心基础函数（如 I/O、文件操作、基础数学、网络套接字等）均由编译器内置函数与 LLVM CodeGen 运行时直接提供（详见 `huzc/crates/huzi-codegen/src/codegen/builtins.rs`）。

随着语言泛型与 Trait 能力逐步完备，纯 Huzi 实现的高阶算法与数据结构（例如算法库、集合扩展等）将统一步署于本目录中。

## 示例

参见同目录下的 `sample.hz`。
