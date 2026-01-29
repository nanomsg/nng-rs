//! Integration tests for NNG initialization

use anng::{InitError, NngConfig, init_nng};

#[test]
fn init_with_none_always_succeeds() {
    assert!(init_nng(None).is_ok());
}

#[test]
fn init_with_default_config_succeeds() {
    let config = NngConfig::default();
    let result = init_nng(Some(config));
    // Either succeeds or already initialized (depends on test order)
    assert!(result.is_ok() || matches!(result, Err(InitError::AlreadyInitialized)));
}
