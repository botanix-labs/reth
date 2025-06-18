//! Models for staged headers with their associated pegins and pegouts.

use reth_codecs::{add_arbitrary_tests, Compact};
use reth_primitives_traits::Header;
use serde::{Deserialize, Serialize};

/// A header with associated pegins and pegouts.
#[derive(Debug, Default, Eq, PartialEq, Clone, Serialize, Deserialize, Compact)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[add_arbitrary_tests(compact)]
pub struct HeaderWithPegs {
    /// The pegins associated with this header.
    pub pegins: Vec<PeginData>,
    /// The pegouts associated with this header.
    pub pegouts: Vec<PegoutData>,
    /// The header to which these pegins and pegouts are associated.
    pub header: Header,
}

/// Pegin data associated with a header.
#[derive(Debug, Default, Eq, PartialEq, Clone, Serialize, Deserialize, Compact)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[add_arbitrary_tests(compact)]
pub struct PeginData {
    /// The Bitcoin transaction ID that contains the pegin output.
    pub txid: Vec<u8>,
    /// The output index of the pegin output in the Bitcoin transaction.
    pub vout: u64,
    /// The value of the pegin output in satoshis.
    pub value: u64,
    /// The script that must be satisfied to claim the output.
    pub script_pubkey: Vec<u8>,
    /// Final destination address of the pegin (non-hex encoded).
    pub eth_address: Vec<u8>,
}

/// Pegout data associated with a header.
#[derive(Debug, Default, Eq, PartialEq, Clone, Serialize, Deserialize, Compact)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[add_arbitrary_tests(compact)]
pub struct PegoutData {
    /// The pegout identifier.
    pub pegout_id: Vec<u8>,
    /// The script that must be satisfied to claim the output.
    pub script_pubkey: Vec<u8>,
    /// Amount to be pegged out.
    pub amount: u64,
    /// Height at which the pegout was requested.
    pub height: u64,
}
