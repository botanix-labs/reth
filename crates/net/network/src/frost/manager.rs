use super::{FrostPeerCommand, FrostProtocolEvent, PeerMessageResponse, WalletStateResponse};
use crate::{
    frost::{ConnectionEstablishedStatus, PeerMessageStatus},
    session::Direction,
    NetworkHandle,
};
use frost_secp256k1_tr as frost;
use futures::{Future, StreamExt};
use reth_network_peers::PeerId;
use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::{
    mpsc::{self, error::SendError, UnboundedSender},
    oneshot,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, info, warn};

/// Trait for sending commands to the [`FrostManager`]
/// Trait was created mainly for the convenience of mocking during testing
pub trait ToFrostManager {
    /// Sends a command to the Protocol
    fn send_command(&self, cmd: FrostCommand) -> Result<(), SendError<FrostCommand>>;
}

/// Frost Handle for communication with the protocol
#[derive(Clone, Debug)]
pub struct FrostHandle {
    manager_tx: mpsc::UnboundedSender<FrostCommand>,
}

/// Implementations for the [`FrostHandle`]
impl ToFrostManager for FrostHandle {
    /// Sends a command to the Protocol
    fn send_command(&self, cmd: FrostCommand) -> Result<(), SendError<FrostCommand>> {
        self.manager_tx.send(cmd)
    }
}

/// Structure that contains a valid connection to the peer
#[derive(Debug, Clone)]
pub struct PeerData {
    /// peer id
    pub peer_id: PeerId,
    /// channel use for sending commands to the peer
    pub peer_commands_tx: UnboundedSender<FrostPeerCommand>,
    /// in or outgoing connection
    // TODO(lamafab): this is not really used anywhere.
    pub direction: Direction,
    /// the frost identifier of the peer
    pub frost_identifier: frost::Identifier,
}

/// Context for forwarding peer messages to the frost task
#[derive(Debug, Clone)]
pub struct PeerMessageContext {
    /// the peer id
    pub peer_id: PeerId,
    /// frost identifier
    pub frost_identifier: Option<frost::Identifier>,
    /// The message itself
    pub message: PeerMessageResponse,
}

/// Context for an authority peer, containing all active connection Ids and the
/// frost identifier.
#[derive(Debug)]
struct AuthorityContext {
    connections: HashSet<ConnIdx>,
    /// the frost identifier of the peer
    frost_identifier: frost::Identifier,
}

type ConnIdx = u64;

/// An active connection to a peer.
#[derive(Debug)]
struct Connection {
    /// peer id
    peer_id: PeerId,
    /// channel use for sending commands to the peer
    peer_commands_tx: UnboundedSender<FrostPeerCommand>,
    /// in or outgoing connection
    direction: Direction,
}

/// Frost Manager implementation
#[derive(Debug)]
pub struct FrostManager {
    /// Network access.
    network: NetworkHandle,
    /// Subscriptions to all network related events.
    ///
    /// From which we get all new incoming transaction related messages.
    from_network: UnboundedReceiverStream<FrostProtocolEvent>,
    /// Copy of the sender half, so new [`FrostManager`] can be created on demand.
    command_tx: mpsc::UnboundedSender<FrostCommand>,
    /// Receiver half of the command channel.
    command_rx: UnboundedReceiverStream<FrostCommand>,
    /// Counter for connection indices.
    connection_counter: ConnIdx,
    /// total authorities to connect to, including ourselves
    authorities: HashMap<PeerId, AuthorityContext>,
    /// All the connected peers.
    peer_connections: HashMap<ConnIdx, Connection>,
    /// Forwards for message to the frost task
    task_forwarder_txs: Vec<mpsc::UnboundedSender<PeerMessageContext>>,
}

