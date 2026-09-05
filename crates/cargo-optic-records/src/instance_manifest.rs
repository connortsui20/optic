//! Scopes concrete compiler instances to one capture.
//!
//! [`InstanceManifest`] is the durable boundary between compiler collection and evidence queries.
//! Its [`CaptureId`] prevents evidence from one capture from satisfying a query
//! for another capture.

use std::collections::HashMap;
use std::collections::HashSet;

use serde::Deserialize;
use serde::Serialize;
use snafu::ensure;

use crate::ArtifactId;
use crate::ArtifactKind;
use crate::ArtifactRecord;
use crate::ByteRange;
use crate::CAPTURE_FORMAT_VERSION;
use crate::CaptureId;
use crate::Error;
use crate::InstanceRecord;
use crate::LlvmCollection;
use crate::LlvmLto;
use crate::LlvmProvenance;
use crate::LlvmStage;
use crate::SourceAvailability;
use crate::error::InvalidFieldSnafu;
use crate::error::UnsupportedFormatSnafu;

/// Every concrete compiler instance collected for one capture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawInstanceManifest")]
pub struct InstanceManifest {
    format_version: u32,
    capture_id: CaptureId,
    instances: Vec<InstanceRecord>,
    artifacts: Vec<ArtifactRecord>,
    llvm_provenance: LlvmProvenance,
    llvm: LlvmCollection,
}

impl InstanceManifest {
    /// Creates a capture-scoped instance manifest using the current durable format.
    ///
    /// An empty instance list is valid when the selected compilation emits no function placements.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate identities, invalid artifact references or kinds, ranges past
    /// declared lengths, or collected LLVM that contradicts its effective configuration or omits a
    /// recorded placement's codegen unit. Collected modules without instance placements are valid.
    pub fn new(
        capture_id: CaptureId,
        instances: Vec<InstanceRecord>,
        artifacts: Vec<ArtifactRecord>,
        llvm_provenance: LlvmProvenance,
        llvm: LlvmCollection,
    ) -> Result<Self, Error> {
        let mut artifact_table = HashMap::with_capacity(artifacts.len());
        for artifact in &artifacts {
            if artifact_table.insert(artifact.id(), artifact).is_some() {
                return InvalidFieldSnafu {
                    field: "artifact table",
                    actual: format!("duplicate artifact {}", artifact.id().value()),
                }
                .fail();
            }
        }

        let mut identities = HashSet::with_capacity(instances.len());
        for instance in &instances {
            let identity = (
                instance.definition(),
                instance.display_name(),
                instance.raw_symbol(),
            );
            if !identities.insert(identity) {
                return InvalidFieldSnafu {
                    field: "instance manifest",
                    actual: format!("a duplicate instance ({})", instance.display_name()),
                }
                .fail();
            }
            if let SourceAvailability::Available(source) = instance.source() {
                validate_artifact_range(
                    &artifact_table,
                    source.artifact(),
                    ArtifactKind::Source,
                    source.range(),
                )?;
            }
        }

        validate_llvm(&instances, &artifact_table, &llvm_provenance, &llvm)?;

        Ok(Self {
            format_version: CAPTURE_FORMAT_VERSION,
            capture_id,
            instances,
            artifacts,
            llvm_provenance,
            llvm,
        })
    }

    /// Returns the durable record format version.
    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Returns the capture that owns every instance in this manifest.
    pub fn capture_id(&self) -> &CaptureId {
        &self.capture_id
    }

    /// Returns the concrete instances in their recorded order.
    pub fn instances(&self) -> &[InstanceRecord] {
        &self.instances
    }

    /// Returns the complete table of declared source and LLVM files.
    pub fn artifacts(&self) -> &[ArtifactRecord] {
        &self.artifacts
    }

    /// Returns the actual backend and codegen configuration for this capture.
    pub fn llvm_provenance(&self) -> &LlvmProvenance {
        &self.llvm_provenance
    }

