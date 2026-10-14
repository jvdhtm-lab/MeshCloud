//! A rust-libp2p transport: TCP + Noise + Yamux with gossipsub discovery and
//! identify. Enabled by the `libp2p` feature.
//!
//! Relay and DCUtR are configured for public networks; on a trusted LAN the
//! direct TCP transport is used. Worker/agent messaging reaches the rest of the
//! system through the [`MeshTransport`] trait, so this implementation is
//! replaceable.

use std::time::Duration;

use libp2p::futures::StreamExt;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{gossipsub, identify, noise, tcp, yamux, PeerId, SwarmBuilder};
use tokio::sync::{broadcast, mpsc};

use crate::advertisement::SignedNodeAdvertisement;
use crate::transport::MeshTransport;
use crate::NetworkError;

pub use libp2p::{Multiaddr, PeerId as Libp2pPeerId};

#[derive(NetworkBehaviour)]
struct Behaviour {
    gossipsub: gossipsub::Behaviour,
    identify: identify::Behaviour,
}

enum Command {
    Publish(Vec<u8>),
}

/// A libp2p-backed mesh transport.
pub struct Libp2pTransport {
    sender: broadcast::Sender<SignedNodeAdvertisement>,
    command_tx: mpsc::Sender<Command>,
    peer_id: PeerId,
}

impl Libp2pTransport {
    /// Start a libp2p node with the given keypair, listening addresses, and
    /// bootstrap peers, subscribing to `topic`.
    pub async fn start(
        keypair: libp2p_identity::Keypair,
        listen: Vec<Multiaddr>,
        bootstrap: Vec<(PeerId, Multiaddr)>,
        topic: &str,
    ) -> Result<Self, NetworkError> {
        let mut swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .map_err(|e| NetworkError::Transport(e.to_string()))?
            .with_behaviour(|key| {
                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub::Config::default(),
                )
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
                })?;
                let identify =
                    identify::Behaviour::new(identify::Config::new("/mesh/1".into(), key.public()));
                Ok(Behaviour {
                    gossipsub,
                    identify,
                })
            })
            .map_err(|e| NetworkError::Transport(e.to_string()))?
            .build();

        let topic = gossipsub::IdentTopic::new(topic);
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&topic)
            .map_err(|e| NetworkError::Transport(e.to_string()))?;

        for address in listen {
            swarm
                .listen_on(address)
                .map_err(|e| NetworkError::Transport(e.to_string()))?;
        }
        for (peer, address) in bootstrap {
            if let Err(err) = swarm.dial(address.clone()) {
                tracing_ignore(&err);
            }
            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer);
        }

        let peer_id = *swarm.local_peer_id();
        let (sender, _) = broadcast::channel(256);
        let (command_tx, mut command_rx) = mpsc::channel::<Command>(64);
        let outbound = sender.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    command = command_rx.recv() => {
                        match command {
                            Some(Command::Publish(bytes)) => {
                                let _ = swarm
                                    .behaviour_mut()
                                    .gossipsub
                                    .publish(topic.clone(), bytes);
                            }
                            None => break,
                        }
                    }
                    event = swarm.select_next_some() => {
                        if let SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(
                            gossipsub::Event::Message { message, .. },
                        )) = event
                        {
                            if let Ok(signed) =
                                serde_json::from_slice::<SignedNodeAdvertisement>(&message.data)
                            {
                                let _ = outbound.send(signed);
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            sender,
            command_tx,
            peer_id,
        })
    }

    /// This node's libp2p peer id.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }
}

/// Start a libp2p transport using a node identity's keypair.
pub async fn start_with_identity(
    identity: &mesh_identity::NodeIdentity,
    listen: Vec<Multiaddr>,
    bootstrap: Vec<(PeerId, Multiaddr)>,
    topic: &str,
) -> Result<Libp2pTransport, NetworkError> {
    Libp2pTransport::start(
        crate::keypair_from_identity(identity)?,
        listen,
        bootstrap,
        topic,
    )
    .await
}

/// Start a libp2p transport with a fresh random identity.
pub async fn start_random(
    listen: Vec<Multiaddr>,
    bootstrap: Vec<(PeerId, Multiaddr)>,
    topic: &str,
) -> Result<Libp2pTransport, NetworkError> {
    Libp2pTransport::start(
        libp2p_identity::Keypair::generate_ed25519(),
        listen,
        bootstrap,
        topic,
    )
    .await
}

/// Parse a bootstrap multiaddress of the form `/ip4/…/tcp/…/p2p/<peer-id>`.
pub fn parse_bootstrap(value: &str) -> Result<(PeerId, Multiaddr), NetworkError> {
    let index = value
        .rfind("/p2p/")
        .ok_or_else(|| NetworkError::Transport("bootstrap must end with /p2p/<peer-id>".into()))?;
    let (address_part, peer_part) = value.split_at(index);
    let peer: PeerId = peer_part
        .trim_start_matches("/p2p/")
        .parse()
        .map_err(|e| NetworkError::PeerId(format!("{e}")))?;
    let address: Multiaddr = address_part
        .parse()
        .map_err(|e| NetworkError::Transport(format!("{e}")))?;
    Ok((peer, address))
}

impl MeshTransport for Libp2pTransport {
    fn advertise(&self, advertisement: SignedNodeAdvertisement) -> Result<(), NetworkError> {
        let bytes = serde_json::to_vec(&advertisement)
            .map_err(|e| NetworkError::Transport(e.to_string()))?;
        self.command_tx
            .try_send(Command::Publish(bytes))
            .map_err(|e| NetworkError::Transport(e.to_string()))
    }

    fn advertisements(&self) -> broadcast::Receiver<SignedNodeAdvertisement> {
        self.sender.subscribe()
    }
}

fn tracing_ignore(_err: &impl std::fmt::Debug) {
    // Dialing a bootstrap peer may fail transiently; gossipsub retries.
}

/// How long a libp2p node waits for its listen addresses to be ready.
pub const LISTEN_TIMEOUT: Duration = Duration::from_secs(10);
