use kmr_common::consts::{AID_APP_END, AID_APP_START, AID_SYSTEM, AID_USER_OFFSET};

use crate::config::FilterConfig;

#[derive(Debug, Clone)]
pub enum PackageResolution {
    Known(Vec<String>),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterReason {
    Disabled,
    Allowed,
    AllowedDelegatedGrant,
    // Application-global UID exclusion must not enter descriptor grant probing.
    RejectedNonApplicationUid,
    RejectedAndroidPackage,
    RejectedByDenylist,
    RejectedNotInScope,
    RejectedUnknownPackage,
}

#[derive(Debug, Clone)]
pub struct FilterDecision {
    pub allowed: bool,
    pub reason: FilterReason,
    pub packages: Vec<String>,
}

pub fn evaluate(
    scoop: &[String],
    config: &FilterConfig,
    uid: u32,
    resolution: PackageResolution,
) -> FilterDecision {
    if !config.enabled {
        // AOSP allocates regular application IDs in this range for every user.
        // Native services, SDK sandboxes and isolated processes are outside it.
        // UID-1000 applications require evaluate_system_app's identity checks.
        let allowed = (AID_APP_START..=AID_APP_END).contains(&(uid % AID_USER_OFFSET));
        return FilterDecision {
            allowed,
            reason: if allowed {
                FilterReason::Disabled
            } else {
                FilterReason::RejectedNonApplicationUid
            },
            packages: match resolution {
                PackageResolution::Known(packages) => packages,
                PackageResolution::Unknown => Vec::new(),
            },
        };
    }

    if config.block_android_package && uid % AID_USER_OFFSET < AID_APP_START {
        return FilterDecision {
            allowed: false,
            reason: FilterReason::RejectedAndroidPackage,
            packages: match resolution {
                PackageResolution::Known(packages) => packages,
                PackageResolution::Unknown => Vec::new(),
            },
        };
    }

    let packages = match resolution {
        PackageResolution::Known(packages) => packages,
        PackageResolution::Unknown => {
            let allowed = config.allow_unknown_package;
            return FilterDecision {
                allowed,
                reason: if allowed {
                    FilterReason::Allowed
                } else {
                    FilterReason::RejectedUnknownPackage
                },
                packages: Vec::new(),
            };
        }
    };

    let reason = if config.block_android_package
        && packages
            .iter()
            .any(|pkg| pkg == "android" || pkg.starts_with("android."))
    {
        FilterReason::RejectedAndroidPackage
    } else if packages
        .iter()
        .any(|pkg| config.deny_packages.contains(pkg))
    {
        FilterReason::RejectedByDenylist
    } else if !packages.iter().any(|pkg| scoop.contains(pkg)) {
        FilterReason::RejectedNotInScope
    } else {
        FilterReason::Allowed
    };

    FilterDecision {
        allowed: reason == FilterReason::Allowed,
        reason,
        packages,
    }
}

/// Resolve an explicitly configured system app without treating every UID-1000
/// package as its caller. The SID comes from Binder, not from the process name.
/// Missing identity evidence keeps the ordinary system-UID rejection in force.
pub fn evaluate_system_app(
    scoop: &[String],
    config: &FilterConfig,
    uid: u32,
    sid: &str,
    process: &str,
) -> Option<FilterDecision> {
    if uid % AID_USER_OFFSET != AID_SYSTEM {
        return None;
    }
    // Native/system-server domains cannot opt in, even under a matching name.
    let domain = sid.strip_prefix("u:r:")?.split_once(':')?.0;
    if !domain.ends_with("_app")
        || !process.contains('.')
        || process.split('.').any(|part| {
            part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        || config.system_app_contexts.get(process).map(String::as_str) != Some(sid)
    {
        return None;
    }
    if !config.enabled {
        // Application-global scope skips lists only after authenticating this
        // configured app process; the shared UID alone never opts callers in.
        return Some(FilterDecision {
            allowed: true,
            reason: FilterReason::Disabled,
            packages: vec![process.to_owned()],
        });
    }
    // Reuse scope and denylist precedence, relaxing only the system-UID gate
    // for this authenticated, explicitly named process.
    let mut app_filter = config.clone();
    app_filter.block_android_package = false;
    Some(evaluate(
        scoop,
        &app_filter,
        uid,
        PackageResolution::Known(vec![process.to_owned()]),
    ))
}

#[cfg(test)]
mod tests;
