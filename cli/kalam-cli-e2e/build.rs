fn main() {
    // Some CLI e2e tests include `cli/src/args.rs`, which embeds build metadata via
    // `env!`. Provide stable values when compiling this test crate directly.
    println!("cargo:rustc-env=GIT_COMMIT_HASH=testing");
    println!("cargo:rustc-env=GIT_BRANCH=testing");
    println!("cargo:rustc-env=BUILD_DATE=testing");
}
