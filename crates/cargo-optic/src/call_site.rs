//! Structural classification of LLVM call sites.
//!
//! [`CallSiteSummary`] counts each `call`, `invoke`, and `callbr` instruction exactly once. It keeps
//! runtime calls separate from LLVM intrinsics that express memory operations, assumptions,
//! lifetimes, or metadata. The parser uses LLVM syntax only. It does not infer dynamic call counts.

use serde::{Deserialize, Serialize};

/// Structural call-site counts for one LLVM body or body set.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CallSiteSummary {
    /// All classified `call`, `invoke`, and `callbr` instructions.
    pub total: usize,

    /// Direct calls to non-intrinsic global symbols.
    pub direct_non_intrinsic: usize,

    /// Calls through an SSA value or another computed callee.
    pub indirect: usize,

    /// Calls to inline assembly.
    pub inline_asm: usize,

    /// Calls to LLVM memory intrinsics.
    pub memory_intrinsics: usize,

    /// Calls to LLVM assumption intrinsics.
    pub assumption_intrinsics: usize,

    /// Calls to LLVM lifetime intrinsics.
    pub lifetime_intrinsics: usize,

    /// Calls to LLVM debug, probe, or alias-scope metadata intrinsics.
    pub metadata_only_intrinsics: usize,

    /// Calls to all other LLVM intrinsics.
    pub other_intrinsics: usize,
}

impl CallSiteSummary {
    pub(crate) fn record_line(&mut self, line: &str) -> bool {
        let Some(kind) = classify_call_site(line) else {
            return false;
        };

        self.total += 1;
        match kind {
            CallSiteKind::DirectNonIntrinsic => self.direct_non_intrinsic += 1,
            CallSiteKind::Indirect => self.indirect += 1,
            CallSiteKind::InlineAssembly => self.inline_asm += 1,
            CallSiteKind::MemoryIntrinsic => self.memory_intrinsics += 1,
            CallSiteKind::AssumptionIntrinsic => self.assumption_intrinsics += 1,
            CallSiteKind::LifetimeIntrinsic => self.lifetime_intrinsics += 1,
            CallSiteKind::MetadataOnlyIntrinsic => self.metadata_only_intrinsics += 1,
            CallSiteKind::OtherIntrinsic => self.other_intrinsics += 1,
        }

        true
    }

    pub(crate) fn add_assign(&mut self, other: &Self) {
        self.total += other.total;
        self.direct_non_intrinsic += other.direct_non_intrinsic;
        self.indirect += other.indirect;
        self.inline_asm += other.inline_asm;
        self.memory_intrinsics += other.memory_intrinsics;
        self.assumption_intrinsics += other.assumption_intrinsics;
        self.lifetime_intrinsics += other.lifetime_intrinsics;
        self.metadata_only_intrinsics += other.metadata_only_intrinsics;
        self.other_intrinsics += other.other_intrinsics;
    }

    #[cfg(test)]
    fn classified_total(&self) -> usize {
        self.direct_non_intrinsic
            + self.indirect
            + self.inline_asm
            + self.memory_intrinsics
            + self.assumption_intrinsics
            + self.lifetime_intrinsics
            + self.metadata_only_intrinsics
            + self.other_intrinsics
    }
}

/// Signed changes in structural call-site counts.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CallSiteDelta {
    /// Change in all classified call sites.
    pub total: i128,

    /// Change in direct calls to non-intrinsic global symbols.
    pub direct_non_intrinsic: i128,

    /// Change in calls through an SSA value or another computed callee.
    pub indirect: i128,

    /// Change in calls to inline assembly.
    pub inline_asm: i128,

    /// Change in calls to LLVM memory intrinsics.
    pub memory_intrinsics: i128,

    /// Change in calls to LLVM assumption intrinsics.
    pub assumption_intrinsics: i128,

    /// Change in calls to LLVM lifetime intrinsics.
    pub lifetime_intrinsics: i128,

    /// Change in calls to LLVM debug, probe, or alias-scope metadata intrinsics.
    pub metadata_only_intrinsics: i128,

    /// Change in calls to all other LLVM intrinsics.
    pub other_intrinsics: i128,
}

impl CallSiteDelta {
    pub(crate) fn between(before: &CallSiteSummary, after: &CallSiteSummary) -> Self {
        Self {
            total: delta(before.total, after.total),
            direct_non_intrinsic: delta(before.direct_non_intrinsic, after.direct_non_intrinsic),
            indirect: delta(before.indirect, after.indirect),
            inline_asm: delta(before.inline_asm, after.inline_asm),
            memory_intrinsics: delta(before.memory_intrinsics, after.memory_intrinsics),
            assumption_intrinsics: delta(before.assumption_intrinsics, after.assumption_intrinsics),
            lifetime_intrinsics: delta(before.lifetime_intrinsics, after.lifetime_intrinsics),
            metadata_only_intrinsics: delta(
                before.metadata_only_intrinsics,
                after.metadata_only_intrinsics,
            ),
            other_intrinsics: delta(before.other_intrinsics, after.other_intrinsics),
        }
    }
}

