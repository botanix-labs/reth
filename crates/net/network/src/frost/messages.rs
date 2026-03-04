#![allow(unreachable_pub)]
use core::fmt;
use std::str::FromStr;

use reth_eth_wire::{protocol::Protocol, Capability};
use reth_network_peers::PeerId;
use alloy_primitives::bytes::{Buf, BufMut, BytesMut};

const MESSAGE_VERSION: usize = 0;
const WALLET_STATE_MESSAGE_VERSION: usize = 0;

/// A structured frost DKG message
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DkgRequest {
    /// The version of the message
    pub version: u16,
    /// Frost sender
    pub sender: Vec<u8>,
    /// Frost recipient
    pub recipient: Vec<u8>,
    /// Frost data
    pub data: Vec<u8>,
    /// Multisig Id for which the message is intended
    pub multisig_id: u32,
}

impl DkgRequest {
    /// Constructs a new DKG Request using a frost identifier and a data payload.
    pub const fn new(data: Vec<u8>, sender: Vec<u8>, recipient: Vec<u8>, multisig_id: u32) -> Self {
        Self { version: MESSAGE_VERSION as u16, data, sender, recipient, multisig_id }
    }
}

/// A structured frost sign message
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignRequest {
    /// The version of the message
    pub version: u16,
    /// Signing session id
    pub signing_session_id: Vec<u8>,
    /// Frost data
    pub psbt: Vec<u8>,
    /// Multisig Id for which the message is intended
    pub multisig_id: u32,
}

impl SignRequest {
    /// Constructs a new sign Request using a frost identifier, signing session id and a psbt
    /// payload.
    pub const fn new(signing_session_id: Vec<u8>, psbt: Vec<u8>, multisig_id: u32) -> Self {
        Self { version: MESSAGE_VERSION as u16, signing_session_id, psbt, multisig_id }
    }
}

/// A structured wallet state sync request message
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalletStateRequest {
    /// uuid of the wallet state sync request
    pub uuid: String,
    /// The version of the message
    pub version: u16,
    /// finalized pegout ids
    pub finalized_pegout_ids: Vec<u8>,
    /// Multisig Id for which the message is intended
    pub multisig_id: u32,
}

impl fmt::Display for WalletStateRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "WalletStateRequest:\n\
            - Pegout Ids: {} bytes",
            self.finalized_pegout_ids.len()
        )
    }
}

impl WalletStateRequest {
    /// Constructs a new wallet state request using a data payload.
    pub fn new(uuid: String, finalized_pegout_ids: Vec<u8>, multisig_id: u32) -> Self {
        Self {
            version: MESSAGE_VERSION as u16,
            finalized_pegout_ids,
            uuid,
            multisig_id,
        }
    }
}

/// Enum defining the frost message type as u8
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrostProtoMessageId {
    /// Dkg package
    Dkg = 0x00,
    /// Ping
    Ping = 0x02,
    /// Pong
    Pong = 0x03,
    /// Ping message with a user-defined message
    PingMessage = 0x04,
    /// Pong message with a user-defined message
    PongMessage = 0x05,
    /// Signers will add their signing commitments to the psbt
    SignerRound1SigningPackage = 0x06,
    /// Coordinating node will collect the PSBTs with the signing commitments
    CoordinatorRound1SigningPackage = 0x07,
    /// Signers get round 2 signing package
    SignerRound2SigningPackage = 0x08,
    /// Coordinating node will collect the PSBTs with the partial sigs
    CoordinatorRound2SigningPackage = 0x09,
    /// `WalletState`
    WalletState = 0x0B,
}

/// Enum defining the frost message kind
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrostProtoMessageKind {
    /// Dkg package
    Dkg(DkgRequest),
    /// Ping
    Ping,
    /// Pong
    Pong,
    /// Ping message with a user-defined message
    PingMessage(PeerId),
    /// Pong message with a user peer id
    PongMessage(PeerId),
    /// Signers will add their signing commitments to the psbt
    SignerRound1SigningPackage(SignRequest),
    /// Coordinating node will collect the PSBTs with the signing commitments
    CoordinatorRound1SigningPackage(SignRequest),
    /// Signers get round 2 signing package
    SignerRound2SigningPackage(SignRequest),
    /// Coordinating node will collect the PSBTs with the partial sigs
    CoordinatorRound2SigningPackage(SignRequest),
    /// Wallet state message
    WalletState(WalletStateRequest),
}

