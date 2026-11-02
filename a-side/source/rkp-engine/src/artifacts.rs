use std::{fs, path::Path};

use crate::paths::AppPaths;
use anyhow::{Result, ensure};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::{
    keybox_xml::{
        CertificateChainSummary, build_keybox_xml, summarize_chain, validate_keybox_roundtrip,
    },
    private_output::write_new_verified,
    workflow::Issued,
};

#[derive(Serialize)]
pub(crate) struct SavedKey {
    pub index: usize,
    pub keybox_path: String,
    pub path: String,
    pub device_id: String,
    pub summary: CertificateChainSummary,
}

pub(crate) struct Saved {
    pub csr_path: String,
    pub keys: Vec<SavedKey>,
}

pub(crate) fn private_run(paths: &AppPaths, prefix: &str) -> Result<tempfile::TempDir> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&paths.outputs_dir)?;
    Ok(tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(&paths.outputs_dir)?)
}

pub(crate) fn save_issued(
    paths: &AppPaths,
    issued: &Issued,
    prefix: &str,
    explicit_output: Option<&Path>,
) -> Result<Saved> {
    let directory = private_run(paths, prefix)?;
    let csr_path = directory.path().join("request.cbor");
    write_new_verified(&csr_path, &issued.csr, |bytes| {
        ensure!(
            crate::verify::verify_csr(bytes)?.signature_valid,
            "saved CSR is invalid"
        );
        Ok(())
    })?;
    let mut saved = Vec::new();
    for (index, key) in issued.keys.iter().enumerate() {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|_| anyhow::anyhow!("random device id unavailable"))?;
        let device_id = format!("rkp-{}", hex::encode(nonce));
        let xml = Zeroizing::new(build_keybox_xml(
            &key.pair.secret_key,
            &key.chain,
            &device_id,
        )?);
        let path = directory.path().join(format!("keybox_{index}.xml"));
        let verify = |bytes: &[u8]| {
            validate_keybox_roundtrip(
                std::str::from_utf8(bytes)?,
                &key.pair.secret_key,
                &key.chain,
                &device_id,
            )
        };
        write_new_verified(&path, xml.as_bytes(), verify)?;
        let output = if let Some(explicit) = explicit_output {
            ensure!(
                issued.keys.len() == 1,
                "explicit output requires exactly one key"
            );
            write_new_verified(explicit, xml.as_bytes(), verify)?;
            explicit.to_path_buf()
        } else {
            path
        };
        saved.push(SavedKey {
            index,
            keybox_path: output.display().to_string(),
            path: output.display().to_string(),
            device_id,
            summary: summarize_chain(&key.chain),
        });
    }
    let _ = directory.keep();
    Ok(Saved {
        csr_path: csr_path.display().to_string(),
        keys: saved,
    })
}
