use anng::Message;
use std::io::Write;
use tracing::Instrument;

#[tokio::test]
async fn basic_pair_communication() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut endpoint1 = anng::protocols::pair1::Pair1::listen(c"inproc://basic_pair")
        .await
        .unwrap();
    let mut endpoint2 = anng::protocols::pair1::Pair1::dial(c"inproc://basic_pair")
        .await
        .unwrap();

    // Endpoint 1 sends to Endpoint 2
    let mut msg1 = Message::with_capacity(20);
    let _ = msg1.write(b"Hello from endpoint 1").unwrap();
    endpoint1.send(msg1).await.unwrap();

    // Endpoint 2 receives message from Endpoint 1
    let received = endpoint2.receive().await.unwrap();
    assert_eq!(received.as_slice(), b"Hello from endpoint 1");

    // Endpoint 2 sends back to Endpoint 1
    let mut msg2 = Message::with_capacity(20);
    let _ = msg2.write(b"Hello from endpoint 2").unwrap();
    endpoint2.send(msg2).await.unwrap();

    // Endpoint 1 receives message from Endpoint 2
    let received = endpoint1.receive().await.unwrap();
    assert_eq!(received.as_slice(), b"Hello from endpoint 2");
}

#[tokio::test]
async fn sequential_messaging() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Test multiple messages sent sequentially between pair endpoints
    let mut sender = anng::protocols::pair1::Pair1::listen(c"inproc://sequential_pair")
        .await
        .unwrap();
    let mut receiver = anng::protocols::pair1::Pair1::dial(c"inproc://sequential_pair")
        .await
        .unwrap();

    const NUM_MESSAGES: usize = 5;

    // Send and receive messages one by one (better for pair protocol)
    let mut received_messages = Vec::new();
    for i in 0..NUM_MESSAGES {
        let mut msg = Message::with_capacity(20);
        write!(msg, "Message {}", i).unwrap();
        sender.send(msg).await.unwrap();
        tracing::info!("Sent message {}", i);

        // Immediately receive the message
        let received = receiver.receive().await.unwrap();
        let content = String::from_utf8(received.as_slice().to_vec()).unwrap();
        received_messages.push(content);
        tracing::info!("Received message {}", i);
    }

    // Verify we received the expected number of messages
    assert_eq!(received_messages.len(), NUM_MESSAGES);

    // Verify content (pair maintains message order)
    for (i, msg) in received_messages.iter().enumerate() {
        let expected = format!("Message {}", i);
        assert_eq!(msg, &expected);
    }
}

#[tokio::test]
async fn bidirectional_messaging() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut endpoint1 = anng::protocols::pair1::Pair1::listen(c"inproc://bidirectional")
        .await
        .unwrap();
    let mut endpoint2 = anng::protocols::pair1::Pair1::dial(c"inproc://bidirectional")
        .await
        .unwrap();

    const NUM_EXCHANGES: u32 = 3;

    let endpoint1_task = async {
        for i in 0..NUM_EXCHANGES {
            // Endpoint 1 sends
            let mut msg = Message::with_capacity(20);
            write!(msg, "From EP1: {}", i).unwrap();
            endpoint1.send(msg).await.unwrap();
            tracing::info!(i, "Endpoint 1 sent message");

            // Endpoint 1 receives response
            let response = endpoint1.receive().await.unwrap();
            let content = String::from_utf8(response.as_slice().to_vec()).unwrap();
            tracing::info!(i, content, "Endpoint 1 received");
            assert_eq!(content, format!("From EP2: {}", i));
        }
    }
    .instrument(tracing::info_span!("endpoint1"));

    let endpoint2_task = async {
        for i in 0..NUM_EXCHANGES {
            // Endpoint 2 receives
            let msg = endpoint2.receive().await.unwrap();
            let content = String::from_utf8(msg.as_slice().to_vec()).unwrap();
            tracing::info!(i, content, "Endpoint 2 received");
            assert_eq!(content, format!("From EP1: {}", i));

            // Endpoint 2 sends response
            let mut response = Message::with_capacity(20);
            write!(response, "From EP2: {}", i).unwrap();
            endpoint2.send(response).await.unwrap();
            tracing::info!(i, "Endpoint 2 sent response");
        }
    }
    .instrument(tracing::info_span!("endpoint2"));

    tokio::join!(endpoint1_task, endpoint2_task);
}

