// This file is part of Web3Games.

// Copyright (C) 2021-2022 Web3Games https://web3games.org
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::{
	dispatch::DispatchResult,
	traits::{Currency, ExistenceRequirement::KeepAlive, Get, ReservableCurrency},
	PalletId,
};
use sp_runtime::traits::{AccountIdConversion, UniqueSaturatedFrom};

use sp_std::prelude::*;

pub use pallet::*;
use primitives::Balance;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

type BalanceOf<T> =
	<<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::{dispatch::DispatchResult, pallet_prelude::*};
	use frame_system::pallet_prelude::*;

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_token_fungible::Config {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		// #[pallet::constant]
		type PalletId: Get<PalletId>;

		#[pallet::constant]
		type CreateTokenDeposit: Get<BalanceOf<Self>>;

		type Currency: Currency<Self::AccountId> + ReservableCurrency<Self::AccountId>;
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::storage]
	pub(super) type WrapToken<T: Config> = StorageValue<_, T::FungibleTokenId, ValueQuery>;

	#[pallet::genesis_config]
	#[derive(Default)]
	pub struct GenesisConfig {}

	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig
	where
		<T as pallet_token_fungible::Config>::FungibleTokenId: From<u128>,
	{
		fn build(&self) {
			let result = Pallet::<T>::create_wrap_token();
			assert!(result.is_ok());
		}
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		Deposited(T::AccountId, Balance),
		Withdrawn(T::AccountId, Balance),
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::weight(10_000)]
		pub fn deposit(origin: OriginFor<T>, amount: Balance) -> DispatchResult {
			let who = ensure_signed(origin)?;
			Self::do_deposit(who.clone(), amount)?;
			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn withdraw(origin: OriginFor<T>, amount: Balance) -> DispatchResult {
			let who = ensure_signed(origin)?;
			Self::do_withdraw(who.clone(), amount)?;
			Ok(())
		}
	}
}

impl<T: Config> Pallet<T> {
	pub fn account_id() -> T::AccountId {
		<T as pallet::Config>::PalletId::get().into_account_truncating()
	}

	pub fn do_deposit(who: T::AccountId, amount: Balance) -> DispatchResult {
		let vault_account = Self::account_id();

		<T as Config>::Currency::transfer(
			&who,
			&vault_account,
			BalanceOf::<T>::unique_saturated_from(amount),
			KeepAlive,
		)?;

		let token_id = WrapToken::<T>::get();
		pallet_token_fungible::Pallet::<T>::do_mint(token_id, &vault_account, who.clone(), amount)?;
		Self::deposit_event(Event::Deposited(who, amount));
		Ok(())
	}

	pub fn do_withdraw(who: T::AccountId, amount: Balance) -> DispatchResult {
		<T as Config>::Currency::transfer(
			&Self::account_id(),
			&who,
			BalanceOf::<T>::unique_saturated_from(amount),
			KeepAlive,
		)?;

		let token_id = WrapToken::<T>::get();
		pallet_token_fungible::Pallet::<T>::do_burn(token_id, &who, amount)?;
		Self::deposit_event(Event::Withdrawn(who, amount));
		Ok(())
	}

	fn create_wrap_token() -> DispatchResult {
		let vault_account = Self::account_id();

		let deposit = <T as Config>::CreateTokenDeposit::get();
		<T as Config>::Currency::deposit_creating(&vault_account, deposit);

		let id: T::FungibleTokenId =
			<T as pallet_token_fungible::Config>::FungibleTokenId::default();
		let name: Vec<u8> = "Wrapped Currency".as_bytes().to_vec();
		let symbol: Vec<u8> = "WW3G".as_bytes().to_vec();

		pallet_token_fungible::Pallet::<T>::do_create_token(&vault_account, id, name, symbol, 18)?;

		WrapToken::<T>::put(id);

		Ok(())
	}
}
