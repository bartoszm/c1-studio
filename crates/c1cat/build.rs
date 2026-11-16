fn main() {
    // Unoptimized, the argument parser and `main`'s many commands need more than the
    // 1 MiB Windows gives the main thread (Linux gives 8 MiB): give c1cat as much there.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    }
}
