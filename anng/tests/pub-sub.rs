use anng::Message;
use std::io::Write;
use std::time::Duration;
use tracing::Instrument;

#[tokio::test]
async fn pubsub() {
    tracing_subscriber::fmt().with_test_writer().init();

    let mut pub0 = anng::protocols::pubsub0::Pub0::listen(c"inproc://foobar")
        .await
        .unwrap();
    let sub0 = anng::protocols::pubsub0::Sub0::dial(c"inproc://foobar")
        .await
        .unwrap();
    let mut sub0_1 = sub0.context();
    sub0_1.disable_filtering();
    let mut sub0_2 = sub0.context();
    // should _only_ receive numbers above 256
    sub0_2.subscribe_to(&[0x00, 0x00, 0x01]);

    // without a barrier, this test is racy since pub/sub doesn't guarantee that every message is
    // delivered. and indeed, it _can_ be observed to fail if the subscribers fall particularly far
    // behind. but fixing it by allowing holes would make the test _much_ less complete, so we fix
    // the race for now, even if it means we're testing the concurrency less.
    let barrier = tokio::sync::Barrier::new(3);

    const N: u32 = 0x2000;
    let pub0 = async {
        for i in 0..N {
            barrier.wait().await;
            tracing::info!(i, "publish");
            let mut msg = Message::with_capacity(std::mem::size_of_val(&i));
            let _ = msg.write(&i.to_be_bytes()).unwrap();
            pub0.publish(msg).await.unwrap();
            tracing::debug!("published");
            // give subscribers a chance to not miss messages
            tokio::task::yield_now().await;
        }
        tracing::info!("done");

        // NNG does not guarantee that all messages have been sent just because the sends have
        // completed, and there is no flush-style method, so wait a little before dropping the
        // socket (and thus closing it).
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    .instrument(tracing::info_span!("pub0"));
    let sub0_1 = async {
        for i in 0..N {
            barrier.wait().await;
            tracing::info!(i, "expect");
            let msg = sub0_1.next().await.unwrap();
            assert_eq!(msg.as_slice(), i.to_be_bytes());
            tracing::debug!("satisfied");
        }
        tracing::info!("done");
    }
    .instrument(tracing::info_span!("sub0_1"));
    let sub0_2 = async {
        for _ in 0..0x0100 {
            barrier.wait().await;
        }
        for i in 0x0100..std::cmp::min(N, 0x0200) {
            barrier.wait().await;
            tracing::info!(i, "expect");
            let msg = sub0_2.next().await.unwrap();
            assert_eq!(msg.as_slice(), i.to_be_bytes());
            tracing::debug!("satisfied");
        }
        for _ in std::cmp::min(N, 0x0200)..N {
            barrier.wait().await;
        }
        tracing::info!("done");
    }
    .instrument(tracing::info_span!("sub0_2"));

    tokio::time::timeout(Duration::from_secs(30), async move {
        tokio::join!(biased; sub0_1, sub0_2, pub0);
    })
    .await
    .unwrap()
}