impl FrostManager {
    /// Create a new [`FrostManager`] instance with the given config
    pub fn new(
        config: FrostConfig,
        network: NetworkHandle,
        from_network: mpsc::UnboundedReceiver<FrostProtocolEvent>,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        // Prepare the authorities with their respective FROST identifiers and
        // connection trackers.
        let authorities = config
            .authorities
            .iter()
            .enumerate()
            .map(|(index, pk)| {
                let frost_identifier = authority_index_to_frost_identifier(index as u16);
                let peer_id = PeerId::from_slice(&pk.serialize_uncompressed()[1..]);
                let authority = AuthorityContext { connections: HashSet::new(), frost_identifier };

                (peer_id, authority)
            })
            .collect();

        Self {
            command_tx,
            command_rx: UnboundedReceiverStream::new(command_rx),
            network,
            from_network: UnboundedReceiverStream::new(from_network),
            connection_counter: 0,
            peer_connections: HashMap::default(),
            authorities,
            task_forwarder_txs: Vec::new(),
        }
    }

    /// Retrieve an arbitrary active connection to the given peer.
    fn retrieve_peer_data(&mut self, peer_id: &PeerId) -> Option<PeerData> {
        let authority = self.authorities.get_mut(peer_id)?;

        let mut to_remove = vec![];
        let mut peer_data = None;

        // we retrieve any connection to the given peer in arbitrary order. This
        // also acts as implicit garbage collection; if we encounter a dead
        // connection - which can happen if the event channel is choking and the
        // cleanup mechanism failed to trigger - the dead connection will be
        // removed. It's not expected that *all* dead connections are removed
        // here.
        //
        // It's very unlikely that such a scenario actually ever happens in
        // practice.
        for idx in &authority.connections {
            let conn = self.peer_connections.get_mut(idx).expect("registered idx not found");

            if conn.peer_commands_tx.is_closed() {
                warn!(
                    target: "network::frost::retrieve_peer_data",
                    "Dead connection detected and preparing for removal, from peer with id {:?}, conn idx = {}",
                    peer_id, idx
                );

                to_remove.push(*idx);

                // try next connection
                continue;
            }

            peer_data = Some(PeerData {
                peer_id: *peer_id,
                peer_commands_tx: conn.peer_commands_tx.clone(),
                direction: conn.direction,
                frost_identifier: authority.frost_identifier,
            });

            break;
        }

        // if dead connections have been found, remove them.
        for idx in to_remove {
            self.peer_connections.remove(&idx);
            authority.connections.remove(&idx);
        }

        peer_data
    }

    fn all_authority_peers_connected(&mut self) -> bool {
        let peer_ids: Vec<_> = self.authorities.keys().cloned().collect();
        // filter all peers with active connections
        let connected =
            peer_ids.iter().filter_map(|peer_id| self.retrieve_peer_data(peer_id)).count();

        info!(
            target: "network::frost::all_authority_peers_connected",
            "Number of peer connections: {:?}", connected
        );

        connected == self.authorities.len() - 1
    }

    /// Returns a new [`FrostHandle`] that can send commands to this type.
    pub fn handle(&self) -> FrostHandle {
        FrostHandle { manager_tx: self.command_tx.clone() }
    }

    fn is_authority_peer(&self, peer_id: &PeerId) -> bool {
        self.authorities.contains_key(peer_id)
    }

    fn respond_with_status(
        sender: tokio::sync::oneshot::Sender<ConnectionEstablishedStatus>,
        status: ConnectionEstablishedStatus,
        message: &str,
        peer_id: &PeerId,
    ) -> bool {
        if sender.is_closed() {
            return false;
        }
        let msg = format!(
            "Received FrostProtocolEvent::ConnectionEstablished event from peer {:?}. Error: {:?}",
            peer_id, message
        );
        if sender.send(status).is_err() {
            warn!(
                target: "network::frost::on_network_event",
                msg
            );
            return false;
        }
        true
    }