fn delta(before: usize, after: usize) -> i128 {
    after as i128 - before as i128
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CallSiteKind {
    DirectNonIntrinsic,
    Indirect,
    InlineAssembly,
    MemoryIntrinsic,
    AssumptionIntrinsic,
    LifetimeIntrinsic,
    MetadataOnlyIntrinsic,
    OtherIntrinsic,
}

fn classify_call_site(line: &str) -> Option<CallSiteKind> {
    let instruction = instruction_after_call_opcode(line)?;

    if contains_keyword(instruction, "asm") {
        return Some(CallSiteKind::InlineAssembly);
    }

    if let Some(symbol) = callee_symbol(instruction, '@') {
        return Some(classify_global_symbol(symbol));
    }

    if callee_symbol(instruction, '%').is_some() {
        return Some(CallSiteKind::Indirect);
    }

    if contains_keyword(instruction, "bitcast")
        && let Some(symbol) = any_symbol(instruction, '@')
    {
        return Some(classify_global_symbol(symbol));
    }

    // LLVM also permits constant-expression callees such as `inttoptr`. They do not name a direct
    // global symbol, so the generated call dispatches through a computed address.
    Some(CallSiteKind::Indirect)
}

fn instruction_after_call_opcode(line: &str) -> Option<&str> {
    syntax_tokens(line).find_map(|token| {
        let is_opcode = matches!(token.text, "call" | "invoke" | "callbr");
        let has_operand = line
            .as_bytes()
            .get(token.end)
            .is_some_and(u8::is_ascii_whitespace);
        let is_symbol_name = token.start.checked_sub(1).is_some_and(|preceding| {
            matches!(line.as_bytes()[preceding], b'@' | b'%' | b'!' | b'.' | b'$')
        });

        (is_opcode && has_operand && !is_symbol_name).then_some(&line[token.end..])
    })
}

fn contains_keyword(text: &str, expected: &str) -> bool {
    syntax_tokens(text).any(|token| {
        token.text == expected
            && token.start.checked_sub(1).is_none_or(|preceding| {
                !matches!(text.as_bytes()[preceding], b'@' | b'%' | b'!' | b'.' | b'$')
            })
    })
}

fn callee_symbol(instruction: &str, sigil: char) -> Option<&str> {
    let bytes = instruction.as_bytes();
    let mut index = 0;
    let mut candidate = None;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = quoted_end(bytes, index + 1),
            current if current == sigil as u8 => {
                let symbol_start = index + 1;
                let symbol_end = if bytes.get(symbol_start) == Some(&b'"') {
                    quoted_end(bytes, symbol_start + 1)
                } else {
                    unquoted_symbol_end(bytes, symbol_start)
                };
                let following = skip_ascii_whitespace(bytes, symbol_end);

                if bytes.get(following) == Some(&b'(') {
                    candidate = Some(trim_symbol_quotes(&instruction[symbol_start..symbol_end]));
                }
                index = symbol_end;
            }
            _ => index += 1,
        }
    }

    candidate
}

fn any_symbol(instruction: &str, sigil: char) -> Option<&str> {
    let bytes = instruction.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = quoted_end(bytes, index + 1),
            current if current == sigil as u8 => {
                let symbol_start = index + 1;
                let symbol_end = if bytes.get(symbol_start) == Some(&b'"') {
                    quoted_end(bytes, symbol_start + 1)
                } else {
                    unquoted_symbol_end(bytes, symbol_start)
                };

                return Some(trim_symbol_quotes(&instruction[symbol_start..symbol_end]));
            }
            _ => index += 1,
        }
    }

    None
}

fn quoted_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = (index + 2).min(bytes.len()),
            b'"' => return index + 1,
            _ => index += 1,
        }
    }

    bytes.len()
}

fn unquoted_symbol_end(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'$' | b'_' | b'\\')
    }) {
        index += 1;
    }

    index
}

fn skip_ascii_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }

    index
}

fn trim_symbol_quotes(symbol: &str) -> &str {
    symbol
        .strip_prefix('"')
        .and_then(|symbol| symbol.strip_suffix('"'))
        .unwrap_or(symbol)
}

fn classify_global_symbol(symbol: &str) -> CallSiteKind {
    let Some(intrinsic) = symbol.strip_prefix("llvm.") else {
        return CallSiteKind::DirectNonIntrinsic;
    };

    if is_intrinsic_family(intrinsic, "memcpy")
        || is_intrinsic_family(intrinsic, "memmove")
        || is_intrinsic_family(intrinsic, "memset")
    {
        CallSiteKind::MemoryIntrinsic
    } else if intrinsic == "assume" {
        CallSiteKind::AssumptionIntrinsic
    } else if intrinsic.starts_with("lifetime.start") || intrinsic.starts_with("lifetime.end") {
        CallSiteKind::LifetimeIntrinsic
    } else if intrinsic.starts_with("dbg.")
        || intrinsic == "pseudoprobe"
        || intrinsic == "experimental.noalias.scope.decl"
    {
        CallSiteKind::MetadataOnlyIntrinsic
    } else {
        CallSiteKind::OtherIntrinsic
    }
}

