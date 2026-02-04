// Note: NNG configuration is static per process. Since Rust runs each integration
// test file in a separate process, this file intentionally contains only one test.

use anng::init_nng;

/// Verifies that calling `init_nng(None)` always succeeds, even when called multiple times.
#[test]
fn init_with_none_always_succeeds() {
    init_nng(None).expect("first init_nng(None) should succeed");
    init_nng(None).expect("second init_nng(None) should succeed");
}
