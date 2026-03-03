use anng::Message;
use std::{
    ffi::CString,
    io::Write,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
};
use tracing::Instrument;

#[tokio::test]
async fn remote_addr_tcp() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let socket = anng::protocols::reqrep0::Rep0::socket().unwrap();
    let listener = socket
        .listen_tcp(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            &anng::pipes::TcpOptions::default(),
        )
        .await
        .unwrap();

    let listener_addr = listener.local_addr().unwrap();
    tracing::info!(?listener_addr, "Listener bound to address");

    let url = CString::new(format!("tcp://{}", listener_addr)).unwrap();
    let req0 = anng::protocols::reqrep0::Req0::dial(&url).await.unwrap();

    let rep_task = async {
        let mut rep0 = socket.context();
        let (msg, reply) = rep0.receive().await.unwrap();

        // Check remote address - should be a tcp:// URL
        let addr = msg.remote_addr().unwrap();
        assert!(
            addr.as_str().starts_with("tcp://"),
            "Expected tcp:// URL, got: {addr}"
        );

        // Parse and verify the address
        let addr_part = addr.as_str().strip_prefix("tcp://").unwrap();
        let sock_addr: SocketAddr = addr_part.parse().unwrap();
        assert!(
            sock_addr.ip().is_loopback(),
            "Expected loopback, got: {sock_addr}"
        );
        assert_ne!(sock_addr.port(), 0);
        assert_ne!(sock_addr.port(), listener_addr.port());

        reply.reply(msg).await.unwrap();
    }
    .instrument(tracing::info_span!("rep0"));

    let req_task = async {
        let mut req0 = req0.context();
        let mut msg = Message::with_capacity(4);
        let _ = msg.write(b"test").unwrap();
        let reply = req0.request(msg).await.unwrap();
        let reply_msg = reply.await.unwrap();

        // Check remote address of the reply - should be tcp:// URL with the listener address
        let addr = reply_msg.remote_addr().unwrap();
        assert!(
            addr.as_str().starts_with("tcp://"),
            "Expected tcp:// URL, got: {addr}"
        );

        let addr_part = addr.as_str().strip_prefix("tcp://").unwrap();
        let sock_addr: SocketAddr = addr_part.parse().unwrap();
        assert!(sock_addr.ip().is_loopback());
        assert_eq!(sock_addr.port(), listener_addr.port());
        reply_msg
    }
    .instrument(tracing::info_span!("req0"));

    let (_, req_msg) = tokio::join!(rep_task, req_task);

    // Explicitly close the connection which invalidates the pipe referenced
    // in the message
    drop(req0);

    assert_eq!(req_msg.remote_addr(), None);
}

#[tokio::test]
async fn remote_addr_inproc() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let rep0 = anng::protocols::reqrep0::Rep0::listen(c"inproc://test_remote_addr")
        .await
        .unwrap();
    let req0 = anng::protocols::reqrep0::Req0::dial(c"inproc://test_remote_addr")
        .await
        .unwrap();

    let rep_task = async {
        let mut rep0 = rep0.context();
        let (msg, reply) = rep0.receive().await.unwrap();

        // Check remote address - should be inproc:// URL
        let addr = msg.remote_addr().unwrap();
        assert_eq!(addr.as_str(), "inproc://test_remote_addr");

        reply.reply(msg).await.unwrap();
    }
    .instrument(tracing::info_span!("rep0"));

    let req_task = async {
        let mut req0 = req0.context();
        let mut msg = Message::with_capacity(4);
        let _ = msg.write(b"test").unwrap();
        let reply = req0.request(msg).await.unwrap();
        let reply_msg = reply.await.unwrap();

        // Check remote address of the reply - should be inproc:// URL
        let addr = reply_msg.remote_addr().unwrap();
        assert_eq!(addr.as_str(), "inproc://test_remote_addr");
    }
    .instrument(tracing::info_span!("req0"));

    tokio::join!(rep_task, req_task);
}
