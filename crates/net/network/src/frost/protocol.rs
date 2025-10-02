#![allow(unreachable_pub)]
use futures::{FutureExt, Stream, StreamExt};
use reth_eth_wire::{
    capability::SharedCapabilities, multiplex::ProtocolConnection, protocol::Protocol,
};
use reth_network_api::Direction;
use reth_network_peers::PeerId;
use alloy_primitives::bytes::BytesMut;
use std::{
    net::SocketAddr,
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::PollSender;
use tracing::{debug, error, info, warn};

use crate::{
    frost::{
        messages::{DkgRequest, WalletStateRequest},
        DkgResponse, SigningEventResponseType, SigningResponse, WalletStateResponse,
    },
    protocol::{ConnectionHandler, OnNotSupported, ProtocolHandler},
};

use super::{
    messages::{FrostProtoMessage, FrostProtoMessageKind, SignRequest},
    ConnectionEstablishedStatus, FrostPeerCommand, FrostProtocolEvent, PeerMessageResponse,
};

/// Frost Protocol Handler
#[derive(Debug)]
pub struct FrostProtoHandler {
    /// My peer id
    pub my_peer_id: PeerId,
    /// Channel to send protocol events to the manager (Conn established/confirmed), peer message
    /// command
    pub protocol_events_tx: mpsc::Sender<FrostProtocolEvent>,
}

impl ProtocolHandler for FrostProtoHandler {
    type ConnectionHandler = FrostConnectionHandler;

    /// Invoked when a new incoming connection from the remote is requested
    ///
    /// If protocols for this outgoing should be announced to the remote, return a connection
    /// handler.
    fn on_incoming(&self, _socket_addr: SocketAddr) -> Option<Self::ConnectionHandler> {
        // once the other side establishes conn with us, clone and send the sender half to them
        Some(FrostConnectionHandler {
            my_peer_id: self.my_peer_id,
            protocol_events_tx: self.protocol_events_tx.clone(),
        })
    }

    /// Invoked when a new outgoing connection to the remote is requested.
    ///
    /// If protocols for this outgoing should be announced to the remote, return a connection
    /// handler.
    fn on_outgoing(
        &self,
        _socket_addr: SocketAddr,
        _peer_id: PeerId,
    ) -> Option<Self::ConnectionHandler> {
        // once I establish conn with the other peer, clone and send the sender half to them
        Some(FrostConnectionHandler {
            my_peer_id: self.my_peer_id,
            protocol_events_tx: self.protocol_events_tx.clone(),
        })
    }
}

/// Frost Connection Handler
#[derive(Debug)]
pub struct FrostConnectionHandler {
    /// My peer id
    my_peer_id: PeerId,
    /// Channel to send protocol events to the manager (Conn established/confirmed), peer message
    /// command
    protocol_events_tx: mpsc::Sender<FrostProtocolEvent>,
}

impl ConnectionHandler for FrostConnectionHandler {
    type Connection = FrostProtoConnection;

    /// Returns the protocol to announce when the `RLPx` connection will be established.
    ///
    /// This will be negotiated with the remote peer.
    fn protocol(&self) -> Protocol {
        FrostProtoMessage::protocol()
    }

    /// Invoked when the `RLPx` connection has been established by the peer does not share the
    /// protocol.
    fn on_unsupported_by_peer(
        self,
        _supported: &SharedCapabilities,
        _direction: Direction,
        _peer_id: PeerId,
    ) -> OnNotSupported {
        OnNotSupported::KeepAlive
    }

    /// Invoked when the `RLPx` connection was established.
    ///
    /// The returned future should resolve when the connection should disconnect.
    fn into_connection(
        self,
        direction: Direction,
        peer_id: PeerId,
        conn: ProtocolConnection,
    ) -> Self::Connection {
        info!(target: "network::frost::protocol::into_connection", "Establishing connection with peer with id = {:?}, direction = {:?}", peer_id, direction);

        let protocol_events_tx = PollSender::new(self.protocol_events_tx.clone());
        // update connection state
        FrostProtoConnection {
            protocol_events_tx,
            registration: RegistrationState::NotRegistered,
            conn_rx: conn,
            // Used to receive commands from me to be sent to the other peer.
            // Initialized once the connection can be registered.
            commands_rx: None,
            pending_pong: None, // when the conn. is just established, there is no pending pong
            my_peer_id: self.my_peer_id,
            peer_id,
            direction,
        }
    }
}

/// Frost Protocol Connection
#[derive(Debug)]
pub struct FrostProtoConnection {
    /// Channel to send protocol events to the manager (Conn established/confirmed), peer message
    /// command
    protocol_events_tx: PollSender<FrostProtocolEvent>,
    /// Connection registration state.
    registration: RegistrationState,
    /// Channel to receive messages from other peers on the wire
    conn_rx: ProtocolConnection,
    /// Channel to receive commands from in the internal application to send to the other peers
    commands_rx: Option<UnboundedReceiverStream<FrostPeerCommand>>,
    /// My peer id
    my_peer_id: PeerId,
    /// Remote peer id
    peer_id: PeerId,
    /// direction of the connection
    direction: Direction,
    pending_pong: Option<oneshot::Sender<String>>,
}

#[derive(Debug)]
enum RegistrationState {
    NotRegistered,
    Pending {
        remote_peer_rx: mpsc::UnboundedReceiver<FrostPeerCommand>,
        callback_rx: oneshot::Receiver<ConnectionEstablishedStatus>,
    },
    Registered(u64),
}

impl FrostProtoConnection {
    fn reservation_guard(&self) -> SlotReservationGuard {
        SlotReservationGuard { protocol_events_tx: self.protocol_events_tx.clone() }
    }
}

/// Guard to ensure that the reserved slot in the events channel is released
/// once dropped.
struct SlotReservationGuard {
    protocol_events_tx: PollSender<FrostProtocolEvent>,
}

impl Drop for SlotReservationGuard {
    fn drop(&mut self) {
        self.protocol_events_tx.abort_send();
    }
}

impl Drop for FrostProtoConnection {
    fn drop(&mut self) {
        info!(target: "network::frost::protocol", "Dropping FrostProtoConnection for peer with id = {:?}", self.peer_id);

        let RegistrationState::Registered(idx) = self.registration else {
            // connection hasn't been registered yet
            return;
        };

        let event = FrostProtocolEvent::ConnectionClosed { idx };

        // try to let the FROST manager know about the connection termination so
        // the cleanup can be triggered. This can fail if the event channel is
        // full - the FROST manager does still have an implicit garbage
        // collection mechanism that will remove the dead connection after a
        // while.
        let res = self
            .protocol_events_tx
            .get_ref()
            .expect("poll sender closed unexpectedly")
            .try_send(event);

        if res.is_err() {
            warn!(target: "network::frost::protocol", "Failed to notify FROST manager about connection drop for peer with id = {:?}, connection idx = {}", self.peer_id, idx);
        }
    }
}

impl Stream for FrostProtoConnection {
    type Item = BytesMut;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // on every new connection to us, we send an Established event to the FROST manager
        // and a tx handle to send Command messages to us directly.
        match &mut this.registration {
            RegistrationState::NotRegistered => {
                // reserve a slot for the connection registration event.
                if let Err(e) = ready!(this.protocol_events_tx.poll_reserve(cx)) {
                    // this error only occurs if the FROST manager has been dropped.
                    error!(target: "network::frost::protocol", "Failed to reserve slot in the events channel: {:?}", e);
                    return Poll::Ready(None);
                }

                let (remote_peer_tx, remote_peer_rx) = mpsc::unbounded_channel();
                let (callback_tx, mut callback_rx) = oneshot::channel();

                let connection_established_event = FrostProtocolEvent::ConnectionEstablished {
                    direction: this.direction,
                    peer_id: this.peer_id,
                    peer_commands_tx: remote_peer_tx,
                    sender: callback_tx,
                };

                if let Err(e) = this.protocol_events_tx.send_item(connection_established_event) {
                    // this error only occurs if the FROST manager has been dropped.
                    error!(target: "network::frost::protocol", "Failed to send ConnectionEstablished event: {:?}", e.to_string());
                    return Poll::Ready(None);
                }

                // Wait for the FROST manager to assign an idx to this connection.
                match callback_rx.poll_unpin(cx) {
                    Poll::Ready(Ok(conn_established_status)) => {
                        // connection was registered immediately and
                        // successfully (unlikely that actually happens in
                        // practice)
                        match conn_established_status {
                            ConnectionEstablishedStatus::Success(idx) => {
                                // connection was registered successfully
                                this.registration = RegistrationState::Registered(idx);
                                this.commands_rx =
                                    Some(UnboundedReceiverStream::new(remote_peer_rx));
                            }
                            ConnectionEstablishedStatus::ClosedPeerCommandsCommunicationChannel |
                            ConnectionEstablishedStatus::NoneAuthority |
                            ConnectionEstablishedStatus::ConnectedToOurself => {
                                return Poll::Ready(None);
                            }
                        }
                    }
                    Poll::Ready(Err(e)) => {
                        // this error only occurs if the FROST manager has been dropped.
                        error!(target: "network::frost::protocol", "Failed to send ConnectionEstablished event: {:?}", e.to_string());
                        return Poll::Ready(None);
                    }
                    Poll::Pending => {
                        // stage pending state, wait for callback to be resolved
                        this.registration =
                            RegistrationState::Pending { remote_peer_rx, callback_rx };
                        return Poll::Pending;
                    }
                }
            }
            RegistrationState::Pending { .. } => {
                // take ownership of data
                let RegistrationState::Pending { remote_peer_rx, mut callback_rx } =
                    std::mem::replace(&mut this.registration, RegistrationState::NotRegistered)
                else {
                    panic!("checked above")
                };

                match callback_rx.poll_unpin(cx) {
                    Poll::Ready(Ok(conn_established_status)) => {
                        match conn_established_status {
                            ConnectionEstablishedStatus::Success(idx) => {
                                // connection was registered successfully
                                this.registration = RegistrationState::Registered(idx);
                                this.commands_rx =
                                    Some(UnboundedReceiverStream::new(remote_peer_rx));
                            }
                            ConnectionEstablishedStatus::ClosedPeerCommandsCommunicationChannel |
                            ConnectionEstablishedStatus::NoneAuthority |
                            ConnectionEstablishedStatus::ConnectedToOurself => {
                                return Poll::Ready(None);
                            }
                        }
                    }
                    Poll::Ready(Err(e)) => {
                        error!(target: "network::frost::protocol", "Failed to send ConnectionEstablished event: {:?}", e.to_string());
                        return Poll::Ready(None);
                    }
                    Poll::Pending => {
                        // stage pending state, wait for callback to be resolved
                        this.registration =
                            RegistrationState::Pending { remote_peer_rx, callback_rx };

                        return Poll::Pending;
                    }
                }
            }
            RegistrationState::Registered(_) => { /* connection is already registered */ }
        }

        let commands_rx = this.commands_rx.as_mut().expect("commands_rx is not initialized");

        // poll the commands sent by us to send to another peer
        // TODO(lamafab): should we shutdown the connection if the commands_rx is dropped?
        if let Poll::Ready(Some(cmd)) = commands_rx.poll_next_unpin(cx) {
            debug!(target: "network::frost::protocol", "Received command: {:?}", cmd);
            let resp = match cmd {
                // if I want to send a ping message, save the response channel to later (below)
                // answer once the pong is received
                FrostPeerCommand::PingMessage { msg: _, response } => {
                    this.pending_pong = Some(response);
                    FrostProtoMessage::ping_message(this.my_peer_id)
                }
                FrostPeerCommand::PeerMessage(response) => match response {
                    PeerMessageResponse::Error(e) => {
                        error!(target: "network::frost::protocol", "Received error: {:?}", e);
                        return Poll::Ready(None);
                    }
                    PeerMessageResponse::Dkg(dkg_response) => {
                        let DkgResponse { data, sender, recipient } = dkg_response;

                        let req = DkgRequest::new(data, sender, recipient);
                        FrostProtoMessage::dkg_request_message(req)
                    }
                    PeerMessageResponse::Signing(signing_response) => {
                        let SigningResponse { response_type, signing_session_id, psbt } =
                            signing_response;

                        match response_type {
                            SigningEventResponseType::SignerRound1SigningPackage => {
                                let req = SignRequest::new(signing_session_id, psbt);
                                FrostProtoMessage::round1_signer_package_message(req)
                            }
                            SigningEventResponseType::CoordinatorRound1SigningPackage => {
                                let req = SignRequest::new(signing_session_id, psbt);
                                FrostProtoMessage::round1_coordinator_signing_package_message(req)
                            }
                            SigningEventResponseType::SignerRound2SigningPackage => {
                                let req = SignRequest::new(signing_session_id, psbt);
                                FrostProtoMessage::round2_signer_package_message(req)
                            }
                            SigningEventResponseType::CoordinatorRound2SigningPackage => {
                                let req = SignRequest::new(signing_session_id, psbt);
                                FrostProtoMessage::round2_coordinator_signing_package_message(req)
                            }
                        }
                    }
                    PeerMessageResponse::WalletState(wallet_state_response) => {
                        let WalletStateResponse { uuid, finalized_pegout_ids } =
                            wallet_state_response;
                        let req = WalletStateRequest::new(&uuid, finalized_pegout_ids);
                        FrostProtoMessage::wallet_state_message(req)
                    }
                },
            };

            return Poll::Ready(Some(resp.encoded()));
        }

        // before we even start processing the incoming messages from the other
        // peer, we first make sure that the FROST manager has enough capacity
        // by reserving a slot in the bounded events channel.
        //
        // The Waker will be woken up once the event channel has enough capacity
        // again.
        if let Err(e) = ready!(this.protocol_events_tx.poll_reserve(cx)) {
            // this error only occurs if the FROST manager has been dropped.
            error!(target: "network::frost::protocol", "Failed to reserve slot in the events channel: {:?}", e);
            return Poll::Ready(None);
        }

        // this drop guard releases the reserved slot automatically when this
        // function returns (early), including for any pending states. This has
        // no effect if a message actually gets sent to the FROST manager.
        let _guard = this.reservation_guard();

        let msg;
        loop {
            // poll the actual conn to peers for events from this other peer
            let Some(bytes) = ready!(this.conn_rx.poll_next_unpin(cx)) else {
                return Poll::Ready(None);
            };

            match FrostProtoMessage::decode_message(&mut &bytes[..]) {
                Some(m) => {
                    msg = m;
                    break;
                }
                None => {
                    // drop this invalid message and continue the loop to poll the conn again.
                    warn!(target: "network::frost::protocol", "Failed to decode frost protocol message");

                    /* continue... */
                }
            }
        }

        // The frost manager will handle this message (often by forwarding it to
        // another task) and the response will be sent on command_rx for us to
        // send back to another peer.
        let protocol_event = match msg.message {
            FrostProtoMessageKind::Ping => {
                info!(target: "network::frost::protocol", "Received ping message from peer. Replying with pong...");
                return Poll::Ready(Some(FrostProtoMessage::pong().encoded()));
            }
            FrostProtoMessageKind::PingMessage(_peer_id) => {
                // answer with pong and my peer id
                info!(target: "network::frost::protocol", "Received ping message from peer. Replying with pong...");
                return Poll::Ready(Some(
                    FrostProtoMessage::pong_message(this.my_peer_id).encoded(),
                ));
            }
            FrostProtoMessageKind::Pong => {
                info!(target: "network::frost::protocol", "Received pong message from peer.");
                if let Some(sender) = this.pending_pong.take() {
                    sender.send("Confirmed".to_string()).ok();
                }
                return Poll::Pending;
            }
            // other peers answers with pong message with a peer id and authority index
            FrostProtoMessageKind::PongMessage(_peer_id) => {
                info!(target: "network::frost::protocol", "Received pong message from peer.");
                if let Some(sender) = this.pending_pong.take() {
                    sender.send("Confirmed".to_string()).ok();
                }
                return Poll::Pending;
            }
            FrostProtoMessageKind::Dkg(data) => FrostProtocolEvent::PeerMessage {
                response: PeerMessageResponse::Dkg(DkgResponse {
                    sender: data.sender,
                    recipient: data.recipient,
                    data: data.data,
                }),
                peer_id: this.peer_id,
            },
            FrostProtoMessageKind::SignerRound1SigningPackage(data) => {
                FrostProtocolEvent::PeerMessage {
                    response: PeerMessageResponse::Signing(SigningResponse {
                        response_type: SigningEventResponseType::SignerRound1SigningPackage,
                        signing_session_id: data.signing_session_id,
                        psbt: data.psbt,
                    }),
                    peer_id: this.peer_id,
                }
            }
            FrostProtoMessageKind::CoordinatorRound1SigningPackage(data) => {
                FrostProtocolEvent::PeerMessage {
                    response: PeerMessageResponse::Signing(SigningResponse {
                        response_type: SigningEventResponseType::CoordinatorRound1SigningPackage,
                        signing_session_id: data.signing_session_id,
                        psbt: data.psbt,
                    }),
                    peer_id: this.peer_id,
                }
            }
            FrostProtoMessageKind::SignerRound2SigningPackage(data) => {
                FrostProtocolEvent::PeerMessage {
                    response: PeerMessageResponse::Signing(SigningResponse {
                        response_type: SigningEventResponseType::SignerRound2SigningPackage,
                        signing_session_id: data.signing_session_id,
                        psbt: data.psbt,
                    }),
                    peer_id: this.peer_id,
                }
            }
            FrostProtoMessageKind::CoordinatorRound2SigningPackage(data) => {
                FrostProtocolEvent::PeerMessage {
                    response: PeerMessageResponse::Signing(SigningResponse {
                        response_type: SigningEventResponseType::CoordinatorRound2SigningPackage,
                        signing_session_id: data.signing_session_id,
                        psbt: data.psbt,
                    }),
                    peer_id: this.peer_id,
                }
            }
            FrostProtoMessageKind::WalletState(data) => FrostProtocolEvent::PeerMessage {
                response: PeerMessageResponse::WalletState(WalletStateResponse {
                    uuid: data.uuid,
                    finalized_pegout_ids: data.finalized_pegout_ids,
                }),
                peer_id: this.peer_id,
            },
        };

        if let Err(e) = this.protocol_events_tx.send_item(protocol_event) {
            // this error only occurs if the FROST manager has been dropped.
            error!(target: "network::frost::protocol", "Failed to send protocol event: {:?}", e.to_string());
            return Poll::Ready(None);
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use futures::task::noop_waker;

    use super::*;

    #[test]
    fn frost_proto_registration() {
        let (protocol_events_tx, mut protocol_events_rx) = mpsc::channel(3);
        let protocol_events_tx = PollSender::new(protocol_events_tx);

        let (conn_tx, conn_rx) = mpsc::unbounded_channel();
        let conn_rx = UnboundedReceiverStream::new(conn_rx);
        let conn_rx = ProtocolConnection::from(conn_rx);

        let direction = Direction::Incoming;

        let mut conn = FrostProtoConnection {
            protocol_events_tx,
            registration: RegistrationState::NotRegistered,
            conn_rx,
            commands_rx: None,
            my_peer_id: PeerId::new([0; 64]),
            peer_id: PeerId::new([1; 64]),
            direction,
            pending_pong: None,
        };

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        // Poll connection for the first time; it's pending as it needs to be registered first.
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };

        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };

        // Manager receives the connection established event.
        let msg = protocol_events_rx.try_recv().unwrap();
        let FrostProtocolEvent::ConnectionEstablished {
            direction: _,
            peer_id: _,
            peer_commands_tx: _,
            sender,
        } = msg
        else {
            panic!()
        };

        // Manager assigns the connection idx
        sender.send(ConnectionEstablishedStatus::Success(0)).unwrap();

        // Send wire message to the connection (ping)
        let wire_msg = FrostProtoMessage::ping().encoded();
        conn_tx.send(wire_msg).unwrap();

        // Connection receives wire message and generates a response (pong)
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Ready(Some(bytes)) = res else { panic!() };

        let resp = FrostProtoMessage::decode_message(&mut &bytes[..]).unwrap();
        assert_eq!(resp.message, FrostProtoMessageKind::Pong);

        // Connection is pending
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };
    }

    #[test]
    fn frost_proto_backlog() {
        let (protocol_events_tx, mut protocol_events_rx) = mpsc::channel(3);
        let protocol_events_tx = PollSender::new(protocol_events_tx);

        let (conn_tx, conn_rx) = mpsc::unbounded_channel();
        let conn_rx = UnboundedReceiverStream::new(conn_rx);
        let conn_rx = ProtocolConnection::from(conn_rx);

        let direction = Direction::Incoming;

        let mut conn = FrostProtoConnection {
            protocol_events_tx,
            registration: RegistrationState::NotRegistered,
            conn_rx,
            commands_rx: None,
            my_peer_id: PeerId::new([0; 64]),
            peer_id: PeerId::new([1; 64]),
            direction,
            pending_pong: None,
        };

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        // Send wire messages to the connection
        for _ in 0..4 {
            let wire_msg = FrostProtoMessage::ping().encoded();
            conn_tx.send(wire_msg).unwrap();
        }

        // Poll connection for the first time; it's pending as it needs to be
        // registered first.
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };

        // Manager receives the connection established event.
        let msg = protocol_events_rx.try_recv().unwrap();
        let FrostProtocolEvent::ConnectionEstablished {
            direction: _,
            peer_id: _,
            peer_commands_tx: _,
            sender,
        } = msg
        else {
            panic!()
        };

        // Manager assigns the connection idx
        sender.send(ConnectionEstablishedStatus::Success(0)).unwrap();

        // Connection receives all wire messages and generates a response each (pong)
        for _ in 0..4 {
            let res = conn.poll_next_unpin(&mut cx);
            let Poll::Ready(Some(bytes)) = res else { panic!() };
            let resp = FrostProtoMessage::decode_message(&mut &bytes[..]).unwrap();
            assert_eq!(resp.message, FrostProtoMessageKind::Pong);
        }

        // Connection is pending
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };
    }

    #[test]
    fn frost_proto_backpressure() {
        let (protocol_events_tx, mut protocol_events_rx) = mpsc::channel(3);
        let protocol_events_tx = PollSender::new(protocol_events_tx);

        let (conn_tx, conn_rx) = mpsc::unbounded_channel();
        let conn_rx = UnboundedReceiverStream::new(conn_rx);
        let conn_rx = ProtocolConnection::from(conn_rx);

        let direction = Direction::Incoming;

        let mut conn = FrostProtoConnection {
            protocol_events_tx,
            registration: RegistrationState::NotRegistered,
            conn_rx,
            commands_rx: None,
            my_peer_id: PeerId::new([0; 64]),
            peer_id: PeerId::new([1; 64]),
            direction,
            pending_pong: None,
        };

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        let req = WalletStateRequest {
            version: 1,
            uuid: "uuid-1".to_string(),
            finalized_pegout_ids: vec![1, 2, 3],
        };

        // Send wire messages to the connection that create events for the manager
        for _ in 0..4 {
            let wire_msg = FrostProtoMessage::wallet_state_message(req.clone()).encoded();
            conn_tx.send(wire_msg).unwrap();
        }

        // Poll connection for the first time; it's pending as it needs to be
        // registered first.
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };

        // Manager receives the connection established event.
        let msg = protocol_events_rx.try_recv().unwrap();
        let FrostProtocolEvent::ConnectionEstablished {
            direction: _,
            peer_id: _,
            peer_commands_tx: _,
            sender,
        } = msg
        else {
            panic!()
        };

        // Manager assigns the connection idx
        sender.send(ConnectionEstablishedStatus::Success(0)).unwrap();

        let event_queue = protocol_events_rx.len();
        assert_eq!(event_queue, 0); // max=3

        // Connection is pending; no response generated, but events were pushed
        // to the queue for the manager
        for _ in 0..3 {
            let res = conn.poll_next_unpin(&mut cx);
            let Poll::Pending = res else { panic!() };
        }

        // Event queue is now full
        let event_queue = protocol_events_rx.len();
        assert_eq!(event_queue, 3); // max=3

        // Connection is pending
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };

        // Manager processes two events
        for _ in 0..2 {
            let event = protocol_events_rx.try_recv().unwrap();
            let FrostProtocolEvent::PeerMessage { peer_id: _, response } = event else { panic!() };
            let PeerMessageResponse::WalletState(_response) = response else { panic!() };
        }

        // We now have enough space for the last two events
        let event_queue = protocol_events_rx.len();
        assert_eq!(event_queue, 1); // max=3

        // Process the last message
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };

        let event_queue = protocol_events_rx.len();
        assert_eq!(event_queue, 2); // max=3

        // Manager processes the last two events
        for _ in 0..2 {
            let event = protocol_events_rx.try_recv().unwrap();
            let FrostProtocolEvent::PeerMessage { peer_id: _, response } = event else { panic!() };
            let PeerMessageResponse::WalletState(_response) = response else { panic!() };
        }

        // Event queue is now empty
        let event_queue = protocol_events_rx.len();
        assert_eq!(event_queue, 0); // max=3

        // Connection is pending
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };

        assert!(protocol_events_rx.try_recv().is_err());
    }

    #[test]
    fn frost_proto_conn_established_violation() {
        let (protocol_events_tx, mut protocol_events_rx) = mpsc::channel(3);
        let protocol_events_tx = PollSender::new(protocol_events_tx);

        let (_conn_tx, conn_rx) = mpsc::unbounded_channel();
        let conn_rx = UnboundedReceiverStream::new(conn_rx);
        let conn_rx = ProtocolConnection::from(conn_rx);

        let direction = Direction::Incoming;

        let mut conn = FrostProtoConnection {
            protocol_events_tx,
            registration: RegistrationState::NotRegistered,
            conn_rx,
            commands_rx: None,
            my_peer_id: PeerId::new([0; 64]),
            peer_id: PeerId::new([1; 64]),
            direction,
            pending_pong: None,
        };

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        // Poll connection for the first time; it's pending as it needs to be registered first.
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };

        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };

        // Manager receives the connection established event.
        let msg = protocol_events_rx.try_recv().unwrap();
        let FrostProtocolEvent::ConnectionEstablished {
            direction: _,
            peer_id: _,
            peer_commands_tx: _,
            sender,
        } = msg
        else {
            panic!()
        };

        // Connection is pending at this point
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Pending = res else { panic!() };

        // Manager reports a none authority connection attempt
        sender.send(ConnectionEstablishedStatus::NoneAuthority).unwrap();

        // Connection has to be closed and no longer pending here
        let res = conn.poll_next_unpin(&mut cx);
        let Poll::Ready(None) = res else { panic!() };
    }
}
