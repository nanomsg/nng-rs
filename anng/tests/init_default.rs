use anng::{init::NngConfig, init_nng};

/// Verifies that initializing with the default configuration succeeds.
#[test]
fn init_with_default_config_succeeds() {
    let config = NngConfig::default();
    init_nng(Some(config)).expect("can initialize nng with default config");
}