    fn on_network_event(&mut self, protocol_event: FrostProtocolEvent) {
        match protocol_event {
            FrostProtocolEvent::ConnectionEstablished {
                direction,
                peer_id,
                peer_commands_tx,
                sender,
            } => {
                info!(
                    target: "network::frost::on_network_event",
                    "Received FrostProtocolEvent::ConnectionEstablished event from peer with id = {:?}, direction = {:?}, connection channel = {:?}",
                    peer_id, direction, peer_commands_tx
                );
                if !self.is_authority_peer(&peer_id) {
                    warn!(
                        target: "network::frost::on_network_event",
                        "Received FrostProtocolEvent::ConnectionEstablished event from non-authority peer {:?}, protocol_event",
                        peer_id
                    );
                    Self::respond_with_status(
                        sender,
                        ConnectionEstablishedStatus::NoneAuthority,
                        "None authority peer",
                        &peer_id,
                    );
                    return;
                }

                // make sure we ignore our own connection
                if *self.network.peer_id() == peer_id {
                    warn!(
                        target: "network::frost::on_network_event",
                        "Received FrostProtocolEvent::ConnectionEstablished event from our own peer {:?}",
                        peer_id
                    );
                    Self::respond_with_status(
                        sender,
                        ConnectionEstablishedStatus::ConnectedToOurself,
                        "Event from our own peer",
                        &peer_id,
                    );
                    return;
                }

                if peer_commands_tx.is_closed() {
                    warn!(
                        target: "network::frost::on_network_event",
                        "Received FrostProtocolEvent::ConnectionEstablished event from peer with id = {:?}, but the connection channel is already closed",
                        peer_id
                    );
                    Self::respond_with_status(
                        sender,
                        ConnectionEstablishedStatus::ClosedPeerCommandsCommunicationChannel,
                        "Peer commands communication channel is already closed",
                        &peer_id,
                    );
                    return;
                }

                // Assign a unique index to the connection and increment the counter.
                //
                // NOTE: if this ever reaches 2^64-1, which is very unlikely,
                // we just wrap around and start the counter at 0 again.
                let idx = self.connection_counter;
                self.connection_counter = idx.wrapping_add(1);

                // send the assigned idx back to the initiator
                if !Self::respond_with_status(
                    sender,
                    ConnectionEstablishedStatus::Success(idx),
                    "Connection channel is already closed",
                    &peer_id,
                ) {
                    return;
                }

                let conn = Connection { peer_id, peer_commands_tx, direction };

                self.authorities.get_mut(&peer_id).expect("checked above").connections.insert(idx);
                self.peer_connections.insert(idx, conn);
            }
            FrostProtocolEvent::ConnectionClosed { idx } => {
                let Some(conn) = self.peer_connections.get(&idx) else {
                    warn!(
                        target: "network::frost::on_network_event",
                        "Received FrostProtocolEvent::ConnectionClosed event from an unknown connection, idx = {}",
                        idx
                    );
                    return;
                };

                let did_remove = self
                    .authorities
                    .get_mut(&conn.peer_id)
                    .expect("authority not found")
                    .connections
                    .remove(&idx);

                debug_assert!(did_remove);
            }
            FrostProtocolEvent::PeerMessage { peer_id, response } => {
                info!(
                    target: "network::frost::on_network_event",
                    "Received FrostProtocolEvent::PeerMessage message from peer with id = {:?}, response = {:?}",
                    peer_id, response
                );

                let err_auth_peer = if !self.is_authority_peer(&peer_id) {
                    warn!(
                        target: "network::frost::on_network_event",
                        "Received FrostProtocolEvent::PeerMessage event from non-authority peer {:?}, protocol_event",
                        peer_id
                    );
                    Some(PeerMessageResponse::Error(PeerMessageStatus::NoneAuthority(peer_id)))
                } else {
                    None
                };

                // Check if the peer is in the peer connections. i.e. we have a valid connection to
                // the peer. It may be fine to address this message on the
                // application level, but then we cannot respond
                let (peer_data, err_peer_not_found) = match self.retrieve_peer_data(&peer_id) {
                    Some(peer_data) => (Some(peer_data), None),
                    None => {
                        warn!(
                            target: "network::frost::on_network_event",
                            "Received FrostProtocolEvent::PeerMessage message from peer with id = {:?}, but the peer has no active connections",
                            peer_id
                        );
                        (
                            None,
                            Some(PeerMessageResponse::Error(PeerMessageStatus::PeerIdNotFound(
                                peer_id,
                            ))),
                        )
                    }
                };

                let err = err_auth_peer.or(err_peer_not_found);

                for task_forwarder in &self.task_forwarder_txs {
                    if let Err(send_res) = task_forwarder.send(PeerMessageContext {
                        peer_id,
                        frost_identifier: peer_data
                            .clone()
                            .map(|peer_data| peer_data.frost_identifier),
                        message: err.clone().unwrap_or_else(|| response.clone()),
                    }) {
                        error!(
                            target: "network::frost::on_network_event",
                            "Received FrostProtocolEvent::PeerMessage event from peer with id {}, but could not forward it to task. Error: {:?}",
                            peer_id, send_res
                        );
                    }
                }
            }
        }
    }

