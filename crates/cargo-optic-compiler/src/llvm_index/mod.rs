//! Indexes exact definitions in disassembled LLVM text.
//!
//! The scanner retains bounded header tokens and streams past function bodies and unrelated lines.
//! LLVM verification, artifact selection, and alias resolution belong to the caller.

mod header;

mod scanner;
pub(crate) use scanner::index;

mod stream;

#[cfg(test)]
mod tests;