#[tokio::test]
async fn multiple_connection_behavior() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Test what happens when one side of a pair tries to connect to a new peer
    let mut endpoint1 = anng::protocols::pair1::Pair1::listen(c"inproc://multi_connect_1")
        .await
        .unwrap();
    let mut endpoint2 = anng::protocols::pair1::Pair1::dial(c"inproc://multi_connect_1")
        .await
        .unwrap();

    // Verify initial connection works
    let mut test_msg = Message::with_capacity(20);
    let _ = test_msg.write(b"Initial connection").unwrap();
    endpoint1.send(test_msg).await.unwrap();

    let received = endpoint2.receive().await.unwrap();
    assert_eq!(received.as_slice(), b"Initial connection");

    // Now try to have endpoint2 dial to a different address
    let _endpoint3 = anng::protocols::pair1::Pair1::listen(c"inproc://multi_connect_2")
        .await
        .unwrap();

    // The dial will succeed, but the new pipe is effectively immediately dropped and does _not_
    // affect the original pair connection (we remain connected to 1).
    endpoint2.dial(c"inproc://multi_connect_2").await.unwrap();

    // Verify original connection still works regardless
    let mut final_msg = Message::with_capacity(20);
    let _ = final_msg.write(b"Final test").unwrap();
    endpoint1.send(final_msg).await.unwrap();

    let final_received = endpoint2.receive().await.unwrap();
    assert_eq!(final_received.as_slice(), b"Final test");
}

#[tokio::test]
async fn send_error_handling() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut isolated_endpoint = anng::protocols::pair1::Pair1::listen(c"inproc://isolated_pair")
        .await
        .unwrap();

    // Try to send a message with no connected peer
    let mut msg = Message::with_capacity(20);
    let _ = msg.write(b"Isolated message").unwrap();

    // For pair protocol, sending without a peer blocks waiting for a connection
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        isolated_endpoint.send(msg),
    )
    .await;

    // The send should timeout because it blocks waiting for a peer
    assert!(
        result.is_err(),
        "Pair send should block/timeout when no peer is connected"
    );
}

#[tokio::test]
async fn connection_establishment() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Test that we can successfully establish a pair connection
    let listener = anng::protocols::pair1::Pair1::listen(c"inproc://connection_test")
        .await
        .unwrap();
    let dialer = anng::protocols::pair1::Pair1::dial(c"inproc://connection_test")
        .await
        .unwrap();

    // Both sockets should be successfully created
    // The connection is established during the dial/listen calls
    drop(listener);
    drop(dialer);
}

#[tokio::test]
async fn pair_maintains_original_connection() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Test that pair protocol maintains the original connection when new dials occur:
    // When a pair socket already has an active connection to one peer,
    // additional dial attempts may succeed but any new pipes are dropped, leaving the original connection intact.

    // 1. Set up first pair connection
    let mut endpoint1 = anng::protocols::pair1::Pair1::listen(c"inproc://pair_multi_test")
        .await
        .unwrap();
    let mut endpoint2 = anng::protocols::pair1::Pair1::dial(c"inproc://pair_multi_test")
        .await
        .unwrap();

    // Verify the initial connection works
    let mut msg = Message::with_capacity(20);
    let _ = msg.write(b"Test connection").unwrap();
    endpoint1.send(msg).await.unwrap();
    let received = endpoint2.receive().await.unwrap();
    assert_eq!(received.as_slice(), b"Test connection");

    // 2. Try to create a third endpoint that dials to the same listener
    // The dial may succeed but any new pipe will be immediately dropped by pair protocol
    let _endpoint3 = anng::protocols::pair1::Pair1::dial(c"inproc://pair_multi_test")
        .await
        .unwrap();

    // The third endpoint exists but any new pipe was dropped, so the original connection remains active

    // Verify original connection still works after the additional dial attempt
    let mut final_msg = Message::with_capacity(20);
    let _ = final_msg.write(b"Still connected").unwrap();
    endpoint1.send(final_msg).await.unwrap();
    let final_received = endpoint2.receive().await.unwrap();
    assert_eq!(final_received.as_slice(), b"Still connected");
}
