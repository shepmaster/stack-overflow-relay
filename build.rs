use std::env;

fn main() {
    // Heroku doesn't have the git repository, so we need to get the
    // SHA from the environment variable it provides.
    if let Ok(source_version) = env::var("SOURCE_VERSION") {
        println!("cargo:rustc-env=VERGEN_GIT_SHA={source_version}");
    } else {
        vergen::vergen(vergen::Config::default()).expect("Unable to generate the cargo keys!");
    }
}