    /// Handles a command received from a detached [`FrostHandle`]
    fn on_command(&mut self, cmd: FrostCommand) {
        match cmd {
            FrostCommand::CheckConnectedToAll(tx) => {
                let all_connected = self.all_authority_peers_connected();

                // reply to caller
                if let Err(e) = tx.send(all_connected) {
                    error!(
                        target: "network::frost::on_command",
                        "Error replying to call on CheckConnectedToAll {:?}", e
                    );
                }
            }
            FrostCommand::GetAllConnectedPeers(tx) => {
                let peer_ids: Vec<_> = self.authorities.keys().cloned().collect();
                let peer_connections: HashMap<PeerId, PeerData> = peer_ids
                    .iter()
                    .filter_map(|peer_id| {
                        self.retrieve_peer_data(peer_id).map(|peer_data| (*peer_id, peer_data))
                    })
                    .collect();

                // reply to caller
                if let Err(e) = tx.send(peer_connections) {
                    error!(
                        target: "network::frost::on_command",
                        "Error replying to call on GetAllConnectedPeers {:?}", e
                    );
                }
            }
            FrostCommand::GetPeerMessagesStream(tx) => {
                // create channel whereby keeping the sender half and sending to the caller the
                // receiver
                let (task_forwarder_txs, frost_task_forwarder_rx) =
                    mpsc::unbounded_channel::<PeerMessageContext>();

                self.task_forwarder_txs.push(task_forwarder_txs);

                // reply to caller
                if let Err(e) = tx.send(frost_task_forwarder_rx) {
                    error!(
                        target: "network::frost::on_command",
                        "Error replying to call on GetPeerMessagesStream {:?}", e
                    );
                }
            }
            FrostCommand::GetWalletStateFromPeer(uuid) => {
                let peer_ids: Vec<_> = self.authorities.keys().cloned().collect();
                // filter all peers with active connections
                let connected_peers =
                    peer_ids.iter().filter_map(|peer_id| self.retrieve_peer_data(peer_id));

                for peer in connected_peers {
                    match peer.peer_commands_tx.send(FrostPeerCommand::PeerMessage(
                        PeerMessageResponse::WalletState(WalletStateResponse {
                            uuid: uuid.to_string(),
                            finalized_pegout_ids: vec![],
                        }),
                    )) {
                        Ok(_) => {
                            tracing::debug!(
                                target: "network::frost::on_command",
                                "Request for finalized pegout ids sent to peer {:?}", peer.peer_id
                            );
                        }
                        Err(e) => {
                            error!(target: "network::frost::on_command", "Failed to send finalized pegout ids request to peer {:?}, error: {:?}", peer.peer_id, e);
                        }
                    }
                }
            }
        }
    }
}