/// An protocol message, containing a message ID and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrostProtoMessage {
    /// Message Type
    pub message_type: FrostProtoMessageId,
    /// Message Content
    pub message: FrostProtoMessageKind,
}

impl FrostProtoMessage {
    /// Returns the capability for the `frost` protocol.
    pub const fn capability() -> Capability {
        Capability::new_static("frost", MESSAGE_VERSION)
    }

    /// Returns the protocol for the `frost` protocol.
    pub const fn protocol() -> Protocol {
        Protocol::new(Self::capability(), 16)
    }

    /// Creates a ping message
    pub const fn ping() -> Self {
        Self { message_type: FrostProtoMessageId::Ping, message: FrostProtoMessageKind::Ping }
    }

    /// Creates a pong message
    pub const fn pong() -> Self {
        Self { message_type: FrostProtoMessageId::Pong, message: FrostProtoMessageKind::Pong }
    }

    /// Creates a ping message
    pub const fn ping_message(peer_id: PeerId) -> Self {
        Self {
            message_type: FrostProtoMessageId::PingMessage,
            message: FrostProtoMessageKind::PingMessage(peer_id),
        }
    }
    /// Creates a pong message
    pub const fn pong_message(peer_id: PeerId) -> Self {
        Self {
            message_type: FrostProtoMessageId::PongMessage,
            message: FrostProtoMessageKind::PongMessage(peer_id),
        }
    }

    /// Creates a dkg package message
    pub const fn dkg_request_message(resource: DkgRequest) -> Self {
        Self {
            message_type: FrostProtoMessageId::Dkg,
            message: FrostProtoMessageKind::Dkg(resource),
        }
    }

    /// Signers adding their signing commitments to the psbt
    pub const fn round1_signer_package_message(resource: SignRequest) -> Self {
        Self {
            message_type: FrostProtoMessageId::SignerRound1SigningPackage,
            message: FrostProtoMessageKind::SignerRound1SigningPackage(resource),
        }
    }

    /// Coordinating node collecting the PSBTs with the signing commitments
    pub const fn round1_coordinator_signing_package_message(resource: SignRequest) -> Self {
        Self {
            message_type: FrostProtoMessageId::CoordinatorRound1SigningPackage,
            message: FrostProtoMessageKind::CoordinatorRound1SigningPackage(resource),
        }
    }

    /// Signers get round 2 signing package
    pub const fn round2_signer_package_message(resource: SignRequest) -> Self {
        Self {
            message_type: FrostProtoMessageId::SignerRound2SigningPackage,
            message: FrostProtoMessageKind::SignerRound2SigningPackage(resource),
        }
    }

    /// Coordinating node collecting the PSBTs with the partial sigs
    pub const fn round2_coordinator_signing_package_message(resource: SignRequest) -> Self {
        Self {
            message_type: FrostProtoMessageId::CoordinatorRound2SigningPackage,
            message: FrostProtoMessageKind::CoordinatorRound2SigningPackage(resource),
        }
    }

    /// Creates a wallet state message
    pub const fn wallet_state_message(resource: WalletStateRequest) -> Self {
        Self {
            message_type: FrostProtoMessageId::WalletState,
            message: FrostProtoMessageKind::WalletState(resource),
        }
    }

    /// Encodes the message into bytes
    pub fn encoded(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_u8(self.message_type as u8);
        match &self.message {
            FrostProtoMessageKind::Dkg(resource) => {
                // sender
                buf.put_u8(resource.sender.len() as u8);
                buf.put_slice(&resource.sender);
                // recipient
                buf.put_u8(resource.recipient.len() as u8);
                buf.put_slice(&resource.recipient);
                // multisig_id
                buf.put_u32_le(resource.multisig_id);
                // data
                buf.put_u32_le(resource.data.len() as u32);
                buf.put_slice(&resource.data);
            }
            FrostProtoMessageKind::Ping | FrostProtoMessageKind::Pong => {}
            FrostProtoMessageKind::PingMessage(peer_id) |
            FrostProtoMessageKind::PongMessage(peer_id) => {
                let peer_id_str = peer_id.to_string();
                let peer_id_bytes = peer_id_str.as_bytes();
                buf.put_u16_le(peer_id_bytes.len() as u16);
                buf.put_slice(peer_id_bytes);
            }
            FrostProtoMessageKind::SignerRound1SigningPackage(resource) |
            FrostProtoMessageKind::SignerRound2SigningPackage(resource) |
            FrostProtoMessageKind::CoordinatorRound1SigningPackage(resource) |
            FrostProtoMessageKind::CoordinatorRound2SigningPackage(resource) => {
                // signing session id
                buf.put_u32_le(resource.signing_session_id.len() as u32);
                buf.put_slice(&resource.signing_session_id);
                // psbt
                buf.put_u32_le(resource.psbt.len() as u32);
                buf.put_slice(&resource.psbt);
            }
            FrostProtoMessageKind::WalletState(resource) => {
                // uuid
                buf.put_u32_le(resource.uuid.len() as u32);
                buf.put_slice(resource.uuid.as_bytes());
                // version
                buf.put_u16_le(resource.version);
                // finalized_pegout_ids
                buf.put_u32_le(resource.finalized_pegout_ids.len() as u32);
                buf.put_slice(&resource.finalized_pegout_ids);
            }
        }
        buf
    }

