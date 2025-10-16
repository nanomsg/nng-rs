use anng::Message;
use std::io::Write;
use tracing::Instrument;

#[tokio::test]
async fn basic_bus_communication() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut node1 = anng::protocols::bus0::Bus0::listen(c"inproc://basic_bus")
        .await
        .unwrap();
    let mut node2 = anng::protocols::bus0::Bus0::dial(c"inproc://basic_bus")
        .await
        .unwrap();

    // NOTE(jon): we run these concurrently rather than sequentially, because otherwise node1's
    // message to node2 may be dropped as node2 isn't receiving yet.

    let node1 = async {
        // Node 1 sends to Node 2
        let mut msg1 = Message::with_capacity(20);
        let _ = msg1.write(b"Hello from node 1").unwrap();
        node1.send(msg1).await.unwrap();

        // Node 1 receives message from Node 2
        let received = node1.receive().await.unwrap();
        assert_eq!(received.as_slice(), b"Hello from node 2");
    };

    let node2 = async {
        // Node 2 receives message from Node 1
        let received = node2.receive().await.unwrap();
        assert_eq!(received.as_slice(), b"Hello from node 1");

        // Give node1 a chance to start receiving
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Node 2 sends back to Node 1
        let mut msg2 = Message::with_capacity(20);
        let _ = msg2.write(b"Hello from node 2").unwrap();
        node2.send(msg2).await.unwrap();

        // Ensure the message is flushed before exiting
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    tokio::join!(biased; node2, node1);
}

#[tokio::test]
async fn sequential_messaging() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Test multiple messages sent sequentially
    let mut sender = anng::protocols::bus0::Bus0::listen(c"inproc://sequential")
        .await
        .unwrap();
    let mut receiver = anng::protocols::bus0::Bus0::dial(c"inproc://sequential")
        .await
        .unwrap();

    const NUM_MESSAGES: usize = 5;

    // Send messages one by one
    let sender = async {
        for i in 0..NUM_MESSAGES {
            // wait before sending a message; since bus is unreliable we want receiver ready
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;

            let mut msg = Message::with_capacity(20);
            write!(msg, "Message {}", i).unwrap();
            sender.send(msg).await.unwrap();
            tracing::info!("Sent message {}", i);
        }

        // NNG does not guarantee that all messages have been sent just because the sends have
        // completed, and there is no flush-style method, so wait a little before dropping the
        // socket (and thus closing it).
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    };

    // Receive all messages
    let mut received_messages = Vec::new();
    let receiver = async {
        for i in 0..NUM_MESSAGES {
            let msg = receiver.receive().await.unwrap();
            let content = String::from_utf8(msg.as_slice().to_vec()).unwrap();
            received_messages.push(content);
            tracing::info!("Received message {}", i);
        }
    };

    tokio::join!(biased; receiver, sender);

    // Verify we received the expected number of messages
    assert_eq!(received_messages.len(), NUM_MESSAGES);

    // Verify content
    for i in 0..NUM_MESSAGES {
        let expected = format!("Message {}", i);
        assert!(
            received_messages.contains(&expected),
            "Missing message: {}",
            expected
        );
    }
}

#[tokio::test]
async fn broadcast_behavior() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut sender = anng::protocols::bus0::Bus0::listen(c"inproc://broadcast")
        .await
        .unwrap();

    // Create multiple receivers
    let mut receiver1 = anng::protocols::bus0::Bus0::dial(c"inproc://broadcast")
        .await
        .unwrap();
    let mut receiver2 = anng::protocols::bus0::Bus0::dial(c"inproc://broadcast")
        .await
        .unwrap();
    let mut receiver3 = anng::protocols::bus0::Bus0::dial(c"inproc://broadcast")
        .await
        .unwrap();

    const NUM_BROADCASTS: u32 = 5;

    let broadcaster = async {
        for i in 0..NUM_BROADCASTS {
            // wait before sending a message; since bus is unreliable we want receiver ready
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;

            let mut msg = Message::with_capacity(20);
            write!(msg, "Broadcast {}", i).unwrap();
            sender.send(msg).await.unwrap();
            tracing::info!(i, "Sent broadcast");
        }
        tracing::info!("Broadcasting complete");

        // NNG does not guarantee that all messages have been sent just because the sends have
        // completed, and there is no flush-style method, so wait a little before dropping the
        // socket (and thus closing it).
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    .instrument(tracing::info_span!("broadcaster"));

    async fn receive_all_broadcasts(
        socket: &mut anng::Socket<anng::protocols::bus0::Bus0>,
        name: &str,
    ) -> Vec<String> {
        let mut messages = Vec::new();

        for i in 0..NUM_BROADCASTS {
            tracing::info!(i, "Waiting for broadcast {}", i);
            let msg = socket.receive().await.unwrap();
            let content = String::from_utf8(msg.as_slice().to_vec()).unwrap();
            messages.push(content.clone());
            tracing::info!(i, content, "{} received", name);
        }
        messages
    }

    let receiver1_task = receive_all_broadcasts(&mut receiver1, "receiver1")
        .instrument(tracing::info_span!("receiver1"));
    let receiver2_task = receive_all_broadcasts(&mut receiver2, "receiver2")
        .instrument(tracing::info_span!("receiver2"));
    let receiver3_task = receive_all_broadcasts(&mut receiver3, "receiver3")
        .instrument(tracing::info_span!("receiver3"));

    let (messages1, messages2, messages3, _) =
        tokio::join!(biased; receiver1_task, receiver2_task, receiver3_task, broadcaster);

    // Every receiver should get all broadcasts
    assert_eq!(messages1.len(), NUM_BROADCASTS as usize);
    assert_eq!(messages2.len(), NUM_BROADCASTS as usize);
    assert_eq!(messages3.len(), NUM_BROADCASTS as usize);

    // Verify content of all messages
    for i in 0..NUM_BROADCASTS {
        let expected = format!("Broadcast {}", i);
        assert!(messages1.contains(&expected));
        assert!(messages2.contains(&expected));
        assert!(messages3.contains(&expected));
    }
}

