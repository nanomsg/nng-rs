use anng::{Message, pipes::Addr};
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

        // Check remote address - should be inet with localhost
        let addr = msg.remote_addr().unwrap();
        let Addr::Inet(v4) = addr else {
            panic!("Expected inet address, got: {addr}");
        };

        assert_eq!(v4.ip(), &Ipv4Addr::LOCALHOST);
        // Should be some ephemeral port from the client
        assert_ne!(v4.port(), 0);
        // and it should _not_ be the listening port.
        assert_ne!(v4.port(), listener_addr.port());

        reply.reply(msg).await.unwrap();
    }
    .instrument(tracing::info_span!("rep0"));

    let req_task = async {
        let mut req0 = req0.context();
        let mut msg = Message::with_capacity(4);
        let _ = msg.write(b"test").unwrap();
        let reply = req0.request(msg).await.unwrap();
        let reply_msg = reply.await.unwrap();

        // Check remote address of the reply - should be inet with the listener address
        let addr = reply_msg.remote_addr().unwrap();
        let Addr::Inet(v4) = addr else {
            panic!("Expected inet address, got: {addr}");
        };

        assert_eq!(v4.ip(), &Ipv4Addr::LOCALHOST);
        assert_eq!(v4.port(), listener_addr.port()); // Should be the listener port
        reply_msg
    }
    .instrument(tracing::info_span!("req0"));

    let (_, req_msg) = tokio::join!(rep_task, req_task);

    // Explicitly close the connection which invalidates the pipe referenced
    // in the message
    drop(req0);

    assert!(req_msg.remote_addr().is_none());
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

        // Check remote address - should be inproc with the connection name
        let addr = msg.remote_addr().unwrap();
        let Addr::Inproc { name } = addr else {
            panic!("Expected inproc address, got: {addr}");
        };

        assert_eq!(name, CString::new("inproc://test_remote_addr").unwrap());

        reply.reply(msg).await.unwrap();
    }
    .instrument(tracing::info_span!("rep0"));

    let req_task = async {
        let mut req0 = req0.context();
        let mut msg = Message::with_capacity(4);
        let _ = msg.write(b"test").unwrap();
        let reply = req0.request(msg).await.unwrap();
        let reply_msg = reply.await.unwrap();

        // Check remote address of the reply - should be inproc with the connection name
        let addr = reply_msg.remote_addr().unwrap();
        let Addr::Inproc { name } = addr else {
            panic!("Expected inproc address, got: {addr}");
        };

        assert_eq!(name, CString::new("inproc://test_remote_addr").unwrap());
    }
    .instrument(tracing::info_span!("req0"));

    tokio::join!(rep_task, req_task);
}
