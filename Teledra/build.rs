//! Stamps a build identity into the binary.
//!
//! V1 carried one and V2 dropped it, which cost a stream: `Teledra.bat` falls
//! back to launching the previous executable when a build fails, and there was
//! no way to tell a running court from a stale one except by noticing a feature
//! was missing. An identity in the backstage trace answers it in a glance.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn main() {
    let revision = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "no-git".to_owned());
    // Uncommitted work is the normal state during a session, and it is exactly
    // when "which build is this?" matters most.
    let state = if git(&["status", "--porcelain"]).is_some() {
        "dirty"
    } else {
        "clean"
    };
    let built_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    println!("cargo:rustc-env=TELEDRA_BUILD_ID={revision}.{state}.{built_at}");
}
