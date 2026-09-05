# Connor's Rust Style

This personal layer works with project guides such as `AGENTS.md`, `STYLE.md`, and `rustfmt.toml`.
Project guides take priority during conflicts. They define project rules such as error macros and
naming conventions.

When you write or refactor code, apply these preferences. For reviews, use them as style criteria.
The review skill controls the scope, severity, evidence, and output. Assume standard Rust idioms,
`rustfmt`, and clippy.

Write code that a junior engineer can follow in one pass. Use names, signatures, types, and doc
comments to make the code clear. The reader can usually understand the code without a function body.
Documentation can be longer than code when the contract requires more detail.

Treat clever code as a bug. If clever code is necessary, explain it. Add an obvious reference
implementation to the tests. Use code to show what it does and how it does it. Use comments to
preserve intent, constraints, and history that code cannot express.

## Scope a style pass

- Treat style as a constraint on an authorized change. Do not use style to increase the scope.
- Inspect every changed line and public contract during an aggressive pass. Do not change unrelated
  code.
- Preserve the user's edits, staging choices, and local order. If the requested change requires
  replacement, replace only the necessary user edits. Before editing, inspect the worktree. Separate
  the user's work from your work.
- Separate correctness and API changes from cosmetic changes.
- If a style improvement needs new plumbing or a public decision, report it as a separate change.
- Treat structural changes in a stacked branch as downstream-sensitive. Before moving an item,
  changing visibility, or reshaping a hot path, inspect the consumers above the branch.
- For each downstream-sensitive change, state the contract that it preserves. Check the affected
  consumers or generated code. Match the checks to the risk.

## Finish a style pass

Before handoff, inspect the complete in-scope diff. Do not inspect only the most recent edits.

- Apply every relevant style rule to every changed line.
- Inspect all affected public surfaces and consumers together.
- Search for stale names after each rename.
- Inspect organization, documentation, whitespace, and public contracts.

## Types carry the invariants

Put invariants in the type system first. Put other invariants in documentation and one validation
point. Do not keep invariants only in the reader's head.

- Use distinct types for values that have different invariants or valid operations. The internal
  representations can match.
- Design APIs so that invalid operations and invalid combinations do not compile.
- Add a type distinction only for plausible, harmful confusion.
- Establish invariants at construction and safe-mutation boundaries. Then trust the invariants
  inside the type.
- Do not repeat invariant checks in hot accessors such as `len`, `is_empty`, or indexing.
- Document each escape hatch. State what the caller must uphold.
- State whether a contract violation causes memory unsafety or an incorrect result.
- Use `assert!` for checks that safety or correctness requires. Account for the cost in every build.
- Use `debug_assert!` only for redundant diagnostics. The contract must remain valid without these
  diagnostics.
- Consider the cost of a `debug_assert!` only in debug builds.
- Prefer the borrowed context type that already exists. Do not create a parallel borrowed mirror.
- Add a separate view only for changes to the representation, fields, lifetime contract,
  capabilities, or public boundary. Document the reason.
- When call sites repeat the same match, replace the match with an enum and a `classify`-style
  constructor.

## Module layout

- Keep one concept in each file. Name the file for that concept.
- Keep `mod.rs` mostly declarative. Use it for module documentation, submodule declarations, and
  re-exports.
- A `mod.rs` can also define module-wide types, module-wide traits, and small helpers for several
  submodules.
- Put substantial algorithms, stateful workflows, and other implementation logic in named sibling
  files.
- Use `foo.rs` for a leaf module. Use `foo/mod.rs` for an inner module.
- **Never** keep a parent at `foo.rs` with children at paths such as `foo/bar.rs`.
- When a leaf gains submodules, move it to `foo/mod.rs` before you add the submodules.
- Put file-backed submodule declarations near the top.
- Put shared definitions before the modules that use them.
- Put each re-export immediately below its `mod` declaration. Use one blank line between groups.
- Do not shorten a path when its context explains ownership.
- Put `#[cfg(test)] mod tests;` last.
- Keep a type's file focused on its primary concept.
- If this placement helps the reader, keep a trait implementation with its trait or the type that
  implements it.
- If implementations or facets are distinct concerns, split them into named sibling files.
- If implementations or facets obscure the primary definition, split them into named sibling files.
- If several implementations make more sense as a group, use named sibling files.
- Choose the layout that follows the reader's path through the concept.
- If the central type is the reader's entry point, put it in the parent module. Do not hide the
  central type in a strategy or implementation leaf.
- If high-level selection and routing explain the concept, keep them beside the central type. Put
  each substantial strategy implementation in a leaf with the strategy's name.
- Put strategy-specific implementations and helpers in the strategy leaf. Keep shared contracts,
  state, and routing decisions in the parent.
