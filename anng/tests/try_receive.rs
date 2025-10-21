use anng::Message;
use std::{io::Write, time::Duration};
use tracing::Instrument;

#[tokio::test]
async fn contextful_try_receive_rep0() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let rep0 = anng::protocols::reqrep0::Rep0::listen(c"inproc://contextful_try_receive")
        .await
        .unwrap();
    let req0 = anng::protocols::reqrep0::Req0::dial(c"inproc://contextful_try_receive")
        .await
        .unwrap();

    let rep_task = async {
        let mut rep0 = rep0.context();

        // Initially, no message should be available
        tracing::info!("checking that try_receive returns None when no message available");
        assert!(rep0.try_receive().unwrap().is_none());

        // Poll with try_receive until a request arrives
        tracing::info!("polling with try_receive until request arrives");
        let (request, responder) = loop {
            match rep0.try_receive().unwrap() {
                Some(msg_and_responder) => {
                    tracing::info!("received request via try_receive");
                    break msg_and_responder;
                }
                None => {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    continue;
                }
            }
        };

        tracing::info!(bytes = ?request.as_slice(), "received request");
        assert_eq!(request.as_slice(), b"test request");

        // Send reply
        let mut reply = Message::default();
        write!(&mut reply, "test response").unwrap();
        responder.reply(reply).await.unwrap();
        tracing::info!("sent reply");

        // After handling one request, try_receive should return None again
        assert!(rep0.try_receive().unwrap().is_none());
    }
    .instrument(tracing::info_span!("rep0"));

    let req_task = async {
        let mut req0 = req0.context();

        // Small delay to ensure rep0 is listening
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Send a request
        tracing::info!("sending request");
        let mut request = Message::default();
        write!(&mut request, "test request").unwrap();
        let reply_future = req0.request(request).await.unwrap();

        // Wait for reply
        let reply = reply_future.await.unwrap();
        tracing::info!(bytes = ?reply.as_slice(), "received reply");
        assert_eq!(reply.as_slice(), b"test response");
    }
    .instrument(tracing::info_span!("req0"));

    tokio::join!(biased; rep_task, req_task);
}

#[tokio::test]
async fn socket_try_pull_pull0() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let mut push_socket =
        anng::protocols::pipeline0::Push0::listen(c"inproc://non_contextful_try_receive")
            .await
            .unwrap();
    let mut pull_socket =
        anng::protocols::pipeline0::Pull0::dial(c"inproc://non_contextful_try_receive")
            .await
            .unwrap();

    let push_task = async {
        // Small delay to ensure pull socket is connected
        tokio::time::sleep(Duration::from_millis(10)).await;

        tracing::info!("pushing message");
        let mut msg = Message::default();
        write!(&mut msg, "test message").unwrap();
        push_socket.push(msg).await.unwrap();
        tracing::info!("message pushed");
    }
    .instrument(tracing::info_span!("pusher"));

    let pull_task = async {
        // Initially, no message should be available
        tracing::info!("checking that try_pull returns None when no message available");
        assert!(pull_socket.try_pull().unwrap().is_none());

        // Poll with try_pull until a message arrives
        tracing::info!("polling with try_pull until message arrives");
        let msg = loop {
            match pull_socket.try_pull().unwrap() {
                Some(message) => {
                    tracing::info!("received message via try_pull");
                    break message;
                }
                None => {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    continue;
                }
            }
        };

        tracing::info!(bytes = ?msg.as_slice(), "received message");
        assert_eq!(msg.as_slice(), b"test message");

        // After receiving one message, try_pull should return None again
        assert!(pull_socket.try_pull().unwrap().is_none());
    }
    .instrument(tracing::info_span!("puller"));

    tokio::join!(biased; pull_task, push_task);
}
