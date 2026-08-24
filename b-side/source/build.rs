use std::{fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=aidl");
    println!("cargo:rerun-if-changed=build.rs");

    // The relay / real-TEE forwarding path only needs the keymint HAL types
    // (plus the secureclock *data* types they reference: Timestamp and
    // TimeStampToken). ISecureClock is an async service interface we don't
    // use, and generating it would pull in dyn-compatibility problems on
    // nightly, so it is deliberately not sourced.
    let mut aidl = rsbinder_aidl::Builder::new()
        .include_dir(PathBuf::from("aidl/android/hardware/security/keymint"))
        .include_dir(PathBuf::from("aidl/android/hardware/security/secureclock"))
        .output(PathBuf::from("aidl.rs"));

    let keymint_dir = "aidl/android/hardware/security/keymint";
    for entry in fs::read_dir(keymint_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) == Some("aidl") {
            aidl = aidl.source(path);
        }
    }

    // Only the data types, not the ISecureClock async service.
    for name in ["Timestamp.aidl", "TimeStampToken.aidl"] {
        let path = PathBuf::from("aidl/android/hardware/security/secureclock").join(name);
        aidl = aidl.source(path);
    }

    aidl.generate().unwrap();

    let generated_path = PathBuf::from(format!("{}/aidl.rs", std::env::var("OUT_DIR").unwrap()));
    let content = fs::read_to_string(&generated_path).unwrap();
    fs::write(&generated_path, &content).unwrap();

    // Best-effort rustfmt on the generated file.
    let _ = Command::new("rustfmt")
        .args([&generated_path.as_os_str().to_string_lossy().to_string()])
        .status();
}