    /// Decodes a Frost protocol message from the given message buffer.
    pub fn decode_message(buf: &mut &[u8]) -> Option<Self> {
        // Check if buffer is empty
        if buf.is_empty() {
            return None;
        }

        // Safely get message ID
        let id = buf[0];
        buf.advance(1);

        // Match message type
        let message_type = match id {
            0x00 => FrostProtoMessageId::Dkg,
            0x02 => FrostProtoMessageId::Ping,
            0x03 => FrostProtoMessageId::Pong,
            0x04 => FrostProtoMessageId::PingMessage,
            0x05 => FrostProtoMessageId::PongMessage,
            0x06 => FrostProtoMessageId::SignerRound1SigningPackage,
            0x07 => FrostProtoMessageId::CoordinatorRound1SigningPackage,
            0x08 => FrostProtoMessageId::SignerRound2SigningPackage,
            0x09 => FrostProtoMessageId::CoordinatorRound2SigningPackage,
            0x0B => FrostProtoMessageId::WalletState,
            _ => return None,
        };

        // Decode message based on type
        let message = match message_type {
            FrostProtoMessageId::Dkg => {
                // sender_len
                if buf.is_empty() {
                    return None;
                }
                let sender_len = buf[0] as usize;
                buf.advance(1);

                // sender
                if buf.len() < sender_len {
                    return None;
                }
                let sender = buf[..sender_len].to_vec();
                buf.advance(sender_len);

                // recipient_len
                if buf.is_empty() {
                    return None;
                }
                let recipient_len = buf[0] as usize;
                buf.advance(1);

                // recipient
                if buf.len() < recipient_len {
                    return None;
                }
                let recipient = buf[..recipient_len].to_vec();
                buf.advance(recipient_len);

                // multisig_id
                if buf.len() < 4 {
                    return None;
                }
                let multisig_id = u32::from_le_bytes(buf[..4].try_into().ok()?);
                buf.advance(4);

                // data_len
                if buf.len() < 4 {
                    return None;
                }
                let data_len = u32::from_le_bytes(buf[..4].try_into().ok()?) as usize;
                buf.advance(4);

                // data
                if buf.len() < data_len {
                    return None;
                }
                let data = buf[..data_len].to_vec();
                buf.advance(data_len);

                FrostProtoMessageKind::Dkg(DkgRequest::new(data, sender, recipient, multisig_id))
            }
            FrostProtoMessageId::Ping => FrostProtoMessageKind::Ping,
            FrostProtoMessageId::Pong => FrostProtoMessageKind::Pong,

            FrostProtoMessageId::PingMessage | FrostProtoMessageId::PongMessage => {
                // peer_id_len
                if buf.len() < 2 {
                    return None;
                }
                let peer_id_len = u16::from_le_bytes(buf[..2].try_into().ok()?) as usize;
                buf.advance(2);

                // peer_id_str
                if buf.len() < peer_id_len {
                    return None;
                }
                let peer_id_str = std::str::from_utf8(&buf[..peer_id_len]).ok()?;
                let peer_id = PeerId::from_str(peer_id_str).ok()?;
                buf.advance(peer_id_len);

                match message_type {
                    FrostProtoMessageId::PingMessage => FrostProtoMessageKind::PingMessage(peer_id),
                    FrostProtoMessageId::PongMessage => FrostProtoMessageKind::PongMessage(peer_id),
                    _ => unreachable!(),
                }
            }

            FrostProtoMessageId::SignerRound1SigningPackage |
            FrostProtoMessageId::CoordinatorRound1SigningPackage |
            FrostProtoMessageId::SignerRound2SigningPackage |
            FrostProtoMessageId::CoordinatorRound2SigningPackage => {
                // session_id_len
                if buf.len() < 4 {
                    return None;
                }
                let session_id_len = u32::from_le_bytes(buf[..4].try_into().ok()?) as usize;
                buf.advance(4);

                // signing_session_id
                if buf.len() < session_id_len {
                    return None;
                }
                let signing_session_id = buf[..session_id_len].to_vec();
                buf.advance(session_id_len);

                // psbt_len
                if buf.len() < 4 {
                    return None;
                }
                let psbt_len = u32::from_le_bytes(buf[..4].try_into().ok()?) as usize;
                buf.advance(4);

                // psbt
                if buf.len() < psbt_len {
                    return None;
                }
                let psbt = buf[..psbt_len].to_vec();
                buf.advance(psbt_len);

                let sign_request = SignRequest::new(signing_session_id, psbt);

                match message_type {
                    FrostProtoMessageId::SignerRound1SigningPackage => {
                        FrostProtoMessageKind::SignerRound1SigningPackage(sign_request)
                    }
                    FrostProtoMessageId::CoordinatorRound1SigningPackage => {
                        FrostProtoMessageKind::CoordinatorRound1SigningPackage(sign_request)
                    }
                    FrostProtoMessageId::SignerRound2SigningPackage => {
                        FrostProtoMessageKind::SignerRound2SigningPackage(sign_request)
                    }
                    FrostProtoMessageId::CoordinatorRound2SigningPackage => {
                        FrostProtoMessageKind::CoordinatorRound2SigningPackage(sign_request)
                    }
                    _ => unreachable!(),
                }
            }

            FrostProtoMessageId::WalletState => {
                // uuid_len
                if buf.len() < 4 {
                    return None;
                }
                let uuid_len = u32::from_le_bytes(buf[..4].try_into().ok()?) as usize;
                buf.advance(4);

                // uuid
                if buf.len() < uuid_len {
                    return None;
                }
                let uuid = String::from_utf8(buf[..uuid_len].to_vec()).ok()?;
                buf.advance(uuid_len);

                // version
                if buf.len() < 2 {
                    return None;
                }
                let version = u16::from_le_bytes(buf[..2].try_into().ok()?);
                buf.advance(2);

                // finalized_pegout_ids_len
                if buf.len() < 4 {
                    return None;
                }
                let finalized_pegout_ids_len = u32::from_le_bytes(buf[..4].try_into().ok()?) as usize;
                buf.advance(4);

                // finalized_pegout_ids
                if buf.len() < finalized_pegout_ids_len {
                    return None;
                }
                let finalized_pegout_ids = buf[..finalized_pegout_ids_len].to_vec();
                buf.advance(finalized_pegout_ids_len);

                FrostProtoMessageKind::WalletState(WalletStateRequest {
                    uuid,
                    version,
                    finalized_pegout_ids,
                })
            }
        };

        Some(Self { message_type, message })
    }
}