- Split modules by concepts that a reader can name. Do not split modules by arbitrary file size. Use
  leaves such as dense execution, filter-and-scatter execution, and valid-only execution. Do not use
  names such as `helpers1` or `execute2`.
- After a split, read the module in this order: parent documentation, central type, router, and
  strategy leaves. Then adjust declarations, re-exports, and item placement to make this path
  direct.
- Give each capability one public entry point. Re-export it so that callers use a short path.
- Put an inline `sealed` module after the public implementation and before the test module.
- If tests outgrow one file, split them into a directory by concern.
- Before you split tests, make sure that each test is necessary.

## Functions

- Give each function one job. A long function is acceptable. Avoid deep nesting.
- When control flow has three nested levels, split the function.
- Prefer guards and early returns to a match whose arms rebuild the full result.
- Put fast paths first. Explain what each path uses and why it can differ from the slow path.
- Give each named path its own private function.
- Name intermediate values. Each name gives the reader useful information.
- Use a precise name or a small documented return type to remove explanatory comments from callers.
  Return one coherent prepared state from a setup helper. Do not return an unrelated tuple.
- Put each per-element kernel in a small function that operates on slices.
- Keep kernels separate from the plumbing that calls them.
- Use names that let the reader skip the body. Prefer long, clear names to short, ambiguous names.
- Use associated constants for fixed compile-time metadata.
- When implementations select behavior or policy, use associated functions or methods. This rule
  includes an optional initializer function.
- Use a blanket trait implementation only to make the source trait and a custom target
  implementation mutually exclusive.
- Treat a public blanket implementation as a permanent coherence decision.
- Do not add a macro that only defines a universal implementation.

## Names and public surfaces

- Name operations with clear verbs.
- Give distinct operations visibly distinct names.
- Use one name for the same operation across public surfaces.
- Apply a public rename to all consumers, documentation, and tests.
- Search for the old name after a rename.
- Do not infer compatibility requirements.
- If project policy requires compatibility, preserve the old name.

## Documentation

- Document every public field and enum variant. Do not document only the parent item.
- For a generated identifier, document its generation scheme, version, representation, and
  guarantees.
- If a private field or variant has a non-obvious invariant, document it.
- State what each field means and what it must never contain.
- Explain why the code stores a derived field.
- If names and signatures do not show a helper contract, document the private helper or test helper.
- Do not document trait-implementation methods. These methods inherit documentation from the trait.
- Add `//!` module documentation to the crate root and every `mod.rs`.
- If a file owns a distinct concept or contract, add module documentation. This includes private
  modules.
- Omit module documentation only when parent documentation fully explains the file's role.
- Give module documentation two parts.
- Start with a one-sentence summary paragraph for the rustdoc module list.
- After the summary, add a blank `//!` line.
- Explain why the module exists and where its responsibility ends.
- Do not list functions or repeat visible implementation.
- For a large public module, also explain its main entry points and the reader's path.
- If important workflows, relationships, invariants, or design choices apply, add them.
- Do not rely on the implementation to provide context that the documentation omits.
- Link to the applicable types, traits, and functions. Do not duplicate their item documentation.
- Use bold text for hard constraints, such as "this **must** be ...".
- Use `_foo_` for italics. Do not use `*` for italics.
- Use a bracket link for each item that the documentation mentions.
- Put link definitions at the bottom of the documentation block.
- If a short link occurs only once, keep it inline.
- Document the design. Do not repeat the signature.
- Explain why the item exists. Explain why the code rejects an apparent alternative.
- Define specialized terms in the module or API that owns them. Use that definition consistently. Do
  not assume that each implementation reader knows the term.
- Name both sides of an important distinction. For example, define _batch-constant_ and
  _non-constant_ together. Then use those terms consistently.
- If a collection contains valid and invalid rows, use _partially valid_. Do not use vague words
  such as _mixed_ or _varying_ without a definition.
- Prefer the formal API term in signatures and owning documentation. In local prose, use the
  clearest established term for the contrast being discussed.
- Cover null, empty, and degenerate input.
- Cite the real source for each constant, bound, or workaround. Sources include papers, DOIs, and
  issues.
- If a workaround depends on a benchmark or compiler, ask the user for evidence and reproduction
  steps.

## Comments

- Write comments for collaborators.
- Preserve information that names, types, control flow, and tests cannot express.
- Do not narrate the code.
- If clearer code communicates the same information, improve the code.
- Put shared rationale and API-wide rationale in documentation.
- Use comments only for local implementation constraints. Link to the owner. Do not repeat the
  information.
- Put correctness arguments and critical choices beside the applicable code.
- State which changes that appear harmless can break each argument.
- Explain why an apparent alternative fails. State the accepted trade-off.
- Record hard-learned requirements beside the required code.
- State the failure that occurs without the requirement.
- If the root cause is unknown, state that fact.
- For a non-obvious local constant, explain how you selected it. State the result of a change to the
  constant.
