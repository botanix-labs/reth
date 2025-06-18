//! Models used in storage module.

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/paradigmxyz/reth/main/assets/reth-docs.png",
    html_favicon_url = "https://avatars0.githubusercontent.com/u/97369466?s=256",
    issue_tracker_base_url = "https://github.com/paradigmxyz/reth/issues/"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

/// Accounts
pub mod accounts;
pub use accounts::AccountBeforeTx;

/// Blocks
pub mod blocks;
pub use blocks::{StaticFileBlockWithdrawals, StoredBlockBodyIndices, StoredBlockWithdrawals};

/// Client Version
pub mod client_version;
pub use client_version::ClientVersion;

// Activation Manager
pub mod activation_manager;
pub use activation_manager::*;

// Wallet Sync
pub mod wallet_sync;
pub use wallet_sync::*;

// Chunks
pub mod chunks;
pub use chunks::*;

// Staged Header
pub mod staged_header;
pub use staged_header::*;