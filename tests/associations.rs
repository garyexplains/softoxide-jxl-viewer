//! Windows file-association registration round-trip.
//!
//! Uses a synthetic extension NOT in the supported list so the test never
//! touches real user associations; HKCU keys are written and removed by the
//! test itself, leaving the user's machine unchanged.

#![cfg(windows)]

use jxlviewer::assoc;

#[test]
fn register_unregister_roundtrip() {
    let ext = "zzztestjxl";
    assert!(!assoc::is_registered(ext), "pre-state must be unregistered");

    let msg = assoc::register(ext).expect("register should succeed");
    assert!(
        msg.contains("registered") || msg.contains("default"),
        "msg: {msg}"
    );
    assert!(assoc::is_registered(ext), "post-register state");

    let msg = assoc::unregister(ext).expect("unregister should succeed");
    assert!(msg.contains("removed"), "msg: {msg}");
    assert!(!assoc::is_registered(ext), "post-unregister state");
}

#[test]
fn format_catalog_is_sane() {
    let jxl = assoc::format_for("jxl").expect("jxl must be in the catalog");
    assert_eq!(jxl.ext, "jxl");
    assert!(assoc::format_for(".PNG").is_some(), "dots/case tolerated");
    assert!(assoc::format_for("nope").is_none());
    assert!(assoc::FORMATS.iter().any(|f| f.ext == assoc::DEFAULT_EXT));
}
