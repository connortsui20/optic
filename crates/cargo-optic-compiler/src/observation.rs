//! Classifies Cargo's selected-target artifacts and completion messages.
//!
//! Positive freshness requires one selected artifact, successful completion, and agreement with
//! the stored normalized observation. Cargo's actual profile fields remain authoritative.

use cargo_metadata::Artifact;
use cargo_metadata::PackageId;
use cargo_metadata::Target;
use optic_records::CargoArtifactRecord;

use crate::Error;
use crate::error::invalid_environment;

#[derive(Default)]
pub(crate) struct CargoObservation {
    /// Artifacts with the selected package, target name, and target kinds.
    pub(crate) selected: Vec<Artifact>,
    /// Cargo's final structured completion statuses, which must contain exactly one value.
    pub(crate) finished: Vec<bool>,
    /// Whether any structured compiler diagnostic reports an error.
    pub(crate) has_errors: bool,
}

impl CargoObservation {
    pub(crate) fn record(&mut self, artifact: Artifact, package: &PackageId, target: &Target) {
        let mut kinds = artifact.target.kind.clone();
        kinds.sort();
        let mut expected_kinds = target.kind.clone();
        expected_kinds.sort();
        if artifact.package_id == *package
            && artifact.target.name == target.name
            && kinds == expected_kinds
        {
            self.selected.push(artifact);
        }
    }

    /// Checks completion and prepared target identity before normalizing the selected observation.
    pub(crate) fn completed(
        &self,
        target: &Target,
        fresh: bool,
    ) -> Result<CargoArtifactRecord, Error> {
        if self.finished != [true] || self.has_errors {
            return Err(invalid_environment(
                "successful Cargo completion requires one successful build-finished message and no compiler errors",
            ));
        }
        let [artifact] = self.selected.as_slice() else {
            return Err(invalid_environment(format!(
                "Cargo requires one matching selected artifact, got {}",
                self.selected.len()
            )));
        };
        if artifact.fresh != fresh {
            return Err(invalid_environment(format!(
                "selected artifact freshness must be {fresh}, got {}",
                artifact.fresh
            )));
        }
        let record = CargoArtifactRecord::new(artifact.clone())?;
        let mut expected_target = target.clone();
        expected_target.kind.sort();
        expected_target.crate_types.sort();
        expected_target.required_features.sort();
        if record.artifact().target != expected_target {
            return Err(invalid_environment(
                "selected artifact must match the prepared Cargo target, got a conflicting target",
            ));
        }

        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::CargoObservation;
    use crate::tests::artifact;

    #[test]
    fn requires_one_affirmative_fresh_artifact_and_success() {
        let artifact = artifact();
        let cases = [
            (vec![artifact.clone()], vec![true], false, true), // Complete positive observation.
            (Vec::new(), vec![true], false, false),            // Success without an artifact.
            (
                vec![artifact.clone(), artifact.clone()],
                vec![true],
                false,
                false,
            ), // Conflicting duplicate artifacts.
            (vec![artifact.clone()], Vec::new(), false, false), // Missing completion message.
            (vec![artifact.clone()], vec![false], false, false), // Reported failure.
            (vec![artifact.clone()], vec![true, true], false, false), // Duplicate completion.
            (vec![artifact.clone()], vec![true], true, false), // Contradictory compiler error.
        ];

        for (selected, finished, has_errors, valid) in cases {
            let observation = CargoObservation {
                selected,
                finished,
                has_errors,
            };
            assert_eq!(observation.completed(&artifact.target, true).is_ok(), valid);
        }
    }

    #[test]
    fn rejects_conflicting_target_or_freshness() {
        let artifact = artifact();
        let observation = CargoObservation {
            selected: vec![artifact.clone()],
            finished: vec![true],
            has_errors: false,
        };
        assert!(observation.completed(&artifact.target, false).is_err());
        let mut target = artifact.target;
        target.src_path = "/other/lib.rs".into();
        assert!(observation.completed(&target, true).is_err());
    }

    #[test]
    fn ignores_dependency_artifacts_and_retains_cargo_profile_fields() {
        let artifact = artifact();
        let mut dependency = artifact.clone();
        dependency.package_id.repr = "dependency".to_owned();
        let mut observation = CargoObservation {
            finished: vec![true],
            ..Default::default()
        };
        observation.record(dependency, &artifact.package_id, &artifact.target);
        observation.record(artifact.clone(), &artifact.package_id, &artifact.target);
        let normalized = observation.completed(&artifact.target, true).unwrap();

        assert_eq!(normalized.artifact().profile, artifact.profile);
        assert_eq!(normalized.artifact().features, ["alpha", "beta"]);
        assert!(!normalized.artifact().fresh);
    }
}
