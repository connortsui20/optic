; This fixture retains the pinned LLVM 22.1.8 disassembler's spellings.
source_filename = "llvm-index"
target triple = "x86_64-unknown-linux-gnu"

@data = global [2 x i8] c"xy"
@direct = linkonce_odr hidden unnamed_addr alias void (), ptr @"f\22\\\01\C3\A9"
@expression = alias i8, getelementptr inbounds ([2 x i8], ptr @data, i64 0, i64 1)
@indirect = ifunc void (), ptr @resolver

declare void @declaration()

define void @"f\22\\\01\C3\A9"() prefix { i32, i8 } { i32 1, i8 2 } {
entry:
  call void asm sideeffect "; {\22}", ""()
  ret void
}

define ptr @resolver() {
entry:
  ret ptr @"f\22\\\01\C3\A9"
}
