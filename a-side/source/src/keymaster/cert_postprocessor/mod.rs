// Copyright 2024, The Android Open Source Project
// SPDX-License-Identifier: Apache-2.0
//! Optional Android RKP certificate postprocessing. No KeyMint backend selection.
use crate::android::hardware::security::keymint::Certificate::Certificate;
use crate::android::security::postprocessor::{
    CertificateChain::CertificateChain,
    IKeystoreCertificatePostProcessor::IKeystoreCertificatePostProcessor,
};
use crate::keymaster::utils::get_interface_once;
use anyhow::{anyhow, Context, Result};
use rsbinder::{StatusCode, Strong};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

mod chain;

const SERVICE: &str = "rkp_cert_processor.service";
const ENABLE_PROPERTY: &str = "remote_provisioning.use_cert_processor";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
const LOOKUP_INTERVAL: Duration = Duration::from_millis(100);
static LOOKUP_TIMED_OUT: AtomicBool = AtomicBool::new(false);

fn enabled(value: &str) -> bool {
    matches!(value, "1" | "true" | "y" | "yes" | "on")
}

fn wait_for_processor() -> Result<Strong<dyn IKeystoreCertificatePostProcessor>> {
    if LOOKUP_TIMED_OUT.load(Ordering::Relaxed) {
        return Err(anyhow!(
            "certificate postprocessor lookup previously timed out"
        ));
    }
    let started = Instant::now();
    loop {
        match get_interface_once(SERVICE) {
            Ok(service) => return Ok(service),
            Err(StatusCode::NameNotFound) => {}
            Err(error) => return Err(error).context("connect certificate postprocessor"),
        }
        let Some(remaining) = LOOKUP_TIMEOUT.checked_sub(started.elapsed()) else {
            LOOKUP_TIMED_OUT.store(true, Ordering::Relaxed);
            return Err(anyhow!(
                "certificate postprocessor unavailable after five seconds"
            ));
        };
        std::thread::sleep(remaining.min(LOOKUP_INTERVAL));
    }
}

/// Preserve Android's optional postprocessor contract, including fallback to the
/// complete original chain on service lookup or processing failure. Only the
/// native RKP path calls this; remote B certificates are not rewritten here.
pub(super) fn process_certificate_chain(
    certificates: Vec<Certificate>,
    attestation_certs: Vec<u8>,
) -> Vec<Certificate> {
    let use_processor =
        rsproperties::get::<String>(ENABLE_PROPERTY).is_ok_and(|value| enabled(&value));
    chain::process_chain_with(use_processor, certificates, attestation_certs, |request| {
        let result = wait_for_processor().and_then(|service| {
            service
                .processKeystoreCertificates(&request)
                .context("postprocess RKP certificates")
        });
        if let Err(error) = &result {
            log::warn!("certificate postprocessor failed; preserving original chain: {error:#}");
        }
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn property_boolean_matches_android_true_values() {
        for value in ["1", "true", "y", "yes", "on"] {
            assert!(enabled(value));
        }
        for value in ["", "0", "false", "n", "no", "off", "invalid"] {
            assert!(!enabled(value));
        }
    }
}
