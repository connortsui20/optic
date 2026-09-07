//! Computes keys for compiler recipes and explicit requests.
//!
//! Each field has a length prefix, so field boundaries remain part of the SHA-256 input.

use std::path::Path;

use cargo_metadata::Metadata;
use cargo_metadata::PackageId;
use cargo_metadata::Target;
use optic_records::BuildRecord;
use optic_records::CaptureKey;
use sha2::Digest;
use sha2::Sha256;

use crate::Error;
use crate::error::invalid_environment;

/// Identifies source/LLVM evidence and the explicit-request key layout.
const REQUEST_REVISION: &[u8] = b"optic-request-2:source-llvm-1";

pub(crate) fn request_key(
    build: &BuildRecord,
    package: &PackageId,
    target: &Target,
    metadata: &Metadata,
    cargo_version: &[u8],
    driver_key: &str,
    cargo_home: &Path,
) -> Result<CaptureKey, Error> {
    let request_inputs = serde_json::to_vec(&(
        build,
        package,
        target,
        &metadata.target_directory,
        metadata
            .build_directory
            .as_ref()
            .unwrap_or(&metadata.target_directory),
    ))
    .map_err(|error| {
        invalid_environment(format!(
            "request identity requires serializable Cargo paths, got {error}"
        ))
    })?;

    // The invocation directory fixes ancestor Cargo configuration lookup. Cargo home fixes the
    // remaining configuration location. Cargo itself tracks the contents at those locations.
    digest(&[
        REQUEST_REVISION,
        metadata.workspace_root.as_str().as_bytes(),
        cargo_home.as_os_str().as_encoded_bytes(),
        cargo_version,
        driver_key.as_bytes(),
        &request_inputs,
    ])
    .parse()
    .map_err(Error::from)
}

pub(crate) fn digest(fields: &[&[u8]]) -> String {
    let mut digest = Sha256::new();

    for field in fields {
        digest.update((field.len() as u64).to_le_bytes());
        digest.update(field);
    }

    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use optic_records::CargoTargetKind;
    use optic_records::TargetRecord;

    use super::*;
    use crate::BuildRequest;
    use crate::CargoTarget;
    use crate::build::cargo_arguments;

    #[track_caller]
    fn fixture_build(features: Vec<String>) -> BuildRecord {
        let request = BuildRequest::new("fixture", CargoTarget::Library, "custom")
            .unwrap()
            .with_features(features)
            .unwrap();

        BuildRecord::new(
            "fixture",
            "0.1.0",
            TargetRecord::new("fixture", CargoTargetKind::Lib).unwrap(),
            "custom",
            "/toolchain/bin/cargo".into(),
            "/workspace/member".into(),
            cargo_arguments(&request),
        )
        .unwrap()
    }

    #[track_caller]
    fn fixture_key(
        artifact: &cargo_metadata::Artifact,
        build: &BuildRecord,
        metadata: &Metadata,
        cargo_version: &[u8],
        driver_key: &str,
        home: &Path,
    ) -> CaptureKey {
        request_key(
            build,
            &artifact.package_id,
            &artifact.target,
            metadata,
            cargo_version,
            driver_key,
            home,
        )
        .unwrap()
    }

    #[test]
    fn field_boundaries_are_part_of_the_digest() {
        assert_ne!(digest(&[b"a", b"bc"]), digest(&[b"ab", b"c"]));
        assert_ne!(digest(&[b"a", b""]), digest(&[b"a"]));
    }

    #[test]
    fn request_identity_normalizes_features_and_tracks_configuration_locations() {
        let artifact = crate::tests::artifact();
        let metadata: Metadata = serde_json::from_value(serde_json::json!({
            "packages": [],
            "workspace_members": [],
            "workspace_default_members": [],
            "resolve": null,
            "workspace_root": "/workspace",
            "target_directory": "/target",
            "build_directory": "/build",
            "metadata": null,
            "version": 1
        }))
        .unwrap();

        let first = fixture_build(vec!["beta,alpha".to_owned(), "beta".to_owned()]);
        let equivalent = fixture_build(vec!["alpha beta".to_owned()]);
        let original = fixture_key(
            &artifact,
            &first,
            &metadata,
            b"Cargo 1",
            "driver 1",
            Path::new("/cargo-home"),
        );

        assert_eq!(
            original,
            fixture_key(
                &artifact,
                &equivalent,
                &metadata,
                b"Cargo 1",
                "driver 1",
                Path::new("/cargo-home")
            )
        );

        let changed = fixture_build(vec!["alpha".to_owned()]);

        assert_ne!(
            original,
            fixture_key(
                &artifact,
                &changed,
                &metadata,
                b"Cargo 1",
                "driver 1",
                Path::new("/cargo-home")
            )
        );
        assert_ne!(
            original,
            fixture_key(
                &artifact,
                &first,
                &metadata,
                b"Cargo 2",
                "driver 1",
                Path::new("/cargo-home")
            )
        );
        assert_ne!(
            original,
            fixture_key(
                &artifact,
                &first,
                &metadata,
                b"Cargo 1",
                "driver 2",
                Path::new("/cargo-home")
            )
        );
        assert_ne!(
            original,
            fixture_key(
                &artifact,
                &first,
                &metadata,
                b"Cargo 1",
                "driver 1",
                Path::new("/other-home")
            )
        );

        for field in [
            "workspace_root",   // Change the Cargo workspace identity.
            "target_directory", // Change the artifact output location.
            "build_directory",  // Change the intermediate build location.
        ] {
            let mut changed = serde_json::to_value(&metadata).unwrap();
            changed[field] = "/other-location".into();
            let changed = serde_json::from_value(changed).unwrap();

            assert_ne!(
                original,
                fixture_key(
                    &artifact,
                    &first,
                    &changed,
                    b"Cargo 1",
                    "driver 1",
                    Path::new("/cargo-home")
                )
            );
        }

        let mut changed = serde_json::to_value(&first).unwrap();
        changed["invocation_directory"] = "/other-directory".into();
        let changed = serde_json::from_value(changed).unwrap();

        assert_ne!(
            original,
            fixture_key(
                &artifact,
                &changed,
                &metadata,
                b"Cargo 1",
                "driver 1",
                Path::new("/cargo-home")
            )
        );
    }
}
