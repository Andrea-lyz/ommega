use super::*;

fn base_config() -> FilterConfig {
    FilterConfig::default()
}

fn base_scope() -> Vec<String> {
    vec!["com.allowed".to_string()]
}

#[test]
fn disabled_filter_reports_disabled_and_allows() {
    let mut config = base_config();
    config.enabled = false;

    let decision = evaluate(&base_scope(), &config, 10_000, PackageResolution::Unknown);
    assert!(decision.allowed);
    assert_eq!(decision.reason, FilterReason::Disabled);
}

#[test]
fn scoped_system_app_requires_uid_sid_process_and_explicit_binding() {
    let package = "com.example.engineer";
    let sid = "u:r:factory_app:s0";
    let scope = vec![package.to_owned()];
    let mut config = base_config();
    assert!(evaluate_system_app(&scope, &config, 1000, sid, package).is_none());
    config
        .system_app_contexts
        .insert(package.into(), sid.into());

    for uid in [1000, 101_000] {
        let decision = evaluate_system_app(&scope, &config, uid, sid, package).unwrap();
        assert!(decision.allowed);
        assert_eq!(decision.packages, scope);
        assert_eq!(decision.reason, FilterReason::Allowed);
    }
    for uid in [0, 1001, 1017, 2000, 10_000, 110_000] {
        assert!(evaluate_system_app(&scope, &config, uid, sid, package).is_none());
    }
    for (wrong_sid, wrong_process) in [
        ("", package),
        ("u:r:system_server:s0", package),
        ("u:r:system_app:s0", package),
        (sid, "system_server"),
        (sid, "com.example.other"),
        (sid, "com.example.engineer:remote"),
        (sid, "/system/bin/com.example.engineer"),
    ] {
        assert!(evaluate_system_app(&scope, &config, 1000, wrong_sid, wrong_process).is_none());
    }

    // A caller-supplied configuration cannot opt native core domains in.
    config
        .system_app_contexts
        .insert(package.into(), "u:r:system_server:s0".into());
    assert!(evaluate_system_app(&scope, &config, 1000, "u:r:system_server:s0", package).is_none());
}

#[test]
fn scoped_system_app_keeps_scope_denylist_and_default_uid_policy() {
    let package = "com.example.engineer";
    let sid = "u:r:factory_app:s0";
    let scope = vec![package.to_owned()];
    let mut config = base_config();
    config
        .system_app_contexts
        .insert(package.into(), sid.into());

    let decision = evaluate_system_app(&[], &config, 1000, sid, package).unwrap();
    assert!(!decision.allowed);
    assert_eq!(decision.reason, FilterReason::RejectedNotInScope);

    config.deny_packages.push(package.into());
    let decision = evaluate_system_app(&scope, &config, 1000, sid, package).unwrap();
    assert!(!decision.allowed);
    assert_eq!(decision.reason, FilterReason::RejectedByDenylist);

    // Adding a binding never changes UID-wide filtering or unknown callers.
    let decision = evaluate(&scope, &config, 1000, PackageResolution::Unknown);
    assert!(!decision.allowed);
    assert_eq!(decision.reason, FilterReason::RejectedAndroidPackage);
}

#[test]
fn global_scope_ignores_lists_and_includes_system_and_unresolved_callers() {
    let mut config = base_config();
    config.enabled = false;
    config.deny_packages = vec!["android".into(), "com.example.denied".into()];
    for uid in [0, 1000, 1017, 2000, 9999, 10_000, 101_000, 110_000] {
        for resolution in [
            PackageResolution::Unknown,
            PackageResolution::Known(vec!["android".into()]),
            PackageResolution::Known(vec!["com.example.denied".into()]),
        ] {
            let decision = evaluate(&[], &config, uid, resolution);
            assert!(decision.allowed, "global scope rejected uid {uid}");
            assert_eq!(decision.reason, FilterReason::Disabled);
        }
    }
    config.enabled = true;
    assert_eq!(
        evaluate(&[], &config, 1000, PackageResolution::Unknown).reason,
        FilterReason::RejectedAndroidPackage,
    );
    assert_eq!(
        evaluate(&[], &config, 10_000, PackageResolution::Unknown).reason,
        FilterReason::RejectedUnknownPackage,
    );
}

