use anng::protocols::*;
use std::io::ErrorKind;

#[tokio::test]
async fn test_invalid_url_format() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // NNG returns NNG_ENOTSUP for empty/incomplete URLs, which maps to Unsupported.
    // Earlier versions returned NNG_EINVAL (InvalidInput). Accept both for compatibility.
    let invalid_urls = vec![("", "empty URL"), ("tcp://", "incomplete URL")];

    for (url, description) in invalid_urls {
        let url_cstr = std::ffi::CString::new(url).unwrap();
        let result = reqrep0::Req0::dial(url_cstr.as_c_str()).await;

        let err = result.expect_err(&format!("Expected error for {}", description));
        assert!(
            err.kind() == ErrorKind::InvalidInput || err.kind() == ErrorKind::Unsupported,
            "Expected InvalidInput or Unsupported for '{}', got {:?}",
            description,
            err
        );
    }
}

#[tokio::test]
async fn test_missing_scheme() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let result = reqrep0::Req0::dial(c"://localhost:8080").await;
    let err = result.expect_err("Expected error for missing scheme");
    assert_eq!(
        err.kind(),
        ErrorKind::Unsupported,
        "Expected Unsupported for missing scheme, got {:?}",
        err
    );
}

#[tokio::test]
async fn test_invalid_addresses() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let invalid = vec![
        (c"tcp://256.256.256.256:8080", "IP octets out of range"),
        (c"tcp://999.999.999.999:8080", "clearly invalid IP"),
        (c"tcp://127.0.0.1:70000", "port out of range"),
        (c"tcp://127.0.0.1:abc", "non-numeric port"),
        (c"tcp://[::gggg]:8080", "bad ipv6 address"),
    ];

    for (url, description) in invalid {
        let result = reqrep0::Req0::dial(url).await;

        let err = result.expect_err(&format!("Expected error for {}", description));
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "Expected InvalidInput for '{}', got {:?}",
            description,
            err
        );
    }
}

#[tokio::test]
async fn test_unsupported_transports() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let unsupported = vec![
        "http://example.com:8080",
        "ftp://example.com:21",
        "unknown://test:8080",
    ];

    for url in unsupported {
        let url_cstr = std::ffi::CString::new(url).unwrap();
        let err = reqrep0::Req0::dial(url_cstr.as_c_str()).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unsupported, "{err:?}");
    }
}

#[tokio::test]
async fn test_connection_refused() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Try to connect to a port unlikely to be listening
    let err = reqrep0::Req0::dial(c"tcp://127.0.0.1:1").await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ConnectionRefused, "{err:?}");
}

#[tokio::test]
async fn test_addr_in_use() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Bind to port 0 to let the OS choose an available port
    let socket1 = reqrep0::Rep0::socket().unwrap();
    let listener1 = socket1
        .listen_tcp("127.0.0.1:0".parse().unwrap(), &Default::default())
        .await
        .unwrap();

    // Get the actual port that was assigned
    let actual_addr = listener1.local_addr().unwrap();

    // Try to bind a second socket to the same address
    let socket2 = reqrep0::Rep0::socket().unwrap();
    let err = socket2
        .listen_tcp(actual_addr, &Default::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AddrInUse, "{err:?}");
}

#[tokio::test]
async fn test_permission_denied() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Attempt to bind to port 1, which is privileged on all platforms.
    // This test assumes we're not running as root/admin.
    let err = reqrep0::Rep0::listen(c"tcp://127.0.0.1:1")
        .await
        .unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::PermissionDenied,
        "Expected PermissionDenied for privileged port 1, got {:?}",
        err
    );
}

#[tokio::test]
async fn test_cross_protocol_consistency() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Test that all protocols handle invalid addresses consistently
    let invalid_addr = c"tcp://999.888.777.666:8080";

    macro_rules! test_protocol {
        ($name:expr, $dial:expr) => {
            tracing::info!("attempting protocol {}", $name);
            let err = $dial.await.unwrap_err();
            assert_eq!(
                err.kind(),
                ErrorKind::InvalidInput,
                "Protocol {} should reject invalid address with InvalidInput, got {:?}",
                $name,
                err
            );
        };
    }

    test_protocol!("REQ0", reqrep0::Req0::dial(invalid_addr));
    test_protocol!("REP0", reqrep0::Rep0::dial(invalid_addr));
    test_protocol!("PUB0", pubsub0::Pub0::dial(invalid_addr));
    test_protocol!("SUB0", pubsub0::Sub0::dial(invalid_addr));
    test_protocol!("PUSH0", pipeline0::Push0::dial(invalid_addr));
    test_protocol!("PULL0", pipeline0::Pull0::dial(invalid_addr));
    test_protocol!("PAIR1", pair1::Pair1::dial(invalid_addr));
    test_protocol!("BUS0", bus0::Bus0::dial(invalid_addr));
    test_protocol!("SURVEYOR0", survey0::Surveyor0::dial(invalid_addr));
    test_protocol!("RESPONDENT0", survey0::Respondent0::dial(invalid_addr));
}
