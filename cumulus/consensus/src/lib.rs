// Copyright 2019 Parity Technologies (UK) Ltd.
// This file is part of Cumulus.

// Cumulus is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Cumulus is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Cumulus.  If not, see <http://www.gnu.org/licenses/>.

use substrate_client::{backend::Backend, Client, BlockchainEvents};
use substrate_client::error::{Error as ClientError, Result as ClientResult};
use sr_primitives::traits::{Block as BlockT, Header as HeaderT, ProvideRuntimeApi};
use polkadot_primitives::{BlockNumber as PBlockNumber, Hash as PHash, parachain::Id as ParaId};

use futures::prelude::*;
use parity_codec::Decode;
use log::warn;

use std::sync::Arc;

/// Helper for the local client.
pub trait LocalClient {
	/// The block type of the local client.
	type Block: BlockT;

	/// Mark the given block as the best block.
	/// Returns `false` if the block is not known.
	fn mark_best(&self, hash: <Self::Block as BlockT>::Hash) -> ClientResult<bool>;

	/// Finalize the given block.
	/// Returns `false` if the block is not known.
	fn finalize(&self, hash: <Self::Block as BlockT>::Hash) -> ClientResult<bool>;
}

/// Errors that can occur while following the polkadot relay-chain.
#[derive(Debug)]
pub enum Error<P> {
	/// An underlying client error.
	Client(ClientError),
	/// Polkadot client error.
	Polkadot(P),
	/// Head data returned was not for our parachain.
	InvalidHeadData,
}

/// A parachain head update.
pub struct HeadUpdate {
	/// The relay-chain's block hash where the parachain head updated.
	pub relay_hash: PHash, 
	/// The relay-chain's block number where the parachain head updated.
	pub relay_number: PBlockNumber,
	/// The parachain head-data.
	pub head_data: Vec<u8>,
}

/// Helper for the Polkadot client.
pub trait PolkadotClient {
	/// The error type for interacting with the Polkadot client.
	type Error: std::fmt::Debug + Send;

	/// A stream that yields updates to the parachain head.
	type HeadUpdates: Stream<Item=HeadUpdate,Error=Self::Error> + Send;
	/// A stream that yields finalized head-data for a certain parachain.
	type Finalized: Stream<Item=Vec<u8>,Error=Self::Error> + Send;

	/// Get a stream of head updates.
	fn head_updates(&self, para_id: ParaId) -> Self::HeadUpdates;
	/// Get a stream of finalized heads.
	fn finalized_heads(&self, para_id: ParaId) -> Self::Finalized;
}

/// Spawns a future that follows the Polkadot relay chain for the given parachain.
pub fn follow_polkadot<'a, L: 'a, P: 'a>(para_id: ParaId, local: Arc<L>, polkadot: Arc<P>) 
	-> impl Future<Item=(),Error=()> + Send + 'a
	where 
		L: LocalClient + Send + Sync,
		P: PolkadotClient + Send + Sync,
{
	let head_updates = polkadot.head_updates(para_id);
	let finalized_heads = polkadot.finalized_heads(para_id);

	let follow_best = {
		let local = local.clone();

		head_updates
			.map_err(Error::Polkadot)
			.and_then(|update| {
				<L::Block as BlockT>::Header::decode(&mut &update.head_data[..])
					.ok_or_else(|| Error::InvalidHeadData)
			})
			.for_each(move |p_head| {
				let _synced = local.mark_best(p_head.hash()).map_err(Error::Client)?;
				Ok(())
			})
	};

	let follow_finalized = {
		let local = local.clone();

		finalized_heads
			.map_err(Error::Polkadot)
			.and_then(|head_data| {
				<L::Block as BlockT>::Header::decode(&mut &head_data[..])
					.ok_or_else(|| Error::InvalidHeadData)
			})
			.for_each(move |p_head| {
				let _synced = local.finalize(p_head.hash()).map_err(Error::Client)?;
				Ok(())
			})
	};

	follow_best.join(follow_finalized)
		.map_err(|e| warn!("Could not follow relay-chain: {:?}", e))
		.map(|((), ())| ())
}