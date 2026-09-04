//! Scopes concrete compiler instances to one capture.
//!
//! [`InstanceManifest`] is the durable boundary between compiler collection and evidence queries.
//! Its [`CaptureId`](crate::CaptureId) prevents evidence from one capture from satisfying a query
//! for another capture.

use std::collections::HashSet;

use serde::Deserialize;
use serde::Serialize;
use snafu::ensure;

use crate::CAPTURE_FORMAT_VERSION;
use crate::CaptureId;
use crate::Error;
use crate::InstanceRecord;
use crate::error::InvalidFieldSnafu;
use crate::error::UnsupportedFormatSnafu;

/// Every concrete compiler instance collected for one capture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawInstanceManifest")]
pub struct InstanceManifest {
    format_version: u32,
    capture_id: CaptureId,
    instances: Vec<InstanceRecord>,
}

impl InstanceManifest {
    /// Creates a capture-scoped instance manifest using the current durable format.
    ///
    /// An empty instance list is valid when the selected compilation emits no function placements.
    ///
    /// # Errors
    ///
    /// Returns an error if the same definition, display name, and raw symbol occur more than once.
    pub fn new(capture_id: CaptureId, instances: Vec<InstanceRecord>) -> Result<Self, Error> {
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
        }

        Ok(Self {
            format_version: CAPTURE_FORMAT_VERSION,
            capture_id,
            instances,
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

    /// Returns the number of concrete instances in this manifest.
    pub fn instance_count(&self) -> u64 {
        u64::try_from(self.instances.len())
            .expect("a manifest Vec length must fit in u64 on supported Rust targets")
    }

    /// Returns the total number of codegen-unit placements in this manifest.
    pub fn placement_count(&self) -> u64 {
        self.instances.iter().fold(0, |count, instance| {
            let placements = u64::try_from(instance.placements().len())
                .expect("an instance Vec length must fit in u64 on supported Rust targets");

            count
                .checked_add(placements)
                .expect("manifest placements must fit in u64 within an address space")
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstanceManifest {
    format_version: u32,
    capture_id: CaptureId,
    instances: Vec<InstanceRecord>,
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

        Self::new(manifest.capture_id, manifest.instances)
    }
}
