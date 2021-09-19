#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Decode, Encode};
use frame_support::{
	dispatch::{DispatchError, DispatchResult},
	ensure,
	traits::{Currency, ExistenceRequirement::AllowDeath, Get, ReservableCurrency},
	PalletId,
};
use primitives::{Balance, PoolIndex, TokenId};
use sp_core::U256;
use sp_runtime::{
	traits::{AccountIdConversion, One, Zero},
	RuntimeDebug,
};
use sp_std::{convert::TryInto, prelude::*};

pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

type BalanceOf<T> =
	<<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

#[derive(Encode, Decode, Clone, Eq, PartialEq, RuntimeDebug)]
pub struct Pool<AccountId> {
	/// The owner of pool
	pub owner: AccountId,
	/// The account of the currency
	pub currency: AccountId,
	/// The account of the token
	pub token: AccountId,
	/// The account of liquidity pool token
	pub lp_token: AccountId,
	/// The account of pool
	pub vault: AccountId,
}

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::{dispatch::DispatchResult, pallet_prelude::*};
	use frame_system::pallet_prelude::*;

	#[pallet::config]
	pub trait Config:
		frame_system::Config + pallet_token_fungible::Config + pallet_token_multi::Config
	{
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		type PalletId: Get<PalletId>;

		/// The minimum balance to create pool
		#[pallet::constant]
		type CreatePoolDeposit: Get<BalanceOf<Self>>;

		type Currency: Currency<Self::AccountId> + ReservableCurrency<Self::AccountId>;
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::storage]
	pub(super) type Pools<T: Config> = StorageMap<_, Blake2_128, T::AccountId, Pool<T::AccountId>>;

	#[pallet::storage]
	#[pallet::getter(fn pool_count)]
	pub(super) type PoolCount<T> = StorageValue<_, PoolIndex, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn total_supplies)]
	pub(super) type TotalSupplies<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		T::AccountId,
		Blake2_128Concat,
		TokenId,
		Balance,
		ValueQuery,
	>;

	#[pallet::storage]
	#[pallet::getter(fn currency_reserves)]
	pub(super) type CurrencyReserves<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		T::AccountId,
		Blake2_128Concat,
		TokenId,
		Balance,
		ValueQuery,
	>;

	#[pallet::event]
	#[pallet::metadata(T::AccountId = "AccountId")]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		PoolCreated(T::AccountId, T::AccountId),
		SwapCurrencyToToken(
			T::AccountId,
			T::AccountId,
			Vec<TokenId>,
			Vec<Balance>,
			Balance,
			T::AccountId,
		),
		SwapTokenToCurrency(
			T::AccountId,
			T::AccountId,
			Vec<TokenId>,
			Vec<Balance>,
			Balance,
			T::AccountId,
		),
		LiquidityAdded(
			T::AccountId,
			T::AccountId,
			Vec<TokenId>,
			Vec<Balance>,
			Vec<Balance>,
		),
		LiquidityRemoved(
			T::AccountId,
			T::AccountId,
			Vec<TokenId>,
			Vec<Balance>,
			Vec<Balance>,
		),
	}

	#[pallet::error]
	pub enum Error<T> {
		CurrencyAccountNotFound,
		TokenAccountNotFound,
		Overflow,
		InvalidPoolAccount,
		NullMaxCurrency,
		NullTokensAmount,
		InsufficientCurrencyAmount,
		InsufficientTokens,
		MaxCurrencyAmountExceeded,
		InvalidCurrencyAmount,
		NullTotalLiquidity,
		NullTokensBought,
		NullTokensSold,
		EmptyReserve,
		UnsortedOrDuplicateTokenIds,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::weight(10_000)]
		pub fn create_pool(
			origin: OriginFor<T>,
			currency: T::AccountId, // fungible-token token account
			token: T::AccountId,    // multi-token token account
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			ensure!(
				pallet_token_fungible::Pallet::<T>::exists(&currency),
				Error::<T>::CurrencyAccountNotFound
			);
			ensure!(
				pallet_token_multi::Pallet::<T>::exists(&token),
				Error::<T>::TokenAccountNotFound
			);

			let pool_id =
				PoolCount::<T>::try_mutate(|count| -> Result<PoolIndex, DispatchError> {
					let new_count = count.checked_add(One::one()).ok_or(Error::<T>::Overflow)?;
					*count = new_count;
					Ok(new_count)
				})?;

			let pool_account = Self::pool_account_id(pool_id);

			let vault = Self::account_id();

			let deposit = T::CreatePoolDeposit::get();
			<T as Config>::Currency::transfer(&who, &vault, deposit, AllowDeath)?;

			let lp_token = pallet_token_multi::Pallet::<T>::do_create_token(&vault, [].to_vec())?;

			let pool = Pool {
				owner: who.clone(),
				currency,
				token,
				lp_token,
				vault,
			};

			Pools::<T>::insert(&pool_account, pool);

			Self::deposit_event(Event::PoolCreated(pool_account, who));

			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn swap_currency_to_token(
			origin: OriginFor<T>,
			pool_account: T::AccountId,
			token_ids: Vec<TokenId>,
			token_amounts_out: Vec<Balance>,
			max_currency: Balance,
			to: T::AccountId,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			Self::do_swap_currency_to_token(
				&who,
				&pool_account,
				token_ids,
				token_amounts_out,
				max_currency,
				&to,
			)?;

			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn swap_token_to_currency(
			origin: OriginFor<T>,
			pool_account: T::AccountId,
			token_ids: Vec<TokenId>,
			token_amounts_in: Vec<Balance>,
			min_currency: Balance,
			to: T::AccountId,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			Self::do_swap_token_to_currency(
				&who,
				&pool_account,
				token_ids,
				token_amounts_in,
				min_currency,
				&to,
			)?;

			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn add_liquidity(
			origin: OriginFor<T>,
			pool_account: T::AccountId,
			token_ids: Vec<TokenId>,
			token_amounts: Vec<Balance>,
			max_currencies: Vec<Balance>,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			Self::do_add_liquidity(
				&who,
				&pool_account,
				token_ids,
				token_amounts,
				max_currencies,
			)?;

			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn remove_liquidity(
			origin: OriginFor<T>,
			pool_account: T::AccountId,
			token_ids: Vec<TokenId>,
			liquidities: Vec<Balance>,
			min_currencies: Vec<Balance>,
			min_tokens: Vec<Balance>,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			Self::do_remove_liquidity(
				&who,
				&pool_account,
				token_ids,
				liquidities,
				min_currencies,
				min_tokens,
			)?;

			Ok(())
		}
	}
}

impl<T: Config> Pallet<T> {
	// The account ID of the vault
	pub fn account_id() -> T::AccountId {
		<T as Config>::PalletId::get().into_account()
	}

	// The account ID of a pool account
	pub fn pool_account_id(id: PoolIndex) -> T::AccountId {
		<T as Config>::PalletId::get().into_sub_account(id)
	}

	pub fn do_swap_currency_to_token(
		who: &T::AccountId,
		pool_account: &T::AccountId,
		token_ids: Vec<TokenId>,
		token_amounts_out: Vec<Balance>,
		max_currency: Balance,
		to: &T::AccountId,
	) -> Result<Vec<Balance>, DispatchError> {
		let pool = Pools::<T>::get(pool_account).ok_or(Error::<T>::InvalidPoolAccount)?;

		// Transfer max currency token to vault account
		pallet_token_fungible::Pallet::<T>::do_transfer_from(
			who,
			&pool.currency,
			who,
			&pool.vault,
			max_currency,
		)?;

		let n = token_ids.len();
		let mut currency_amounts_in = vec![Balance::from(0u128); n];
		let mut total_refund_currency = max_currency;

		let token_reserves = Self::get_token_reserves(&pool.vault, &pool.token, token_ids.clone())?;

		for i in 0..n {
			let token_id = token_ids[i];
			let token_amount_out = token_amounts_out[i];
			let token_reserve = token_reserves[i];

			ensure!(
				token_amount_out > Zero::zero(),
				Error::<T>::NullTokensBought
			);

			let currency_reserve = Self::currency_reserves(pool_account, token_id);
			let currency_amount =
				Self::get_buy_price(token_amount_out, currency_reserve, token_reserve)?;

			total_refund_currency = total_refund_currency.saturating_sub(currency_amount);

			currency_amounts_in[i] = currency_amount;

			// Update individual currency reserve amount
			CurrencyReserves::<T>::try_mutate(
				pool_account,
				token_id,
				|currency_reserve| -> DispatchResult {
					*currency_reserve = currency_reserve
						.checked_add(currency_amount)
						.ok_or(Error::<T>::Overflow)?;
					Ok(())
				},
			)?;
		}

		// Refund currency token if any
		if total_refund_currency > 0 {
			pallet_token_fungible::Pallet::<T>::do_transfer_from(
				&pool.vault,
				&pool.currency,
				&pool.vault,
				who,
				total_refund_currency,
			)?;
		}

		// Send Tokens all tokens purchased
		pallet_token_multi::Pallet::<T>::do_batch_transfer_from(
			&pool.vault,
			&pool.token,
			&pool.vault,
			to,
			token_ids.clone(),
			token_amounts_out.clone(),
		)?;

		Self::deposit_event(Event::SwapCurrencyToToken(
			who.clone(),
			pool_account.clone(),
			token_ids,
			token_amounts_out,
			max_currency.saturating_sub(total_refund_currency),
			to.clone(),
		));

		Ok(currency_amounts_in)
	}

	pub fn do_swap_token_to_currency(
		who: &T::AccountId,
		pool_account: &T::AccountId,
		token_ids: Vec<TokenId>,
		token_amounts_in: Vec<Balance>,
		min_currency: Balance,
		to: &T::AccountId,
	) -> Result<Vec<Balance>, DispatchError> {
		let pool = Pools::<T>::get(pool_account).ok_or(Error::<T>::InvalidPoolAccount)?;

		// Transfer the tokens to vault account
		pallet_token_multi::Pallet::<T>::do_batch_transfer_from(
			who,
			&pool.token,
			who,
			&pool.vault,
			token_ids.clone(),
			token_amounts_in.clone(),
		)?;

		let n = token_ids.len();
		let mut total_currency = Balance::from(0u128);
		let mut currency_amounts_out = vec![Balance::from(0u128); n];

		let token_reserves = Self::get_token_reserves(&pool.vault, &pool.token, token_ids.clone())?;

		for i in 0..n {
			let token_id = token_ids[i];
			let token_amount_in = token_amounts_in[i];
			let token_reserve = token_reserves[i];

			ensure!(token_amount_in > Zero::zero(), Error::<T>::NullTokensSold);

			let currency_reserve = Self::currency_reserves(pool_account, token_id);
			let currency_amount = Self::get_sell_price(
				token_amount_in,
				token_reserve.saturating_sub(token_amount_in),
				currency_reserve,
			)?;

			total_currency = total_currency.saturating_add(currency_amount);

			// Update individual currency reserve amount
			CurrencyReserves::<T>::try_mutate(
				pool_account,
				token_id,
				|currency_reserve| -> DispatchResult {
					*currency_reserve = currency_reserve
						.checked_sub(currency_amount)
						.ok_or(Error::<T>::Overflow)?;
					Ok(())
				},
			)?;

			currency_amounts_out[i] = currency_amount;
		}

		// If minCurrency is not met
		ensure!(
			total_currency >= min_currency,
			Error::<T>::InsufficientCurrencyAmount
		);

		// Transfer currency here
		pallet_token_fungible::Pallet::<T>::do_transfer_from(
			&pool.vault,
			&pool.currency,
			&pool.vault,
			to,
			total_currency,
		)?;

		Self::deposit_event(Event::SwapTokenToCurrency(
			who.clone(),
			pool_account.clone(),
			token_ids,
			token_amounts_in,
			total_currency,
			to.clone(),
		));

		Ok(currency_amounts_out)
	}

	pub fn do_add_liquidity(
		who: &T::AccountId,
		pool_account: &T::AccountId,
		token_ids: Vec<TokenId>,
		token_amounts: Vec<Balance>,
		max_currencies: Vec<Balance>,
	) -> DispatchResult {
		let pool = Pools::<T>::get(pool_account).ok_or(Error::<T>::InvalidPoolAccount)?;

		// Transfer all tokens to this contract
		pallet_token_multi::Pallet::<T>::do_batch_transfer_from(
			who,
			&pool.token,
			who,
			&pool.vault,
			token_ids.clone(),
			token_amounts.clone(),
		)?;

		let n = token_ids.len();
		let mut total_currency = Balance::from(0u128);
		let mut liquidities_to_mint = vec![Balance::from(0u128); n];
		let mut currency_amounts = vec![Balance::from(0u128); n];

		let token_reserves = Self::get_token_reserves(&pool.vault, &pool.token, token_ids.clone())?;

		for i in 0..n {
			let token_id = token_ids[i];
			let amount = token_amounts[i];

			ensure!(
				max_currencies[i] > Zero::zero(),
				Error::<T>::NullMaxCurrency
			);
			ensure!(amount > Zero::zero(), Error::<T>::NullTokensAmount);

			let total_liquidity = Self::total_supplies(pool_account, token_id);

			if total_liquidity > Zero::zero() {
				let currency_reserve = Self::currency_reserves(pool_account, token_id);
				let token_reserve = token_reserves[i];

				let (currency_amount, rounded) = Self::div_round(
					U256::from(amount).saturating_mul(U256::from(currency_reserve)),
					U256::from(token_reserve).saturating_sub(U256::from(amount)),
				);
				ensure!(
					max_currencies[i] >= currency_amount,
					Error::<T>::MaxCurrencyAmountExceeded
				);

				// Update currency reserve size for Token id before transfer
				CurrencyReserves::<T>::try_mutate(
					pool_account,
					token_id,
					|currency_reserve| -> DispatchResult {
						*currency_reserve = currency_reserve
							.checked_add(currency_amount)
							.ok_or(Error::<T>::Overflow)?;
						Ok(())
					},
				)?;

				// Update totalCurrency
				total_currency = total_currency.saturating_add(currency_amount);

				// If rounding error occurred, round down to favor previous liquidity providers
				let fixed_currency_amount = if rounded {
					currency_amount.saturating_sub(1u128)
				} else {
					currency_amount
				};
				liquidities_to_mint[i] =
					(fixed_currency_amount.saturating_mul(total_liquidity)) / currency_reserve;
				currency_amounts[i] = currency_amount;

				// Mint liquidity ownership tokens and increase liquidity supply accordingly
				TotalSupplies::<T>::try_mutate(
					pool_account,
					token_id,
					|total_supply| -> DispatchResult {
						*total_supply = total_liquidity
							.checked_add(liquidities_to_mint[i])
							.ok_or(Error::<T>::Overflow)?;
						Ok(())
					},
				)?;
			} else {
				let max_currency = max_currencies[i];

				// Otherwise rounding error could end up being significant on second deposit
				ensure!(
					max_currency >= Balance::from(1000000000u128),
					Error::<T>::InvalidCurrencyAmount
				);

				// Update currency reserve size for Token id before transfer
				CurrencyReserves::<T>::mutate(pool_account, token_id, |currency_reserve| {
					*currency_reserve = max_currency
				});

				// Update totalCurrency
				total_currency = total_currency.saturating_add(max_currency);

				// Initial liquidity is amount deposited (Incorrect pricing will be arbitraged)
				// uint256 initialLiquidity = maxCurrency;
				TotalSupplies::<T>::mutate(pool_account, token_id, |total_supply| {
					*total_supply = max_currency
				});

				// Liquidity to mints
				liquidities_to_mint[i] = max_currency;
				currency_amounts[i] = max_currency;
			}
		}

		// Mint liquidity pool tokens
		pallet_token_multi::Pallet::<T>::do_batch_mint(
			&pool.vault,
			&pool.lp_token,
			&who,
			token_ids.clone(),
			liquidities_to_mint,
		)?;

		// Transfer all currency to this contract
		pallet_token_fungible::Pallet::<T>::do_transfer_from(
			who,
			&pool.currency,
			who,
			&pool.vault,
			total_currency,
		)?;

		Self::deposit_event(Event::LiquidityAdded(
			who.clone(),
			pool_account.clone(),
			token_ids,
			token_amounts,
			currency_amounts,
		));

		Ok(())
	}

	pub fn do_remove_liquidity(
		who: &T::AccountId,
		pool_account: &T::AccountId,
		token_ids: Vec<TokenId>,
		liquidities: Vec<Balance>,
		min_currencies: Vec<Balance>,
		min_tokens: Vec<Balance>,
	) -> DispatchResult {
		let pool = Pools::<T>::get(pool_account).ok_or(Error::<T>::InvalidPoolAccount)?;

		// Transfer the liquidity pool tokens to burn to this contract
		pallet_token_multi::Pallet::<T>::do_batch_transfer_from(
			who,
			&pool.lp_token,
			who,
			&pool.vault,
			token_ids.clone(),
			liquidities.clone(),
		)?;

		let n = token_ids.len();
		let mut total_currency = Balance::from(0u128);
		let mut token_amounts = vec![Balance::from(0u128); n];
		let mut currency_amounts = vec![Balance::from(0u128); n];

		let token_reserves = Self::get_token_reserves(&pool.vault, &pool.token, token_ids.clone())?;

		for i in 0..n {
			let token_id = token_ids[i];
			let liquidity = liquidities[i];
			let token_reserve = token_reserves[i];

			let total_liquidity = Self::total_supplies(pool_account, token_id);
			ensure!(
				total_liquidity > Zero::zero(),
				Error::<T>::NullTotalLiquidity
			);

			let currency_reserve = Self::currency_reserves(pool_account, token_id);

			let currency_amount = U256::from(liquidity)
				.saturating_mul(U256::from(currency_reserve))
				.checked_div(U256::from(total_liquidity))
				.and_then(|n| TryInto::<Balance>::try_into(n).ok())
				.unwrap_or_else(Zero::zero);

			let token_amount = U256::from(liquidity)
				.saturating_mul(U256::from(token_reserve))
				.checked_div(U256::from(total_liquidity))
				.and_then(|n| TryInto::<Balance>::try_into(n).ok())
				.unwrap_or_else(Zero::zero);

			ensure!(
				currency_amount >= min_currencies[i],
				Error::<T>::InsufficientCurrencyAmount
			);
			ensure!(
				token_amount >= min_tokens[i],
				Error::<T>::InsufficientTokens
			);

			// Update total liquidity pool token supply of token_id
			TotalSupplies::<T>::try_mutate(
				pool_account,
				token_id,
				|total_supply| -> DispatchResult {
					*total_supply = total_liquidity
						.checked_sub(liquidity)
						.ok_or(Error::<T>::Overflow)?;
					Ok(())
				},
			)?;

			// Update currency reserve size for token_id
			CurrencyReserves::<T>::try_mutate(
				pool_account,
				token_id,
				|currency_reserve| -> DispatchResult {
					*currency_reserve = currency_reserve
						.checked_sub(currency_amount)
						.ok_or(Error::<T>::Overflow)?;
					Ok(())
				},
			)?;

			// Update totalCurrency and tokenAmounts
			total_currency = total_currency.saturating_add(currency_amount);
			token_amounts[i] = token_amount;
			currency_amounts[i] = currency_amount;
		}

		// Burn liquidity pool tokens for offchain supplies
		pallet_token_multi::Pallet::<T>::do_batch_burn(
			&pool.vault,
			&pool.lp_token,
			&pool.vault,
			token_ids.clone(),
			liquidities,
		)?;

		// Transfer total currency
		pallet_token_fungible::Pallet::<T>::do_transfer_from(
			&pool.vault,
			&pool.currency,
			&pool.vault,
			who,
			total_currency,
		)?;

		// Transfer all tokens to provider
		pallet_token_multi::Pallet::<T>::do_batch_transfer_from(
			&pool.vault,
			&pool.lp_token,
			&pool.vault,
			who,
			token_ids.clone(),
			token_amounts.clone(),
		)?;

		Self::deposit_event(Event::LiquidityRemoved(
			who.clone(),
			pool_account.clone(),
			token_ids,
			token_amounts,
			currency_amounts,
		));

		Ok(())
	}

	/// Pricing function used for converting between currency token to Tokens.
	///
	/// - `amount_out`: Amount of Tokens being bought.
	/// - `reserve_in`: Amount of currency tokens in pool reserves.
	/// - `reserve_out`: Amount of Tokens in pool reserves.
	/// Return the price Amount of currency tokens to send to pool.
	pub fn get_buy_price(
		amount_out: Balance,
		reserve_in: Balance,
		reserve_out: Balance,
	) -> Result<Balance, DispatchError> {
		ensure!(
			reserve_in > Zero::zero() && reserve_out > Zero::zero(),
			Error::<T>::EmptyReserve
		);

		let numerator: U256 = U256::from(reserve_in)
			.saturating_mul(U256::from(amount_out))
			.saturating_mul(U256::from(1000u128));
		let denominator: U256 = (U256::from(reserve_out).saturating_sub(U256::from(amount_out)))
			.saturating_mul(U256::from(995u128));
		let (amount_in, _) = Self::div_round(numerator, denominator);

		Ok(amount_in)
	}

	/// Pricing function used for converting Tokens to currency token.
	///
	/// - `amount_in`: Amount of Tokens being sold.
	/// - `reserve_in`: Amount of Tokens in pool reserves.
	/// - `reserve_out`: Amount of currency tokens in pool reserves.
	/// Return the price Amount of currency tokens to receive from pool.
	pub fn get_sell_price(
		amount_in: Balance,
		reserve_in: Balance,
		reserve_out: Balance,
	) -> Result<Balance, DispatchError> {
		ensure!(
			reserve_in > Zero::zero() && reserve_out > Zero::zero(),
			Error::<T>::EmptyReserve
		);

		let amount_in_with_fee: U256 = U256::from(amount_in).saturating_mul(U256::from(995u128));
		let numerator: U256 =
			U256::from(amount_in_with_fee).saturating_mul(U256::from(reserve_out));
		let denominator: U256 = (U256::from(reserve_in).saturating_mul(U256::from(1000u128)))
			.saturating_add(amount_in_with_fee);

		let amount_out = numerator
			.checked_div(denominator)
			.and_then(|n| TryInto::<Balance>::try_into(n).ok())
			.unwrap_or_else(Zero::zero);

		Ok(amount_out)
	}

	fn get_token_reserves(
		vault: &T::AccountId,
		token_account: &T::AccountId,
		token_ids: Vec<TokenId>,
	) -> Result<Vec<Balance>, DispatchError> {
		let n = token_ids.len();

		if n == 1 {
			let mut token_reserves = vec![Balance::from(0u128); 1];
			token_reserves[0] =
				pallet_token_multi::Pallet::<T>::balance_of(vault, token_account, token_ids[0]);
			Ok(token_reserves)
		} else {
			let accounts = vec![vault.clone(); n];

			for i in 1..n {
				ensure!(
					token_ids[i - 1] < token_ids[i],
					Error::<T>::UnsortedOrDuplicateTokenIds
				);
			}

			pallet_token_multi::Pallet::<T>::balance_of_batch(&token_account, &accounts, token_ids)
		}
	}

	/// Divides two numbers and add 1 if there is a rounding error
	fn div_round(numerator: U256, denominator: U256) -> (Balance, bool) {
		let remainder = numerator.checked_rem(denominator).unwrap();
		if remainder.is_zero() {
			(
				numerator
					.checked_div(denominator)
					.and_then(|n| TryInto::<Balance>::try_into(n).ok())
					.unwrap_or_else(Zero::zero),
				false,
			)
		} else {
			(
				numerator
					.checked_div(denominator)
					.and_then(|r| r.checked_add(U256::one()))
					.and_then(|n| TryInto::<Balance>::try_into(n).ok())
					.unwrap_or_else(Zero::zero),
				true,
			)
		}
	}
}
