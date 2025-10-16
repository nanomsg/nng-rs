use anng::Message;
use std::{io::Write, time::Duration};
use tracing::Instrument;

#[tokio::test]
async fn ping_pong() {
    tracing_subscriber::fmt().with_test_writer().init();

    let rep0 = anng::protocols::reqrep0::Rep0::listen(c"inproc://ping_pong")
        .await
        .unwrap();
    let req0 = anng::protocols::reqrep0::Req0::dial(c"inproc://ping_pong")
        .await
        .unwrap();

    const N: u32 = 1_000;
    let rep0 = async {
        let mut rep0 = rep0.context();
        for i in 0..N {
            tracing::info!(i, "expect");
            // the loop + select! is _just_ to test cancellation safety
            let (msg, reply) = loop {
                tokio::select! {
                    biased;
                    r = rep0.receive() => { break r.unwrap(); }
                    _ = tokio::time::sleep(Duration::from_micros(1)) => {}
                }
            };
            tracing::debug!(bytes = ?msg.as_slice(), "got request");
            assert_eq!(msg.as_slice(), i.to_be_bytes());
            reply.reply(msg).await.unwrap();
            tracing::debug!("sent reply");
        }
    }
    .instrument(tracing::info_span!("rep0"));
    let req0 = async {
        let mut req0 = req0.context();
        for i in 0..N {
            tracing::info!(i, "request");
            let mut msg = Message::with_capacity(std::mem::size_of_val(&i));
            let _ = msg.write(&i.to_be_bytes()).unwrap();
            let reply = req0.request(msg).await.unwrap();
            tracing::debug!("sent request");
            let msg = reply.await.unwrap();
            tracing::info!(bytes = ?msg.as_slice(), "response");
            assert_eq!(msg.as_slice(), i.to_be_bytes());
        }
    }
    .instrument(tracing::info_span!("req0"));

    tokio::join!(req0, rep0);
}
