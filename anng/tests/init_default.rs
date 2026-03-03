// Note: NNG configuration is static per process. Since Rust runs each integration
// test file in a separate process, this file intentionally contains only one test.

use anng::{init::NngConfig, init_nng};

/// Verifies that initializing with the default configuration succeeds.
#[test]
fn init_with_default_config_succeeds() {
    let config = NngConfig::default();
    init_nng(Some(config)).expect("can initialize nng with default config");
}
