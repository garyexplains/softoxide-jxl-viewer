//! jxlviewer binary entry point. Far logic lives in the library
//! (`jxlviewer::cli`) so integration tests can exercise it.

fn main() {
    let code = jxlviewer::cli_main();
    if code != 0 {
        std::process::exit(code);
    }
}