#[cfg(test)]
mod tests {
    use super::WalletStateRequest;
    use super::{
        DkgRequest, FrostProtoMessage, FrostProtoMessageId, FrostProtoMessageKind, SignRequest,
    };
    use reth_network_peers::PeerId;
    use std::str::FromStr;

    #[test]
    fn test_dkg_encoding_decoding() {
        let dkg_request =
            DkgRequest::new(vec![1, 2, 3, 4], vec![5, 6, 7, 8, 9], vec![9, 8, 7, 6, 5], 42);

        let message = FrostProtoMessage {
            message_type: FrostProtoMessageId::Dkg,
            message: FrostProtoMessageKind::Dkg(dkg_request.clone()),
        };

        // Encode the message
        let encoded_bytes = message.encoded();

        // Simulate receiving the encoded bytes and decoding them
        let mut encoded_bytes_slice: &[u8] = &encoded_bytes;
        let decoded_message = FrostProtoMessage::decode_message(&mut encoded_bytes_slice)
            .expect("Failed to decode message");

        // Check that the decoded message matches the original message
        assert_eq!(decoded_message, message);
        
        // Verify multisig_id specifically
        if let FrostProtoMessageKind::Dkg(decoded_dkg) = decoded_message.message {
            assert_eq!(decoded_dkg.multisig_id, 42);
        }
    }

