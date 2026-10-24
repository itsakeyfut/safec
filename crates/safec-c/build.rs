//! What this compiler was built to run on.
//!
//! `--target` defaults to it, the way `cc` and `rustc` default to their own
//! host. ADR-0013 records why that does not contradict `docs/architecture.md`'s
//! "Output depends on the target, never on the host": the host picks the
//! default and the output then depends on that target alone.
//!
//! Cargo sets `TARGET` for a build script and nothing else can answer as
//! exactly. Composing a triple from `std::env::consts` loses the vendor and the
//! environment, so `x86_64-pc-windows-msvc` and `x86_64-pc-windows-gnu` would
//! be one string, and they are two machines.

fn main() {
    // Not `rerun-if-changed`: a build script with no such line is re-run when
    // any file in the package changes, and this one has to be re-run whenever
    // cargo is invoked for a different target.
    println!(
        "cargo::rustc-env=SAFEC_HOST_TRIPLE={}",
        std::env::var("TARGET").expect("cargo sets TARGET for a build script")
    );
}