- Link external sources with stable permalinks. Explain each necessary difference from the source.
- If the implementation hides the algorithm steps, add a short outline.
- Separate the algorithm explanation from the reason for a source structure. Put the algorithm in
  owning documentation or a short outline. Put a compiler or borrow-checker constraint beside the
  exact structure that it constrains.
- Use `NB:` only for a non-obvious local constraint that a future editor can break. Do not prefix an
  ordinary explanation with `NB:`. Use only one `NB:` note in a short function.
- Put an `NB:` comment immediately before the smallest code region that it governs. If the
  constraint applies only to a branch or loop, put the comment inside that branch or beside that
  loop.
- Explain compiler-sensitive structure precisely. Name the source transformation that causes the
  regression. Name the affected compiler stage or configuration. State the property of the generated
  code that changes.
- Do not cite a particular machine or transient timing result for a durable source or generated-code
  fact. Link the reproducer, issue, or retained evidence from generated code.
- If the compiler mechanism is known, explain it briefly. If it is not known, state only the
  comparison result. Do not guess.
- Make comments refer to the relevant accessor, helper, or invariant owner. Do not refer to a type
  or internal representation that the reader cannot see in the current scope.
- Use concrete branch names with the `ct/` prefix. Do not write phrases such as "this arm" or "the
  all-per-row case". Name the actual condition.
- If a debug session or review suggests hidden intent, get support from a reproduced result,
  repository history, or the author.
- Without this support, record only the observed constraint.
- Keep comments synchronized with the code.
- After a related change, examine each correctness argument again.
- Write all comments and Markdown bullets as full sentences with periods.
- Wrap comments at 100 columns and fill each line. Do not wrap at an arbitrary 80 columns.
- Put comments on their own lines. If a comment after code is clearer, use that placement.
- Do not use em dashes.
- Inside a function, use comments as paragraph headers.
- Put the comment first. Put the applicable lines after it. Then add a blank line.
- Tag each TODO as `TODO(connor)`.
- If a TODO belongs to a larger effort, use `TODO(connor)[Feature]`.
- State the unfinished work, the reason for the delay, and a useful next step or issue link.
- Justify a deliberate use of worse asymptotic performance beside the applicable code.
- For example, explain a linear scan that avoids an allocation.

## Whitespace

Use generous, fine-grained whitespace. Group related code, not only blocks.

- Keep struct fields and enum variants adjacent by default.
- When only a small set has documentation, keep all fields or variants adjacent.
- Reserve blank lines for structs and enums with at least five documented fields or variants.
- Require documentation on most or all fields or variants in the definition.
- Use blank lines only to make the documentation easier to read.
- When a group contains more than two documented methods, add blank lines between them.
- Inside a function, use blank lines to separate related groups of statements.
- Separate setup, each change in responsibility, and the final result into clear groups.
- Add a blank line after guards and around a `match` or loop that follows setup.
- Add a blank line before the final return.
- Do not add a blank line immediately after `fn foo() {`.

## Errors

- Give each project error helper one fixed role.
- Use separate helpers for inline validation, early returns, `Option` wrapping, and states that the
  types must prevent.
- State the requirement first. Then state the actual value.
- Always put the incorrect value in a `, got {actual}` suffix.
- Make each `expect` message state the invariant and where the code established it.
- Never write "should never happen".
- Treat an error as the caller's problem. Treat a panic as the developer's problem.
- If the code is incorrect, panic. If the input is incorrect, return an error.
- Always use the project's error conventions.

## Unsafe code and performance

- Prefer safe Rust unless interoperability or measured performance requires `unsafe`.
- Keep each unsafe block as small as possible.
- Put each safety comment directly beside its unsafe operation. If you compute a value separately,
  bind it first. Make the safety proof and the unsafe computation visually distinct.
- If an abstraction does not uphold its complete safety invariant internally, do not expose it
  through a safe API.
- State the exact safety invariant beside the unsafe operation.
- Use `// SAFETY: ` for unsafe blocks. Use `# Safety` for unsafe functions.
- Point to the code that establishes each part of the invariant.
- Document the caller's duties for each unsafe escape hatch.
- State whether a contract violation causes undefined behavior or an incorrect result.
- Enforce evidence for unsafe code with types and ownership, not documentation alone.
- Bind each proof token to the exact value, slot, allocation, or borrow that it proves.
- This rule includes proof tokens from callbacks and trait methods.
- Do not let safe code forge or duplicate the token.
- Do not let a caller get the token through an unrelated value.
- Audit each safe constructor and conversion that can produce a proof token.
- A sealed trait alone does not make its proof tokens safe.
- If safe Rust cannot enforce the relationship, make the applicable constructor, callback contract,
  or trait method unsafe.
