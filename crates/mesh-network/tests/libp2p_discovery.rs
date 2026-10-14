//! Two in-process libp2p nodes connect over loopback and exchange a signed
//! advertisement through gossipsub.
#![cfg(feature = "libp2p")]

use std::time::Duration;

use mesh_identity::NodeIdentity;
use mesh_network::libp2p_transport::{Libp2pTransport, Multiaddr};
use mesh_network::{
    build_advertisement, keypair_from_identity, sign_advertisement, verify_advertisement,
    MeshTransport,
};
use mesh_protocol::ids::NodeId;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_nodes_exchange_advertisement() {
    let a_identity = NodeIdentity::generate(NodeId::new("node-a"));
    let b_identity = NodeIdentity::generate(NodeId::new("node-b"));

    let addr_a: Multiaddr = "/ip4/127.0.0.1/tcp/40021".parse().unwrap();
    let addr_b: Multiaddr = "/ip4/127.0.0.1/tcp/40022".parse().unwrap();

    let node_a = Libp2pTransport::start(
        keypair_from_identity(&a_identity).unwrap(),
        vec![addr_a.clone()],
        vec![],
        "mesh-test",
    )
    .await
    .expect("node a starts");

    let node_b = Libp2pTransport::start(
        keypair_from_identity(&b_identity).unwrap(),
        vec![addr_b],
        vec![(node_a.peer_id(), addr_a)],
        "mesh-test",
    )
    .await
    .expect("node b starts");

    let mut receiver = node_b.advertisements();

    let advertisement = sign_advertisement(
        &a_identity,
        build_advertisement(&a_identity, vec![], vec!["mesh/1".to_string()], 60, 1).unwrap(),
    );

    // Gossipsub mesh formation is asynchronous; publish until received.
    let mut received = None;
    for _ in 0..40 {
        node_a.advertise(advertisement.clone()).unwrap();
        if let Ok(Ok(signed)) =
            tokio::time::timeout(Duration::from_millis(500), receiver.recv()).await
        {
            received = Some(signed);
            break;
        }
    }

    let signed = received.expect("node b should receive node a's advertisement");
    assert_eq!(signed.advertisement.node_id.as_str(), "node-a");
    assert!(verify_advertisement(&signed).is_ok());
}
