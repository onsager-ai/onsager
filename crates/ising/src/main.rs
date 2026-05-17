//! `ising` — deprecation stub. See the crate docs and spec #362
//! (OBS-02) for the substrate-Observer replacement.

fn main() {
    eprintln!(
        "ising is deprecated. The 0.1 analyzer engine has been replaced \
         by the substrate Observer runtime (onsager-observers, spec #362).\n\
         The `ising` binary is now a no-op; nothing is started."
    );
    std::process::exit(0);
}