#[test]
fn android_package_is_rejected_when_blocking_is_enabled() {
    let config = base_config();
    let scope = vec!["android".to_string()];

    let decision = evaluate(
        &scope,
        &config,
        10_000,
        PackageResolution::Known(vec!["android".to_string()]),
    );
    assert!(!decision.allowed);
    assert_eq!(decision.reason, FilterReason::RejectedAndroidPackage);
}

#[test]
fn denylist_rejection_takes_precedence_over_scope() {
    let mut config = base_config();
    config.block_android_package = false;
    config.deny_packages = vec!["com.example.app".to_string()];
    let scope = vec!["com.example.app".to_string()];

    let decision = evaluate(
        &scope,
        &config,
        10_000,
        PackageResolution::Known(vec!["com.example.app".to_string()]),
    );
    assert!(!decision.allowed);
    assert_eq!(decision.reason, FilterReason::RejectedByDenylist);
}

#[test]
fn known_package_outside_scope_is_rejected() {
    let mut config = base_config();
    config.block_android_package = false;

    let decision = evaluate(
        &base_scope(),
        &config,
        10_000,
        PackageResolution::Known(vec!["com.other".to_string()]),
    );
    assert!(!decision.allowed);
    assert_eq!(decision.reason, FilterReason::RejectedNotInScope);
}

#[test]
fn unknown_package_policy_only_allows_app_uids() {
    let mut config = base_config();
    config.allow_unknown_package = true;

    for uid in [10_000, 110_000] {
        let decision = evaluate(&base_scope(), &config, uid, PackageResolution::Unknown);
        assert!(decision.allowed);
        assert_eq!(decision.reason, FilterReason::Allowed);
    }
    for uid in [9_999, 109_999] {
        let decision = evaluate(&base_scope(), &config, uid, PackageResolution::Unknown);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, FilterReason::RejectedAndroidPackage);
    }
}

#[test]
fn root_follows_android_package_policy() {
    let mut config = base_config();
    config.allow_unknown_package = true;

    let decision = evaluate(&base_scope(), &config, 0, PackageResolution::Unknown);
    assert!(!decision.allowed);
    assert_eq!(decision.reason, FilterReason::RejectedAndroidPackage);

    config.block_android_package = false;
    let decision = evaluate(&base_scope(), &config, 0, PackageResolution::Unknown);
    assert!(decision.allowed);
    assert_eq!(decision.reason, FilterReason::Allowed);
}

#[test]
fn android_prefixed_package_in_scope_is_allowed() {
    let config = base_config();
    let scope = vec!["com.android.vending".to_string()];
    let decision = evaluate(
        &scope,
        &config,
        10_000,
        PackageResolution::Known(scope.clone()),
    );
    assert!(decision.allowed);
    assert_eq!(decision.reason, FilterReason::Allowed);
}

#[test]
fn empty_scope_keeps_android_and_denylist_precedence() {
    let mut config = base_config();
    config.block_android_package = false;
    let decision = evaluate(
        &[],
        &config,
        10_000,
        PackageResolution::Known(vec!["com.anything".to_string()]),
    );
    assert!(!decision.allowed);
    assert_eq!(decision.reason, FilterReason::RejectedNotInScope);

    let config = base_config();
    let decision = evaluate(
        &[],
        &config,
        10_000,
        PackageResolution::Known(vec!["android".to_string()]),
    );
    assert_eq!(decision.reason, FilterReason::RejectedAndroidPackage);

    let mut config = base_config();
    config.block_android_package = false;
    config.deny_packages = vec!["com.blocked".to_string()];
    let decision = evaluate(
        &[],
        &config,
        10_000,
        PackageResolution::Known(vec!["com.blocked".to_string()]),
    );
    assert_eq!(decision.reason, FilterReason::RejectedByDenylist);
}