    /// Returns complete optimized modules or the durable reason for their absence.
    pub fn llvm(&self) -> &LlvmCollection {
        &self.llvm
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstanceManifest {
    format_version: u32,
    capture_id: CaptureId,
    instances: Vec<InstanceRecord>,
    artifacts: Vec<ArtifactRecord>,
    llvm_provenance: LlvmProvenance,
    llvm: LlvmCollection,
}

impl TryFrom<RawInstanceManifest> for InstanceManifest {
    type Error = Error;

    fn try_from(manifest: RawInstanceManifest) -> Result<Self, Self::Error> {
        ensure!(
            manifest.format_version == CAPTURE_FORMAT_VERSION,
            UnsupportedFormatSnafu {
                expected: CAPTURE_FORMAT_VERSION,
                actual: manifest.format_version,
            }
        );

        Self::new(
            manifest.capture_id,
            manifest.instances,
            manifest.artifacts,
            manifest.llvm_provenance,
            manifest.llvm,
        )
    }
}

fn validate_artifact_range(
    artifacts: &HashMap<ArtifactId, &ArtifactRecord>,
    id: ArtifactId,
    kind: ArtifactKind,
    range: ByteRange,
) -> Result<(), Error> {
    let artifact = artifacts.get(&id).ok_or_else(|| {
        InvalidFieldSnafu {
            field: "artifact reference",
            actual: format!("unknown artifact {}", id.value()),
        }
        .build()
    })?;
    if artifact.kind() != kind || range.end() > artifact.byte_len() {
        return InvalidFieldSnafu {
            field: "artifact range",
            actual: format!(
                "wrong kind or range past artifact {} length {}",
                id.value(),
                artifact.byte_len()
            ),
        }
        .fail();
    }

    Ok(())
}

fn validate_llvm(
    instances: &[InstanceRecord],
    artifacts: &HashMap<ArtifactId, &ArtifactRecord>,
    provenance: &LlvmProvenance,
    llvm: &LlvmCollection,
) -> Result<(), Error> {
    let collected = match llvm {
        LlvmCollection::NotCaptured(_) => &[][..],
        LlvmCollection::Collected(collected) => {
            if provenance.backend() != "llvm"
                || provenance.llvm_version().is_none()
                || provenance.incremental()
                || provenance.linker_plugin_lto()
                || !matches!(provenance.lto(), LlvmLto::Off | LlvmLto::LocalThin)
            {
                return InvalidFieldSnafu {
                    field: "LLVM collection",
                    actual: "collected modules with an unsupported configuration",
                }
                .fail();
            }

            collected.as_slice()
        }
    };

    let mut modules = HashSet::new();
    let mut module_artifacts = HashSet::new();
    for module in collected {
        if !modules.insert(module.compiler_module()) || !module_artifacts.insert(module.artifact())
        {
            return InvalidFieldSnafu {
                field: "LLVM modules",
                actual: "duplicate module identity or artifact",
            }
            .fail();
        }
        if !matches!(
            (module.stage(), provenance.lto()),
            (LlvmStage::NoLtoOptimized, LlvmLto::Off)
                | (LlvmStage::LocalThinLtoPostPassManager, LlvmLto::LocalThin)
        ) {
            return InvalidFieldSnafu {
                field: "LLVM module stage",
                actual: "a stage that disagrees with effective LTO",
            }
            .fail();
        }
        validate_artifact_range(
            artifacts,
            module.artifact(),
            ArtifactKind::Llvm,
            ByteRange::new(0, 0)?,
        )?;
        for definition in module.definitions() {
            validate_artifact_range(
                artifacts,
                module.artifact(),
                ArtifactKind::Llvm,
                definition.range(),
            )?;
        }
    }

    // A missing definition is meaningful only when collection includes every recorded codegen unit.
    // Optimization can move or remove a body, so this checks modules rather than symbol placement.
    if matches!(llvm, LlvmCollection::Collected(_)) {
        let missing = instances
            .iter()
            .flat_map(InstanceRecord::placements)
            .find(|placement| !modules.contains(placement.codegen_unit()));
        if let Some(placement) = missing {
            return InvalidFieldSnafu {
                field: "LLVM modules",
                actual: format!("missing module for placement {}", placement.codegen_unit()),
            }
            .fail();
        }
    }

    for artifact in artifacts.values() {
        if artifact.kind() == ArtifactKind::Llvm && !module_artifacts.contains(&artifact.id()) {
            return InvalidFieldSnafu {
                field: "LLVM artifact",
                actual: "an artifact without a collected module",
            }
            .fail();
        }
    }

    Ok(())
}
