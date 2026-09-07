//! Resolves exact raw symbols through stored LLVM indexes.
//!
//! Every captured module participates, even without an original compiler placement. Direct aliases
//! resolve only within their module. Queries do not parse LLVM text or infer why a symbol is
//! absent.

use std::collections::HashMap;
use std::collections::HashSet;

use optic_records::InstanceRef;
use optic_records::LlvmCollection;
use optic_records::LlvmDefinitionKind;
use optic_records::LlvmModuleRecord;
use optic_records::LlvmStage;
use optic_records::UnsupportedLlvmConfiguration;
use optic_store::Store;
use snafu::ResultExt;

use crate::Error;
use crate::EvidenceRange;
use crate::error;
use crate::referenced_instance;

/// The exact standalone LLVM bodies available for one instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LlvmEvidence {
    /// All resolved bodies, ordered by compiler module and byte range.
    Available(Vec<LlvmBody>),
    /// The selected compiler configuration did not support optimized LLVM collection.
    NotCaptured(UnsupportedLlvmConfiguration),
    /// Complete supported modules contain no exact standalone definition for the raw symbol.
    NoExactDefinition,
    /// An exact alias or ifunc was unsupported, and no module supplied a resolved body.
    UnsupportedAlias,
}

/// One exact function body and the direct aliases that selected it.
///
/// The excerpt can depend on types, attributes, and metadata elsewhere in the captured module.
/// It is not necessarily an independently assemblable LLVM module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlvmBody {
    evidence: EvidenceRange,
    compiler_module: String,
    stage: LlvmStage,
    raw_symbol: String,
    aliases: Vec<String>,
}

impl LlvmBody {
    /// Returns the function's byte range in the completed capture.
    pub fn evidence(&self) -> &EvidenceRange {
        &self.evidence
    }

    /// Returns the compiler identity of the module that contains the function.
    pub fn compiler_module(&self) -> &str {
        &self.compiler_module
    }

    /// Returns the optimization stage captured for this module.
    pub fn stage(&self) -> LlvmStage {
        self.stage
    }

    /// Returns the exact decoded symbol of the final function definition.
    pub fn raw_symbol(&self) -> &str {
        &self.raw_symbol
    }

    /// Returns direct-alias symbols in resolution order, excluding the final function symbol.
    ///
    /// The list is empty when the instance symbol names the function directly.
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }
}

/// Selects every exact LLVM body for an immutable instance reference.
///
/// Matching uses only the stored decoded raw symbol. A valid body takes precedence over an
/// unsupported alias in another module. Missing definitions do not establish that optimization
/// removed the function.
///
/// # Errors
///
/// Returns the capture and reference errors described by [`crate::source_evidence`]. A direct-alias
/// cycle also returns an error, even when another module contains a valid body.
pub fn llvm_evidence(store: &Store, reference: &InstanceRef) -> Result<LlvmEvidence, Error> {
    let manifest = store
        .read_instances(reference.capture_id())
        .context(error::StoreSnafu)?;
    let instance = referenced_instance(&manifest, reference)?;

    let modules = match manifest.llvm() {
        LlvmCollection::Collected(modules) => modules,
        LlvmCollection::NotCaptured(reason) => return Ok(LlvmEvidence::NotCaptured(*reason)),
    };

    let mut bodies = Vec::new();
    let mut unsupported_alias = false;

    for module in modules {
        match resolve_module(module, instance.raw_symbol(), reference)? {
            ModuleEvidence::Body(body) => bodies.push(body),
            ModuleEvidence::NoExactDefinition => {}
            ModuleEvidence::UnsupportedAlias => unsupported_alias = true,
        }
    }

    if bodies.is_empty() {
        return Ok(if unsupported_alias {
            LlvmEvidence::UnsupportedAlias
        } else {
            LlvmEvidence::NoExactDefinition
        });
    }

    bodies.sort_by(|left, right| {
        left.compiler_module()
            .cmp(right.compiler_module())
            .then_with(|| {
                left.evidence()
                    .range()
                    .start()
                    .cmp(&right.evidence().range().start())
            })
            .then_with(|| {
                left.evidence()
                    .range()
                    .length()
                    .cmp(&right.evidence().range().length())
            })
    });

    Ok(LlvmEvidence::Available(bodies))
}

enum ModuleEvidence {
    Body(LlvmBody),
    NoExactDefinition,
    UnsupportedAlias,
}

fn resolve_module(
    module: &LlvmModuleRecord,
    raw_symbol: &str,
    reference: &InstanceRef,
) -> Result<ModuleEvidence, Error> {
    let definitions = module
        .definitions()
        .iter()
        .map(|definition| (definition.raw_symbol(), definition))
        .collect::<HashMap<_, _>>();
    let mut visited = HashSet::new();
    let mut aliases = Vec::new();
    let mut symbol = raw_symbol;

    loop {
        if !visited.insert(symbol) {
            return error::AliasCycleSnafu {
                reference: reference.clone(),
                compiler_module: module.compiler_module().to_owned(),
                raw_symbol: symbol.to_owned(),
            }
            .fail();
        }

        let Some(definition) = definitions.get(symbol) else {
            return Ok(ModuleEvidence::NoExactDefinition);
        };

        match definition.kind() {
            LlvmDefinitionKind::Function => {
                return Ok(ModuleEvidence::Body(LlvmBody {
                    evidence: EvidenceRange::new(
                        reference.capture_id().clone(),
                        module.artifact(),
                        definition.range(),
                    ),
                    compiler_module: module.compiler_module().to_owned(),
                    stage: module.stage(),
                    raw_symbol: definition.raw_symbol().to_owned(),
                    aliases,
                }));
            }
            LlvmDefinitionKind::DirectAlias { target } => {
                aliases.push(symbol.to_owned());
                symbol = target;
            }
            LlvmDefinitionKind::ExpressionAlias | LlvmDefinitionKind::Ifunc => {
                return Ok(ModuleEvidence::UnsupportedAlias);
            }
        }
    }
}
