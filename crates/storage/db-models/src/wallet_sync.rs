//! Models for a wallet state sync record.

use reth_codecs::{add_arbitrary_tests, Compact};
use alloy_primitives::{Bytes, B256, B512};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// A peer id is a hexified peer if of a wallet state sync record.
pub type PeerID = B512;

/// A peer id is the hexified uuid for a wallet state sync record.
pub type UuidID = B256;

/// Wallet state sync record
#[derive(Debug, Default, Eq, PartialEq, Clone, Serialize, Deserialize, Compact)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[add_arbitrary_tests(compact)]
pub struct WalletStateSyncRecord {
    /// the uuid of the session
    uuid: B256,
    /// The finalized pegouts data
    data: Vec<Bytes>,
    /// The blocks of each wallet state sync record
    blocks: Vec<u64>,
    /// The total number of chunks expected
    chunks_count: u64,
    /// peer id
    peer_id: B512,
}

impl WalletStateSyncRecord {
    /// Creates a new wallet state sync record
    pub fn new(
        peer_id: PeerID,
        uuid: UuidID,
        chunks_count: u64,
        data: Option<Vec<(u64, Bytes)>>,
    ) -> Self {
        if let Some(tuples) = data {
            let (blocks, data_bytes): (Vec<u64>, Vec<Bytes>) = tuples.into_iter().unzip();
            Self { uuid, data: data_bytes, blocks, chunks_count, peer_id }
        } else {
            Self { uuid, data: Vec::new(), blocks: Vec::new(), chunks_count, peer_id }
        }
    }

    /// Appends data with block number to the existing wallet state sync record.
    pub fn append_data_with_block(&mut self, additional_data: Bytes, block_number: u64) {
        self.add_data_if_not_exists(additional_data, block_number);
    }

    /// Appends additional data chunks with block numbers to the existing wallet state sync record.
    pub fn append_data_block_chunks(
        &mut self,
        additional_data_chunks: Vec<Bytes>,
        blocks: Vec<u64>,
    ) {
        for (block, data) in blocks.iter().zip(additional_data_chunks) {
            self.add_data_if_not_exists(data, *block);
        }
    }

    /// Returns an iterator over block numbers with data.
    pub fn get_blocks_data_iter(&mut self) -> impl Iterator<Item = (&u64, &Bytes)> {
        self.blocks.iter().zip(&self.data)
    }

    /// Return the size of this wallet state sync record.
    pub fn size(&self) -> usize {
        let uuid_size = std::mem::size_of::<B256>();
        let peer_id = std::mem::size_of::<B256>();
        let data_size = self.data.iter().map(|data| data.len()).sum::<usize>();
        let blocks_size = self.blocks.len() * std::mem::size_of::<u64>();
        uuid_size + peer_id + data_size + blocks_size
    }

    /// Return the uuid of this wallet state sync record.
    pub const fn get_uuid(&self) -> B256 {
        self.uuid
    }

    /// Return the data of this wallet state sync record.
    pub fn get_data(&self) -> &[Bytes] {
        self.data.as_ref()
    }

    /// Return the blocks of the wallet state sync records.
    pub fn get_blocks(&self) -> &[u64] {
        self.blocks.as_ref()
    }

    /// Return the `peer_id of` this wallet state sync record.
    pub const fn get_peer_id(&self) -> B512 {
        self.peer_id
    }

    /// Return the chunks count of this wallet state sync record.
    pub const fn get_chunks_count(&self) -> u64 {
        self.chunks_count
    }

    /// Sets the peer id of the wallet state sync record.
    pub fn set_peer_id(&mut self, peer_id: B512) {
        self.peer_id = peer_id;
    }

    /// Sets the chunks count for the wallet state sync record.
    pub fn set_chunks_count(&mut self, chunks_count: u64) {
        self.chunks_count = chunks_count;
    }

    /// Sets the uuid of the wallet state sync record.
    pub fn set_uuid(&mut self, uuid: B256) {
        self.uuid = uuid;
    }

    /// Adds a data chunk with its block number to the wallet state sync record if it doesn't
    /// already exist. Returns `true` if the data or block was added, `false` if it was already
    /// present.
    pub fn add_data_if_not_exists(&mut self, data_chunk: Bytes, block_number: u64) -> bool {
        if self.data.iter().any(|data| data == &data_chunk) {
            return false;
        }
        if self.blocks.iter().any(|block| block == &block_number) {
            return false;
        }
        self.data.push(data_chunk);
        self.blocks.push(block_number);
        true
    }

    /// Converts the blocks and data to a set of unique (block, data) tuples.
    pub fn blocks_and_data_to_set(&mut self) -> HashSet<(u64, Bytes)> {
        self.blocks
            .iter()
            .zip(self.data.iter())
            .map(|(block, data)| (*block, data.clone()))
            .collect()
    }

    /// Gets the hash of the wallet state sync record.
    pub fn get_hash(&self) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(self.peer_id.as_slice());
        hasher.update(self.uuid.as_slice());
        for data_chunk in &self.data {
            hasher.update(data_chunk);
        }
        hasher.finalize().to_vec()
    }
}

/// Converts a `uuid::Uuid` to a `UuidID`.
pub fn uuid_to_b256(uuid: uuid::Uuid) -> UuidID {
    let mut uuid_fixed_bytes = [0u8; 32];
    uuid_fixed_bytes[0..16].copy_from_slice(uuid.as_bytes());
    uuid_fixed_bytes.into()
}

#[cfg(test)]
mod tests {
    use alloy_primitives::hex;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn wallet_state_sync_record_test() {
        let uuid = Uuid::new_v4();
        let uuid_fixed_bytes = uuid_to_b256(uuid);
        let peer_id = PeerID::random();
        let data_chunk = Bytes::from(vec![1, 2, 3]);
        let chunks_count = 1;
        let block_number = 100;
        let wallet_state_sync_record = WalletStateSyncRecord {
            uuid: uuid_fixed_bytes.into(),
            peer_id,
            data: vec![data_chunk.clone()],
            blocks: vec![block_number],
            chunks_count,
        };
        assert_eq!(wallet_state_sync_record.get_uuid(), uuid_fixed_bytes);
        assert_eq!(wallet_state_sync_record.get_peer_id(), peer_id);
        assert_eq!(wallet_state_sync_record.get_data(), [data_chunk]);
        assert_eq!(wallet_state_sync_record.get_blocks(), [block_number]);
        assert_eq!(wallet_state_sync_record.get_chunks_count(), chunks_count);
        assert_eq!(wallet_state_sync_record.size(), 32 + 32 + 3 + 8);

        let hash = wallet_state_sync_record.get_hash();
        assert_eq!(hex::encode(hash).len(), 64);
    }
}