#[tokio::test]
async fn receive_cancellation_safety() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut sender = anng::protocols::bus0::Bus0::listen(c"inproc://recv_cancel")
        .await
        .unwrap();
    let mut receiver = anng::protocols::bus0::Bus0::dial(c"inproc://recv_cancel")
        .await
        .unwrap();

    // 1. Cancel receive before any message is sent
    let receive_future = receiver.receive();
    tokio::select! {
        _ = receive_future => {
            panic!("First receive should have been cancelled");
        }
        _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
            // This branch should execute (cancellation)
        }
    }

    // 2. Send a message after the cancelled receive
    let mut msg1 = Message::with_capacity(30);
    let _ = msg1.write(b"First message").unwrap();
    sender.send(msg1).await.unwrap();

    // Small delay to let message propagate
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    // 3. Cancel receive after message is available
    let receive_future = receiver.receive();
    let mut message_consumed = false;
    tokio::select! {
        result = receive_future => {
            // Message was received before cancellation - verify it's correct
            match result {
                Ok(msg) => {
                    assert_eq!(msg.as_slice(), b"First message");
                    message_consumed = true;
                },
                Err(e) => panic!("Unexpected receive error: {:?}", e),
            }
        }
        _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => {
            // Receive was cancelled - message should still be available
        }
    }

    // 4. Normal receive should get the message (if it wasn't consumed by cancelled receive)
    if !message_consumed {
        let received = receiver.receive().await.unwrap();
        assert_eq!(received.as_slice(), b"First message");
    }
}

#[tokio::test]
async fn send_cancellation_safety() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut sender = anng::protocols::bus0::Bus0::listen(c"inproc://send_cancel")
        .await
        .unwrap();
    let mut receiver = anng::protocols::bus0::Bus0::dial(c"inproc://send_cancel")
        .await
        .unwrap();

    // 0. Start the receiver so that no sent messages are dropped
    let (tx, mut received) = tokio::sync::mpsc::channel(2);
    let _receiver = tokio::task::spawn(async move {
        loop {
            let msg = receiver.receive().await.unwrap();
            if tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    // 1. Try to cancel send (though it might complete immediately for bus protocol)
    let mut msg1 = Message::with_capacity(30);
    let _ = msg1.write(b"Cancelled send message").unwrap();

    let mut send_succeeded = false;
    tokio::select! {
        result = sender.send(msg1)=> {
            // Send completed before cancellation - that's fine for bus protocol
            // It should succeed if it completes:
            result.unwrap();
            send_succeeded = true;
        }
        _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => {
            // Cancellation occurred - message may or may not have been sent
        }
    }

    // 2. Send another message after the cancellation attempt
    let mut msg2 = Message::with_capacity(30);
    let _ = msg2.write(b"Post-cancel message").unwrap();
    sender.send(msg2).await.unwrap();

    // 3. Try to receive - should get at least the post-cancel message
    let received1 = received.recv().await.unwrap();

    if received1.as_slice() == b"Cancelled send message" {
        // The cancelled send actually completed, so we should also get the post-cancel message
        let received2 = received.recv().await.unwrap();
        assert_eq!(received2.as_slice(), b"Post-cancel message");

        // Verify we got exactly both messages and no more
        tokio::select! {
            result = received.recv() => {
                panic!("Should not receive any more messages, but got: {:?}",
                       std::str::from_utf8(result.unwrap().as_slice()));
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                // Good - no more messages available
            }
        }
    } else {
        // If the send succeeded, we _should_ have gotten that send's message first!
        assert!(!send_succeeded);

        // We got the post-cancel message first, which means the cancelled send didn't complete
        assert_eq!(received1.as_slice(), b"Post-cancel message");

        // Verify we don't get the cancelled message
        tokio::select! {
            result = received.recv() => {
                panic!("Should not receive cancelled message, but got: {:?}",
                       std::str::from_utf8(result.unwrap().as_slice()));
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                // Good - the cancelled message was not sent
            }
        }
    }
}

#[tokio::test]
async fn send_error_handling() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut isolated_node = anng::protocols::bus0::Bus0::listen(c"inproc://isolated")
        .await
        .unwrap();

    // Try to send a message with no connected peers
    let mut msg = Message::with_capacity(20);
    let _ = msg.write(b"Isolated message").unwrap();

    // This should succeed even with no peers (bus protocol characteristic)
    let result = isolated_node.send(msg).await;
    assert!(result.is_ok(), "Bus send should succeed even without peers");
}