fn is_intrinsic_family(intrinsic: &str, family: &str) -> bool {
    intrinsic
        .strip_prefix(family)
        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('.'))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SyntaxToken<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

fn syntax_tokens(text: &str) -> impl Iterator<Item = SyntaxToken<'_>> {
    let bytes = text.as_bytes();
    let mut index = 0;

    std::iter::from_fn(move || {
        while index < bytes.len() {
            match bytes[index] {
                b'"' => index = quoted_end(bytes, index + 1),
                b';' => index = bytes.len(),
                byte if byte.is_ascii_alphabetic() => {
                    let start = index;
                    index += 1;
                    while bytes
                        .get(index)
                        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    {
                        index += 1;
                    }

                    return Some(SyntaxToken {
                        text: &text[start..index],
                        start,
                        end: index,
                    });
                }
                _ => index += 1,
            }
        }

        None
    })
}

#[cfg(test)]
mod tests {
    use super::{CallSiteKind, CallSiteSummary, classify_call_site};

    #[test]
    fn classifies_runtime_call_forms() {
        let cases = [
            (
                "%result = tail call fastcc i32 @plain(i32 1)",
                CallSiteKind::DirectNonIntrinsic,
            ),
            (
                "%result = musttail call i32 %callback(i32 1)",
                CallSiteKind::Indirect,
            ),
            (
                "notail call void @\"quoted symbol\"() [ \"deopt\"(i32 0) ]",
                CallSiteKind::DirectNonIntrinsic,
            ),
            (
                "%value = invoke ptr @fallible(ptr %input) to label %ok unwind label %error",
                CallSiteKind::DirectNonIntrinsic,
            ),
            (
                "callbr void %computed() to label %fallthrough [label %other]",
                CallSiteKind::Indirect,
            ),
            (
                "call void asm sideeffect \"call fake\", \"~{dirflag}\"()",
                CallSiteKind::InlineAssembly,
            ),
            (
                "call void inttoptr (i64 4096 to ptr)()",
                CallSiteKind::Indirect,
            ),
            (
                "call i32 bitcast (i8* (...)* @legacy to i32 ()*)()",
                CallSiteKind::DirectNonIntrinsic,
            ),
        ];

        for (line, expected) in cases {
            assert_eq!(classify_call_site(line), Some(expected), "line: {line}");
        }
    }

    #[test]
    fn classifies_intrinsic_families() {
        let cases = [
            (
                "call void @llvm.memcpy.p0.p0.i64(ptr %out, ptr %in, i64 8, i1 false)",
                CallSiteKind::MemoryIntrinsic,
            ),
            (
                "call void @llvm.assume(i1 %condition)",
                CallSiteKind::AssumptionIntrinsic,
            ),
            (
                "call void @llvm.lifetime.start.p0(i64 8, ptr %slot)",
                CallSiteKind::LifetimeIntrinsic,
            ),
            (
                "call void @llvm.experimental.noalias.scope.decl(metadata !4)",
                CallSiteKind::MetadataOnlyIntrinsic,
            ),
            (
                "call void @\"llvm.dbg.value\"(metadata i32 %value, metadata !4, metadata !DIExpression())",
                CallSiteKind::MetadataOnlyIntrinsic,
            ),
            (
                "%sum = call i32 @llvm.sadd.sat.i32(i32 %left, i32 %right)",
                CallSiteKind::OtherIntrinsic,
            ),
        ];

        for (line, expected) in cases {
            assert_eq!(classify_call_site(line), Some(expected), "line: {line}");
        }
    }

    #[test]
    fn ignores_non_call_syntax_and_quoted_words() {
        assert_eq!(classify_call_site("declare void @call()"), None);
        assert_eq!(classify_call_site("%call = load ptr, ptr %slot"), None);
        assert_eq!(classify_call_site("call:"), None);
        assert_eq!(
            classify_call_site("call void @asm()"),
            Some(CallSiteKind::DirectNonIntrinsic)
        );
        assert_eq!(
            classify_call_site("store ptr @callback, ptr @\"call target\""),
            None
        );
        assert_eq!(classify_call_site("ret void ; call void @ignored()"), None);
    }

    #[test]
    fn every_call_site_has_one_category() {
        let mut summary = CallSiteSummary::default();
        for line in [
            "call void @runtime()",
            "call void %callback()",
            "call void asm \"\", \"\"()",
            "call void @llvm.memset.p0.i64(ptr %out, i8 0, i64 8, i1 false)",
            "call void @llvm.assume(i1 true)",
            "call void @llvm.lifetime.end.p0(i64 8, ptr %slot)",
            "call void @llvm.pseudoprobe(i64 1, i64 2, i32 0, i64 -1)",
            "call i64 @llvm.cttz.i64(i64 1, i1 false)",
        ] {
            assert!(summary.record_line(line));
        }

        assert_eq!(summary.total, 8);
        assert_eq!(summary.classified_total(), summary.total);
    }
}
