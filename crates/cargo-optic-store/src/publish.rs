//! Installs the new candidate immediately before the atomic capture commit.
//!
//! A failure before pointer replacement preserves the old candidate. A failure after replacement
//! leaves a dangling pointer, which is a miss. No required fallible operation follows capture commit.

use std::fs;

use optic_records::CaptureRecord;
use optic_records::InstanceManifest;
use snafu::ResultExt;

use crate::CAPTURE_FILE_NAME;
use crate::Error;
use crate::INSTANCES_FILE_NAME;
use crate::MAX_HEADER_BYTES;
use crate::MAX_INSTANCES_BYTES;
use crate::Store;
use crate::candidate::CandidatePointer;
use crate::error::CaptureExistsSnafu;
use crate::error::FilesystemSnafu;
use crate::error::MismatchedPublishedCaptureIdSnafu;
use crate::record_io::write_record;

impl Store {
    /// Atomically publishes one immutable capture and its instance evidence.
    ///
    /// The new candidate pointer names the future capture immediately before its final rename.
    /// No fallible cache update follows that rename. The caller must serialize publication and
    /// prevent external store mutation throughout this operation.
    ///
    /// # Errors
    ///
    /// Returns an error for conflicting identities, oversized records, or failed publication.
    /// A failure publishes no new capture. A failure after pointer replacement invalidates reuse
    /// of the previous candidate. Old completed captures remain explicitly readable.
    pub fn publish(
        &self,
        capture: &CaptureRecord,
        instances: &InstanceManifest,
    ) -> Result<(), Error> {
        if capture.id() != instances.capture_id() {
            return MismatchedPublishedCaptureIdSnafu {
                capture_id: capture.id().clone(),
                manifest_id: instances.capture_id().clone(),
            }
            .fail();
        }

        self.initialize()?;
        let staging = self.root.join("staging").join(capture.id().as_str());
        let completed = self.root.join("captures").join(capture.id().as_str());
        match fs::symlink_metadata(&completed) {
            Ok(_) => {
                return CaptureExistsSnafu {
                    id: capture.id().clone(),
                }
                .fail();
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Error::Filesystem {
                    operation: "read metadata for",
                    path: completed,
                    source,
                });
            }
        }

        fs::create_dir(&staging).with_context(|_| FilesystemSnafu {
            operation: "create",
            path: staging.clone(),
        })?;

        #[cfg(test)]
        self.interrupt_publication(PublicationBoundary::CaptureWrite)?;
        write_record(&staging.join(CAPTURE_FILE_NAME), capture, MAX_HEADER_BYTES)?;
        #[cfg(test)]
        self.interrupt_publication(PublicationBoundary::InstancesWrite)?;
        write_record(
            &staging.join(INSTANCES_FILE_NAME),
            instances,
            MAX_INSTANCES_BYTES,
        )?;

        let staged_pointer = staging.join("candidate.json");
        #[cfg(test)]
        self.interrupt_publication(PublicationBoundary::PointerWrite)?;
        write_record(
            &staged_pointer,
            &CandidatePointer::new(capture),
            MAX_HEADER_BYTES,
        )?;
        let pointer = self.candidate_path(capture.analysis().request_key());

        // Installing the pointer after commit adds a fallible cache update after visible capture.
        // Installing it first makes any failed commit leave a dangling candidate, which is a miss.
        #[cfg(test)]
        self.interrupt_publication(PublicationBoundary::PointerReplace)?;
        fs::rename(&staged_pointer, &pointer).with_context(|_| FilesystemSnafu {
            operation: "replace candidate",
            path: pointer,
        })?;
        #[cfg(test)]
        self.interrupt_publication(PublicationBoundary::CaptureRename)?;
        fs::rename(&staging, &completed).with_context(|_| FilesystemSnafu {
            operation: "publish",
            path: completed,
        })?;

        Ok(())
    }
}

/// Deterministic failures at boundaries that ordinary path obstruction cannot reach after setup.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PublicationBoundary {
    /// Before the staged capture header is written.
    CaptureWrite,
    /// After the header is flushed and before the instance manifest is written.
    InstancesWrite,
    /// After both records are flushed and before the candidate pointer is written.
    PointerWrite,
    /// After the staged pointer is flushed and before it replaces the old pointer.
    PointerReplace,
    /// After pointer replacement and before the capture becomes visible.
    CaptureRename,
}

#[cfg(test)]
impl Store {
    fn interrupt_publication(&self, boundary: PublicationBoundary) -> Result<(), Error> {
        if self.publication_failure == Some(boundary) {
            return Err(Error::Filesystem {
                operation: "publish (test interruption)",
                path: self.root.clone(),
                source: std::io::Error::other(format!("injected {boundary:?} failure")),
            });
        }

        Ok(())
    }
}