/// An endless future. Preemption ensure that future is non-blocking, nonetheless. See
/// [`crate::NetworkManager`] for more context on the design pattern.
///
/// This should be spawned or used as part of `tokio::select!`.
impl Future for FrostManager {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            match this.from_network.poll_next_unpin(cx) {
                Poll::Pending => break,
                Poll::Ready(None) => {
                    // This is only possible if the channel was deliberately closed since we always
                    // have an instance of `NetworkHandle`
                    error!(target: "network::frost::poll", "Network message channel closed.");
                    panic!("Network message channel closed.");
                }
                Poll::Ready(Some(event)) => {
                    this.on_network_event(event);
                }
            };
        }

        loop {
            match this.command_rx.poll_next_unpin(cx) {
                Poll::Pending => break,
                Poll::Ready(None) => {
                    // This is only possible if the channel was deliberately closed since we always
                    // have an instance of `NetworkHandle`
                    error!(target: "network::frost::poll", "Network message channel closed.");
                    panic!("Network command rx message channel closed.");
                }
                Poll::Ready(Some(cmd)) => this.on_command(cmd),
            };
        }
        Poll::Pending
    }
}

/// Commands the [`FrostManager`] listens for.
#[derive(Debug)]
pub enum FrostCommand {
    /// Check if connection to all federated peers is established
    CheckConnectedToAll(oneshot::Sender<bool>),
    /// Get the readily connected peers
    GetAllConnectedPeers(oneshot::Sender<HashMap<PeerId, PeerData>>),
    /// Get a receiver for streaming peer messages
    GetPeerMessagesStream(oneshot::Sender<mpsc::UnboundedReceiver<PeerMessageContext>>),
    /// Get pending pegouts state from peer
    GetWalletStateFromPeer(uuid::Uuid),
}

/// Config type for initiating a [`FrostManager`] instance.
#[derive(Clone, Debug)]
pub struct FrostConfig {
    /// Authority public key of the current peer participating in frost
    pub authority_pk: secp256k1::PublicKey,
    /// Authority index of the current peer participating in frost
    pub authority_index: usize,
    /// Total number of authorities participating in frost
    pub authorities: Vec<secp256k1::PublicKey>,
    /// Minimum number of signers required to participate in frost
    pub min_signers: u16,
    /// Maximum number of signers required to participate in frost
    pub max_signers: u16,
    /// Size of chunks for wallet state sync
    pub wallet_state_sync_chunk_size: u64,
}

impl FrostConfig {
    /// Create a new [`FrostConfig`] with default values
    pub const fn new(
        authority_pk: secp256k1::PublicKey,
        authority_index: usize,
        authorities: Vec<secp256k1::PublicKey>,
        min_signers: u16,
        max_signers: u16,
        wallet_state_sync_chunk_size: u64,
    ) -> Self {
        Self {
            authority_pk,
            authority_index,
            authorities,
            min_signers,
            max_signers,
            wallet_state_sync_chunk_size,
        }
    }

    /// Sets the authority public key
    pub fn set_authority_pk(&mut self, authority_pk: secp256k1::PublicKey) {
        self.authority_pk = authority_pk;
    }

    /// Sets the authority index
    pub fn set_authority_index(&mut self, authority_index: usize) {
        self.authority_index = authority_index;
    }

    /// Sets total authorities
    pub fn set_authorities(&mut self, authorities: Vec<secp256k1::PublicKey>) {
        self.authorities = authorities;
    }

    /// Sets minimum signers
    pub fn set_min_signers(&mut self, min_signers: u16) {
        self.min_signers = min_signers;
    }

    /// Sets maximum signers
    pub fn set_max_signers(&mut self, max_signers: u16) {
        self.max_signers = max_signers;
    }
}

// TODO(armins): import btcserverlib::frost_id and use on the callers
/// Maps an authority index to a frost specific identifier
/// Indices start at 0, so we add 1 to the index to get the correct identifier
/// As 0 is not a valid identifier
pub fn authority_index_to_frost_identifier(authority_index: u16) -> frost::Identifier {
    frost::Identifier::derive(authority_index.to_le_bytes().as_slice())
        .expect("can derive identifier")
}
