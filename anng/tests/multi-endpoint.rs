#![cfg(nng_110)]

use anng::Message;
use std::{io::Write, sync::Arc, time::Duration};
use tokio::select;
use tracing::Instrument;

#[tokio::test]
async fn pub_sub_multiple_listeners() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Create a publisher socket
    let mut pub_socket = anng::protocols::pubsub0::Pub0::listen(c"inproc://multi_pub1")
        .await
        .unwrap();

    // Add an additional listener on a different address
    pub_socket.listen(c"inproc://multi_pub2").await.unwrap();

    // Create a single subscriber with multiple dialers
    let sub_socket = anng::protocols::pubsub0::Sub0::socket().unwrap();
    sub_socket.dial(c"inproc://multi_pub1").await.unwrap();
    sub_socket.dial(c"inproc://multi_pub2").await.unwrap();

    let mut sub1_ctx = sub_socket.context();
    sub1_ctx.disable_filtering();

    let mut sub2_ctx = sub_socket.context();
    sub2_ctx.disable_filtering();

    const N: u32 = 50;

    // Publisher task
    let publisher = async {
        for i in 0..N {
            // Give subscribers time to receive
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            let mut msg = Message::with_capacity(4);
            let _ = msg.write(&i.to_be_bytes()).unwrap();

            pub_socket.publish(msg).await.unwrap();
            tracing::debug!(i, "published");
        }
        tracing::info!("publishing done");

        // NNG does not guarantee that all messages have been sent just because the sends have
        // completed, and there is no flush-style method, so wait a little before dropping the
        // socket (and thus closing it).
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    .instrument(tracing::info_span!("publisher"));

    // There are two pipes between the subscriber socket and the publisher socket.
    // The publisher doesn't know those two pipes lead to the _same_ subscriber,
    // so it will send on both pipes, and thus the subscriber(s) will receive each message twice.
    // All messages go to _both_ subscribers, so we should see 2N messages *2.
    let subscriber1 = async {
        for i in 0..(2 * N) {
            select! {
                msg = sub1_ctx.next() => {
                    let msg = msg.unwrap();
                    tracing::debug!(?msg, "sub1 received");
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    panic!("sub1 timed out waiting for message {}", i);
                }
            }
        }
        tracing::info!("sub1 done");
    }
    .instrument(tracing::info_span!("sub1"));

    let subscriber2 = async {
        for i in 0..(2 * N) {
            select! {
                msg = sub2_ctx.next() => {
                    let msg = msg.unwrap();
                    tracing::debug!(?msg, "sub2 received");
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    panic!("sub2 timed out waiting for message {}", i);
                }
            }
        }
        tracing::info!("sub2 done");
    }
    .instrument(tracing::info_span!("sub2"));

    tokio::join!(biased; subscriber1, subscriber2, publisher);
}

#[tokio::test]
async fn socket_with_multiple_endpoints() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    const N: usize = 10_000;

    // Create a REP socket with multiple listeners
    // This demonstrates a socket accepting connections on multiple addresses
    let rep_socket = anng::protocols::reqrep0::Rep0::listen(c"inproc://multi_rep1")
        .await
        .unwrap();

    // Add a second listener to the same socket
    rep_socket.listen(c"inproc://multi_rep2").await.unwrap();

    // Create a single REQ socket with multiple dialers
    // NNG will handle load balancing/failover between the connections internally
    let req_socket = anng::protocols::reqrep0::Req0::socket().unwrap();
    req_socket.dial(c"inproc://multi_rep1").await.unwrap();
    req_socket.dial(c"inproc://multi_rep2").await.unwrap();

    // Multiple contexts on the same REQ socket can send requests
    // NNG determines which connection to use for each request
    let req1_task = async {
        let mut ctx = req_socket.context();

        for i in 0..N {
            let mut request = Message::with_capacity(20);
            let _ = request.write(b"from_req1").unwrap();

            tracing::info!(i, "request");
            let reply_future = ctx.request(request).await.unwrap();
            tracing::debug!("sent request");
            let reply = reply_future.await.unwrap();
            tracing::info!(bytes = ?reply.as_slice(), "response");
            assert_eq!(reply.as_slice(), b"from_req1");
        }
    }
    .instrument(tracing::info_span!("req1"));

    let req2_task = async {
        let mut ctx = req_socket.context();

        for i in 0..N {
            let mut request = Message::with_capacity(20);
            let _ = request.write(b"from_req2").unwrap();

            tracing::info!(i, "request");
            let reply_future = ctx.request(request).await.unwrap();
            tracing::debug!("sent request");
            let reply = reply_future.await.unwrap();
            tracing::info!(bytes = ?reply.as_slice(), "response");
            assert_eq!(reply.as_slice(), b"from_req2");
        }
    }
    .instrument(tracing::info_span!("req2"));

    // Let's have multiple (concurrent) request handlers:
    let mut handlers = tokio::task::JoinSet::new();
    let rep_socket = Arc::new(rep_socket);
    for i in 0..10 {
        let socket = Arc::clone(&rep_socket);
        handlers.spawn(
            async move {
                let mut ctx = socket.context();
                loop {
                    tracing::debug!("receive");
                    let (request, responder) = ctx.receive().await.unwrap();
                    tracing::info!(bytes = ?request.as_slice(), "got request");
                    responder.reply(request).await.unwrap();
                    tracing::debug!("sent reply");
                }
            }
            .instrument(tracing::info_span!("rep", i)),
        );
    }

    // Now let's see all those requests fly by!
    // (hopefully across multiple connections)
    tokio::join!(req1_task, req2_task);
}