- Document the caller's duties for each unsafe item.
- If the caller must establish the safety contract, make the helper `unsafe fn`. If the helper
  establishes and checks the complete contract, keep it safe.
- Before you extract an unsafe helper, compare its contract with the inline contract. If extraction
  separates the proof from the values that establish it, keep the operation inline.
- If a named boundary clarifies a repeated contract without broadening the unsafe surface, extract
  the operation.
- Use tests to show the intended use. Never use tests to establish the soundness of a safe
  abstraction.
- Measure before you accept complexity for performance.
- For a hot-path refactor, preserve the source shapes that the generated-code evidence requires. A
  rename or binding reorder is usually harmless. A module move, helper extraction, branch move, or
  callback change can affect inlining, loop unswitching, and vectorization.
- If a style decision depends on MIR, LLVM IR, assembly, or code-generation-unit behavior, use the
  `rust-opt` skill.
- Do not add `#[inline(always)]` without assembly or benchmark evidence that it changes the result.
- Do not run a pre-fix regression that can trigger known undefined behavior.
- Prove the defect by inspection, or test a safe part of the failure.
- Alternatively, add the regression together with the fix.

## Tests

- Name the test module `tests`, never `test`.
- Put the test module at the bottom of the file.
- Put one helper at the top that runs the operation from start to finish.
- Make each test contain only the input and expected output.
- Add `#[track_caller]` to custom assertions.
- When two cases share a body and the project uses `rstest`, use named cases.
- Align the arguments into columns.
- Do not add the dependency only to combine two small tests.
- Put data literals on separate rows. Annotate each row.
- Put `//` at the end of each row so that rustfmt cannot collapse the rows.
- Name every applicable test dimension.
- Cover the happy path, nullability, degenerate input, zero-size input, nesting, round trips, and
  documented errors.
- Compare a clever implementation with an obvious, slow reference in the test module.
- Keep shared fixtures in one `#[cfg(test)]` module. Never copy them.
- Put a session static or state static at the crate root.
- Put builders beside the code that they represent.
- Build each fixture once per test and pass it by reference.

## Put code where it is most useful

- Put a generalized item where the rest of the codebase can access it.
- Good locations include its type, a shared helper module, or the crate that owns the concept.
- Do not leave a generalized item beside its first caller.
- Move a required general helper or increase its visibility in the same change.
- Do not add a parallel local version.
- Do not increase the scope for possible future reuse.
- Extend the current abstraction. Do not add a parallel local abstraction.

## Refactoring instincts

- Move an unrelated type out of an overloaded file.
- Replace a nested match with guards.
- Move a one-method extension trait into the main trait.
- Replace a bool or tuple with a named type.
- Put validation in one place. Document the requirements of each unvalidated path.
- Add documentation that changed fields and variants lack. This rule also applies to code that
  another person wrote.
- If practical, add or identify a regression that fails before a bug fix.

## Restraint

The rules above encourage more structure. The rules below limit that structure. Both sets of rules
are important.

- Do not split a function that has one caller and one sequence of work.
- Straight-line functions of 60 to 80 lines are normal. Limit functions by nesting depth, not line
  count.
- When a second caller appears, extract a function.
- Also extract a named alternative that a branch selects.
- Prefer concise documentation for functions, types, traits, fields, and variants.
- Their names, signatures, definitions, and nearby implementations already provide much of the
  context.
- Use item documentation for information that the code does not express. This information includes
  purpose, contracts, invariants, and non-obvious behavior.
- Add all details that the contract requires. Never omit useful information only to keep item
  documentation short.
- Module documentation cannot depend on nearby implementation for context.
- Start module documentation with its one-sentence summary. Then provide the required independent
  orientation.
- Base documentation length on the required contract and context, not the perceived importance of
  the item.
- Do not add an `# Errors` section to an infallible item.
- Never state a contract twice.
- When two entry points share requirements, make one entry point own the list. Make the other entry
  point link to it.
- Duplicate documentation becomes inconsistent over time.
- Delete documentation that only repeats the signature. Improve the name instead.
- Do not target a comment-to-code ratio.
- If the code has an important information gap, add a comment. Remove stale comments.
- Create an abstraction after a second real caller appears. Do not predict a future caller.
- Do not add a trait that has one implementation.
- Do not add an extension trait that has one method.
- Avoid builders for two-field structures and wrappers that only forward calls.
- When duplication appears, introduce an abstraction.
- Treat deletion as a real change.
- Put the removal of an unnecessary layer, stale TODO, or unused wrapper in a separate commit.
- Prefer a design with fewer parts to a design with a more attractive structure.
- Do not add a type, file, and indirection only to save four lines.
