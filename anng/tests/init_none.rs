use anng::init_nng;

/// Verifies that calling `init_nng(None)` always succeeds, even when called multiple times.
#[test]
fn init_with_none_always_succeeds() {
    init_nng(None).expect("first init_nng(None) should succeed");
    init_nng(None).expect("second init_nng(None) should succeed");
}