    #[test]
    fn test_signing_encoding_decoding() {
        let signing_request = SignRequest::new(vec![5, 6, 7, 8, 9], vec![0, 1, 0, 1, 0]);

        let message = FrostProtoMessage {
            message_type: FrostProtoMessageId::SignerRound1SigningPackage,
            message: FrostProtoMessageKind::SignerRound1SigningPackage(signing_request),
        };

        // Encode the message
        let encoded_bytes = message.encoded();

        // Simulate receiving the encoded bytes and decoding them
        let mut encoded_bytes_slice: &[u8] = &encoded_bytes;
        let decoded_message = FrostProtoMessage::decode_message(&mut encoded_bytes_slice)
            .expect("Failed to decode message");
        // Check that the decoded message matches the original message
        assert_eq!(decoded_message, message);
    }

    #[test]
    fn test_ping_message_encode_decode() {
        let peer_id = PeerId::from_str("6f8a80d14311c39f35f516fa664deaaaa13e85b2f7493f37f6144d86991ec012937307647bd3b9a82abe2974e1407241d54947bbb39763a4cac9f77166ad92a0").unwrap();

        let message = FrostProtoMessage {
            message_type: FrostProtoMessageId::PingMessage,
            message: FrostProtoMessageKind::PingMessage(peer_id),
        };

        // Encode the message
        let encoded_bytes = message.encoded();

        // Simulate receiving the encoded bytes and decoding them
        let mut encoded_bytes_slice: &[u8] = &encoded_bytes;
        let decoded_message = FrostProtoMessage::decode_message(&mut encoded_bytes_slice)
            .expect("Failed to decode PingMessage");

        // Verify that the decoded message matches the original message
        if let FrostProtoMessageKind::PingMessage(decoded_peer_id) = decoded_message.message {
            assert_eq!(decoded_peer_id, peer_id, "PeerId does not match");
        } else {
            panic!("Decoded message is not a PingMessage");
        }
    }

    #[test]
    fn test_pong_message_encode_decode() {
        let peer_id = PeerId::from_str("6f8a80d14311c39f35f516fa664deaaaa13e85b2f7493f37f6144d86991ec012937307647bd3b9a82abe2974e1407241d54947bbb39763a4cac9f77166ad92a0").unwrap();

        let message = FrostProtoMessage {
            message_type: FrostProtoMessageId::PongMessage,
            message: FrostProtoMessageKind::PongMessage(peer_id),
        };

        // Encode the message
        let encoded_bytes = message.encoded();

        // Simulate receiving the encoded bytes and decoding them
        let mut encoded_bytes_slice: &[u8] = &encoded_bytes;
        let decoded_message = FrostProtoMessage::decode_message(&mut encoded_bytes_slice)
            .expect("Failed to decode PongMessage");

        // Verify that the decoded message matches the original message
        if let FrostProtoMessageKind::PongMessage(decoded_peer_id) = decoded_message.message {
            assert_eq!(decoded_peer_id, peer_id, "PeerId does not match");
        } else {
            panic!("Decoded message is not a PongMessage");
        }
    }

    #[test]
    fn test_wallet_state_encode_decode() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000".to_string();
        let finalized_pegout_ids = vec![1, 2, 3];
        let message = FrostProtoMessage {
            message_type: FrostProtoMessageId::WalletState,
            message: FrostProtoMessageKind::WalletState(WalletStateRequest {
                uuid,
                version: 1,
                finalized_pegout_ids: finalized_pegout_ids.clone(),
            }),
        };

        // Encode the message
        let encoded_bytes = message.encoded();

        // Simulate receiving the encoded bytes and decoding them
        let mut encoded_bytes_slice: &[u8] = &encoded_bytes;
        let decoded_message = FrostProtoMessage::decode_message(&mut encoded_bytes_slice)
            .expect("Failed to decode WalletStateMessage");

        // Verify that the decoded message matches the original message
        if let FrostProtoMessageKind::WalletState(wallet_state_request) = decoded_message.message {
            assert_eq!(
                wallet_state_request.uuid, "550e8400-e29b-41d4-a716-446655440000",
                "uuid does not match"
            );
            assert_eq!(wallet_state_request.version, 1, "version does not match");
            assert_eq!(
                wallet_state_request.finalized_pegout_ids.len(),
                finalized_pegout_ids.len(),
                "finalized_pegout_ids length does not match"
            );
            assert_eq!(
                wallet_state_request.finalized_pegout_ids, finalized_pegout_ids,
                "pegout id does not match"
            );
        } else {
            panic!("Decoded message is not a WalletState Message");
        }
    }
}