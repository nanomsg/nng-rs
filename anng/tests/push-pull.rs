use anng::Message;
use std::io::Write;
use tracing::Instrument;

#[tokio::test]
async fn basic_push_pull() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut push_socket = anng::protocols::pipeline0::Push0::listen(c"inproc://basic")
        .await
        .unwrap();
    let mut pull_socket = anng::protocols::pipeline0::Pull0::dial(c"inproc://basic")
        .await
        .unwrap();

    const N: u32 = 100;
    let pusher = async {
        for i in 0..N {
            tracing::info!(i, "push");
            let mut msg = Message::with_capacity(std::mem::size_of_val(&i));
            let _ = msg.write(&i.to_be_bytes()).unwrap();
            push_socket.push(msg).await.unwrap();
            tracing::debug!("pushed");
        }
        tracing::info!("pusher done");
    }
    .instrument(tracing::info_span!("pusher"));

    let puller = async {
        for i in 0..N {
            tracing::info!(i, "expect");
            let msg = pull_socket.pull().await.unwrap();
            assert_eq!(msg.as_slice(), i.to_be_bytes());
            tracing::debug!("received");
        }
        tracing::info!("puller done");
    }
    .instrument(tracing::info_span!("puller"));

    tokio::join!(pusher, puller);
}

#[tokio::test]
async fn load_balancing() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut push_socket = anng::protocols::pipeline0::Push0::listen(c"inproc://loadbalance")
        .await
        .unwrap();

    // Create multiple pull sockets to test round-robin distribution
    let pull1 = anng::protocols::pipeline0::Pull0::dial(c"inproc://loadbalance")
        .await
        .unwrap();
    let pull2 = anng::protocols::pipeline0::Pull0::dial(c"inproc://loadbalance")
        .await
        .unwrap();
    let pull3 = anng::protocols::pipeline0::Pull0::dial(c"inproc://loadbalance")
        .await
        .unwrap();

    const TOTAL_MESSAGES: u32 = 120; // Divisible by 3 for even distribution

    let pusher = async {
        for i in 0..TOTAL_MESSAGES {
            tracing::info!(i, "push");
            let mut msg = Message::with_capacity(std::mem::size_of_val(&i));
            let _ = msg.write(&i.to_be_bytes()).unwrap();
            push_socket.push(msg).await.unwrap();
            tracing::debug!("pushed");
        }
        tracing::info!("pusher done");
    }
    .instrument(tracing::info_span!("pusher"));

    async fn create_puller(
        mut pull_socket: anng::Socket<anng::protocols::pipeline0::Pull0>,
        name: &str,
    ) -> u32 {
        let mut count = 0;
        let mut received_values = Vec::new();

        loop {
            tokio::select! {
                result = pull_socket.pull() => {
                    match result {
                        Ok(msg) => {
                            let value = u32::from_be_bytes(msg.as_slice().try_into().unwrap());
                            received_values.push(value);
                            count += 1;
                            tracing::debug!(value, count, "{} received", name);
                        }
                        Err(_) => break,
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    if count > 0 {
                        break;
                    }
                }
            }
        }
        tracing::info!(count, ?received_values, "{} done", name);
        count
    }

    // Each puller should receive approximately 1/3 of the messages due to round-robin
    let puller1 = create_puller(pull1, "puller1").instrument(tracing::info_span!("puller1"));
    let puller2 = create_puller(pull2, "puller2").instrument(tracing::info_span!("puller2"));
    let puller3 = create_puller(pull3, "puller3").instrument(tracing::info_span!("puller3"));

    let (count1, count2, count3, _) = tokio::join!(biased; puller1, puller2, puller3, pusher);

    // Verify that messages were distributed among the pullers
    let total_received = count1 + count2 + count3;
    assert_eq!(
        total_received, TOTAL_MESSAGES,
        "All messages should be received exactly once"
    );

    // Each puller should receive some messages (load balancing verification)
    assert!(count1 > 0, "Puller 1 should receive some messages");
    assert!(count2 > 0, "Puller 2 should receive some messages");
    assert!(count3 > 0, "Puller 3 should receive some messages");
}

#[tokio::test]
async fn flow_control() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut push_socket = anng::protocols::pipeline0::Push0::listen(c"inproc://flow_control")
        .await
        .unwrap();

    // Don't create pull socket immediately to test flow control
    const N: u32 = 50;

    // Try to send messages without any pullers connected
    let push_task = tokio::spawn(async move {
        for i in 0..N {
            let mut msg = Message::with_capacity(std::mem::size_of_val(&i));
            let _ = msg.write(&i.to_be_bytes()).unwrap();
            tracing::info!(i, "attempting push without puller");

            // This should eventually succeed once a puller connects
            let result = push_socket.push(msg).await;
            assert!(result.is_ok(), "Push should succeed once puller connects");
            tracing::info!(i, "push succeeded");
        }

        // NNG does not guarantee that all messages have been sent just because the sends have
        // completed, and there is no flush-style method, so wait a little before dropping the
        // socket (and thus closing it).
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });

    // Wait a bit before pushes start
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Now connect a puller
    let mut pull_socket = anng::protocols::pipeline0::Pull0::dial(c"inproc://flow_control")
        .await
        .unwrap();

    // Receive all messages
    for i in 0..N {
        let msg = pull_socket.pull().await.unwrap();
        assert_eq!(msg.as_slice(), i.to_be_bytes());
        tracing::info!(i, "received");
    }

    // Wait for push task to complete
    push_task.await.unwrap();
}
