use std::sync::Arc;

use nicer::identity::Identity;
use nicer::tls;
use rustls_pki_types::ServerName;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

fn identity(name: &str) -> Identity {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .unwrap()
        .serialize_der();
    Identity::from_pkcs8(key, name).unwrap()
}

#[tokio::test]
async fn ed25519_identities_authenticate_both_directions() {
    let server_id = Arc::new(identity("server"));
    let client_id = Arc::new(identity("client"));
    let server_fp = server_id.fingerprint();
    let client_fp = client_id.fingerprint();

    let acceptor = TlsAcceptor::from(Arc::new(tls::server_config(&server_id, None).unwrap()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let tls = acceptor.accept(stream).await.unwrap();
        let certs = tls.get_ref().1.peer_certificates().unwrap().to_vec();
        tls::fingerprint_of(&certs[0]).unwrap()
    });

    let connector = TlsConnector::from(Arc::new(
        tls::client_config(&client_id, Some(server_fp), None).unwrap(),
    ));
    let stream = TcpStream::connect(addr).await.unwrap();
    let tls = connector
        .connect(
            ServerName::try_from(tls::PLACEHOLDER_SERVER_NAME).unwrap(),
            stream,
        )
        .await
        .unwrap();

    let seen_by_client = tls::fingerprint_of(
        tls.get_ref()
            .1
            .peer_certificates()
            .unwrap()
            .first()
            .unwrap(),
    )
    .unwrap();
    let seen_by_server = server.await.unwrap();

    assert_eq!(seen_by_client, server_fp);
    assert_eq!(seen_by_server, client_fp);
}

#[tokio::test]
async fn a_pinned_identity_that_changed_fails_the_handshake() {
    let server_id = Arc::new(identity("server"));
    let impostor = identity("impostor");

    let acceptor = TlsAcceptor::from(Arc::new(tls::server_config(&server_id, None).unwrap()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = acceptor.accept(stream).await;
        }
    });

    let client_id = identity("client");
    let connector = TlsConnector::from(Arc::new(
        tls::client_config(&client_id, Some(impostor.fingerprint()), None).unwrap(),
    ));
    let stream = TcpStream::connect(addr).await.unwrap();
    let result = connector
        .connect(
            ServerName::try_from(tls::PLACEHOLDER_SERVER_NAME).unwrap(),
            stream,
        )
        .await;

    assert!(
        result.is_err(),
        "a pinned fingerprint mismatch must not succeed"
    );
}
