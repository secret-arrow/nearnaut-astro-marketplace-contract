use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LookupMap, UnorderedMap, UnorderedSet};
use near_sdk::json_types::{U128, U64};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{
    assert_one_yocto, env, ext_contract, near_bindgen, serde_json::json, AccountId, Balance,
    BorshStorageKey, CryptoHash, Gas, PanicOnDefault, Promise, Timestamp,
};
use near_sdk::{is_promise_success, promise_result_as_success};
use std::collections::HashMap;

use crate::external::*;

mod external;
mod nft_callbacks;

const GAS_FOR_NFT_TRANSFER: Gas = Gas(20_000_000_000_000);
const BASE_GAS: Gas = Gas(5_000_000_000_000);
const GAS_FOR_ROYALTIES: Gas = Gas(BASE_GAS.0 * 10u64);
const NO_DEPOSIT: Balance = 0;
const MAX_PRICE: Balance = 1_000_000_000 * 10u128.pow(24);

pub const STORAGE_ADD_MARKET_DATA: u128 = 8590000000000000000000;

pub type PayoutHashMap = HashMap<AccountId, U128>;
pub type ContractAndTokenId = String;
pub type ContractAccountIdTokenId = String;
pub type TokenId = String;
pub type TokenSeriesId = String;
pub type TimestampSec = u32;

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct Payout {
    pub payout: PayoutHashMap,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct Bid {
    pub bidder_id: AccountId,
    pub price: U128,
}

pub type Bids = Vec<Bid>;

fn near_account() -> AccountId {
    AccountId::new_unchecked("near".to_string())
}

const DELIMETER: &str = "||";
const NEAR: &str = "near";

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct MarketData {
    pub owner_id: AccountId,
    pub approval_id: u64,
    pub nft_contract_id: AccountId,
    pub token_id: TokenId,
    pub ft_token_id: AccountId, // "near" for NEAR token
    pub price: u128,            // if auction, price becomes starting price
    pub bids: Option<Bids>,
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub is_auction: Option<bool>,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct OfferData {
    pub buyer_id: AccountId,
    pub nft_contract_id: AccountId,
    pub token_id: TokenId,
    pub ft_token_id: AccountId, // "near" for NEAR token
    pub price: u128,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct OfferDataJson {
    buyer_id: AccountId,
    nft_contract_id: AccountId,
    token_id: TokenId,
    ft_token_id: AccountId, // "near" for NEAR token
    price: U128,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct MarketDataJson {
    owner_id: AccountId,
    approval_id: U64,
    nft_contract_id: AccountId,
    token_id: TokenId,
    ft_token_id: AccountId, // "near" for NEAR token
    price: U128,
    bids: Option<Bids>,
    started_at: Option<U64>,
    ended_at: Option<U64>,
    is_auction: Option<bool>,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    pub owner_id: AccountId,
    pub treasury_id: AccountId,
    pub market: UnorderedMap<ContractAndTokenId, MarketData>,
    pub approved_ft_token_ids: UnorderedSet<AccountId>,
    pub approved_nft_contract_ids: UnorderedSet<AccountId>,
    pub storage_deposits: LookupMap<AccountId, Balance>,
    pub by_owner_id: LookupMap<AccountId, UnorderedSet<TokenId>>,
    pub offers: UnorderedMap<ContractAccountIdTokenId, OfferData>,
    pub transaction_fee: u16
}

#[derive(BorshStorageKey, BorshSerialize)]
pub enum StorageKey {
    Market,
    FTTokenIds,
    NFTContractIds,
    StorageDeposits,
    ByOwnerId,
    ByOwnerIdInner {
        account_id_hash: CryptoHash,
    },
    Offers
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(
        owner_id: AccountId,
        treasury_id: AccountId,
        approved_ft_token_ids: Option<Vec<AccountId>>,
        approved_nft_contract_ids: Option<Vec<AccountId>>,
    ) -> Self {
        let mut this = Self {
            owner_id: owner_id.into(),
            treasury_id: treasury_id.into(),            
            market: UnorderedMap::new(StorageKey::Market),
            approved_ft_token_ids: UnorderedSet::new(StorageKey::FTTokenIds),
            approved_nft_contract_ids: UnorderedSet::new(StorageKey::NFTContractIds),
            storage_deposits: LookupMap::new(StorageKey::StorageDeposits),
            by_owner_id: LookupMap::new(StorageKey::ByOwnerId),
            offers: UnorderedMap::new(StorageKey::Offers),
            transaction_fee: 200
        };

        this.approved_ft_token_ids.insert(&near_account());

        add_accounts(approved_ft_token_ids, &mut this.approved_ft_token_ids);
        add_accounts(
            approved_nft_contract_ids,
            &mut this.approved_nft_contract_ids,
        );

        this
    }
    // Changing treasury & ownership

    #[payable]
    pub fn set_treasury(&mut self, treasury_id: AccountId) {
        assert_one_yocto();
        self.assert_owner();
        self.treasury_id = treasury_id;
    }

    #[payable]
    pub fn set_transaction_fee(&mut self, next_fee: u16) {
        assert_one_yocto();
        self.assert_owner();

        assert!(next_fee < 10_000, "Error: fee is higher than 10_000");

        self.transaction_fee = next_fee;
        return;
        
    }

    pub fn get_transaction_fee(&self) -> u16 {
        self.transaction_fee
    }

    #[payable]
    pub fn transfer_ownership(&mut self, owner_id: AccountId) {
        assert_one_yocto();
        self.assert_owner();
        self.owner_id = owner_id;
    }

    // Approved contracts
    #[payable]
    pub fn add_approved_nft_contract_ids(&mut self, nft_contract_ids: Vec<AccountId>) {
        assert_one_yocto();
        self.assert_owner();
        add_accounts(Some(nft_contract_ids), &mut self.approved_nft_contract_ids);
    }

    #[payable]
    pub fn remove_approved_nft_contract_ids(&mut self, nft_contract_ids: Vec<AccountId>) {
        assert_one_yocto();
        self.assert_owner();
        remove_accounts(Some(nft_contract_ids), &mut self.approved_nft_contract_ids);
    }

    #[payable]
    pub fn add_approved_ft_token_ids(&mut self, ft_token_ids: Vec<AccountId>) {
        assert_one_yocto();
        self.assert_owner();
        add_accounts(Some(ft_token_ids), &mut self.approved_ft_token_ids);
    }

    // Buy & Payment

    #[payable]
    pub fn buy(
        &mut self,
        nft_contract_id: AccountId,
        token_id: TokenId,
        ft_token_id: Option<AccountId>,
        price: Option<U128>,
    ) {
        let contract_and_token_id = format!("{}{}{}", &nft_contract_id, DELIMETER, token_id);

        let market_data: MarketData = self.market.get(&contract_and_token_id).expect("Error: Market data does not exist");

        let buyer_id = env::predecessor_account_id();

        assert_ne!(
            buyer_id, market_data.owner_id,
            "Error: Cannot buy your own sale"
        );

        // only NEAR supported for now
        assert_eq!(
            market_data.ft_token_id.to_string(),
            NEAR,
            "Error: NEAR support only"
        );

        if ft_token_id.is_some() {
            assert_eq!(
                ft_token_id.unwrap().to_string(),
                market_data.ft_token_id.to_string()
            )
        }
        if price.is_some() {
            assert_eq!(price.unwrap().0, market_data.price);
        }

        let price = market_data.price;

        if let Some(auction) = market_data.is_auction {
            assert_eq!(auction, false, "Error: the NFT is on auction");
        }

        assert!(
            env::attached_deposit() >= price,
            "Error: Attached deposit is less than price {}",
            price
        );

        self.internal_process_purchase(nft_contract_id.into(), token_id, buyer_id, price);
    }

    fn internal_process_purchase(
        &mut self,
        nft_contract_id: AccountId,
        token_id: TokenId,
        buyer_id: AccountId,
        price: u128,
    ) -> Promise {
        let market_data = self
            .internal_delete_market_data(&nft_contract_id, &token_id)
            .expect("Error: Sale does not exist");

        ext_contract::nft_transfer_payout(
            buyer_id.clone(),
            token_id,
            Some(market_data.approval_id),
            Some(price.into()),
            Some(10u32), // max length payout
            nft_contract_id,
            1,
            GAS_FOR_NFT_TRANSFER,
        )
        .then(ext_self::resolve_purchase(
            buyer_id,
            market_data,
            price.into(),
            env::current_account_id(),
            NO_DEPOSIT,
            GAS_FOR_ROYALTIES,
        ))
    }

    #[private]
    pub fn resolve_purchase(
        &mut self,
        buyer_id: AccountId,
        market_data: MarketData,
        price: U128,
    ) -> U128 {
        let payout_option = promise_result_as_success().and_then(|value| {
            let parsed_payout = near_sdk::serde_json::from_slice::<PayoutHashMap>(&value);
            if parsed_payout.is_err() {
                near_sdk::serde_json::from_slice::<Payout>(&value)
                    .ok()
                    .and_then(|payout| {
                        let mut remainder = price.0;
                        for &value in payout.payout.values() {
                            remainder = remainder.checked_sub(value.0)?;
                        }
                        if remainder <= 100 {
                            Some(payout.payout)
                        } else {
                            None
                        }
                    })
            } else {
                parsed_payout
                    .ok()
                    .and_then(|payout| {
                        let mut remainder = price.0;
                        for &value in payout.values() {
                            remainder = remainder.checked_sub(value.0)?;
                        }
                        if remainder <= 100 {
                            Some(payout)
                        } else {
                            None
                        }
                    })
            }
        });
        let payout = if let Some(payout_option) = payout_option {
            payout_option
        } else {
            // leave function and return all FTs in ft_resolve_transfer
            if !is_promise_success() {
                if market_data.ft_token_id == near_account() {
                    Promise::new(buyer_id.clone()).transfer(u128::from(market_data.price));
                }
            
                env::log_str(
                    &json!({
                        "event": "resolve_purchase_fail",
                        "params": {
                            "owner_id": market_data.owner_id,
                            "nft_contract_id": market_data.nft_contract_id,
                            "token_id": market_data.token_id,
                            "ft_token_id": market_data.ft_token_id,
                            "price": price,
                            "buyer_id": buyer_id,
                        }
                    })
                    .to_string(),
                );
            }  else if market_data.ft_token_id == near_account() {
                let treasury_fee = price.0 * self.transaction_fee as u128 / 10_000u128;
                Promise::new(market_data.owner_id.clone()).transfer(price.0 - treasury_fee);
                if treasury_fee > 0 {
                    Promise::new(self.treasury_id.clone()).transfer(treasury_fee);
                }

                env::log_str(
                    &json!({
                    "event": "resolve_purchase",
                    "params": {
                        "owner_id": &market_data.owner_id,
                        "nft_contract_id": &market_data.nft_contract_id,
                        "token_id": &market_data.token_id,
                        "ft_token_id": market_data.ft_token_id,
                        "price": price,
                        "buyer_id": buyer_id,
                    }
                })
                        .to_string(),
                );
            }
            
            return price;
        };

        // Payout (transfer to royalties and seller)
        if market_data.ft_token_id == near_account() {
            // 5% fee for treasury
            let treasury_fee = price.0 * self.transaction_fee as u128 / 10_000u128;

            for (receiver_id, amount) in payout {
                if receiver_id == market_data.owner_id {
                    Promise::new(receiver_id).transfer(amount.0 - treasury_fee);
                    if treasury_fee != 0 {
                        Promise::new(self.treasury_id.clone()).transfer(treasury_fee);
                    }
                } else {
                    Promise::new(receiver_id).transfer(amount.0);
                }
            }
            env::log_str(
                &json!({
                    "event": "resolve_purchase",
                    "params": {
                        "owner_id": &market_data.owner_id,
                        "nft_contract_id": &market_data.nft_contract_id,
                        "token_id": &market_data.token_id,
                        "ft_token_id": market_data.ft_token_id,
                        "price": price,
                        "buyer_id": buyer_id,
                    }
                })
                .to_string(),
            );

            return price;
        } else {
            U128(0)
        }
    }

    // Offer

    #[payable]
    pub fn add_offer(
        &mut self,
        nft_contract_id: AccountId,
        token_id: TokenId,
        ft_token_id: AccountId,
        price: U128,
    ) {

        assert!(
            self.approved_nft_contract_ids.contains(&nft_contract_id),
            "Error: offer series for Astro NFT only"
        );

        assert_eq!(
            env::attached_deposit(),
            price.0,
            "Error: Attached deposit != price"
        );

        assert_eq!(
            ft_token_id.to_string(),
            "near",
            "Error: Only NEAR is supported"
        );

        let buyer_id = env::predecessor_account_id();
        let offer_data = self.internal_delete_offer(
            nft_contract_id.clone().into(),
            buyer_id.clone(),
            token_id.clone(),
        );

        if offer_data.is_some() {
            Promise::new(buyer_id.clone()).transfer(offer_data.unwrap().price);
        }

        let storage_amount = self.storage_minimum_balance().0;
        let owner_paid_storage = self.storage_deposits.get(&buyer_id).unwrap_or(0);
        let signer_storage_required =
            (self.get_supply_by_owner_id(buyer_id.clone()).0 + 1) as u128 * storage_amount;

        assert!(
            owner_paid_storage >= signer_storage_required,
            "Insufficient storage paid: {}, for {} offer at {} rate of per offer",
            owner_paid_storage,
            signer_storage_required / storage_amount,
            storage_amount,
        );

        self.internal_add_offer(
            nft_contract_id.clone().into(),
            token_id.clone(),
            ft_token_id.clone(),
            price,
            buyer_id.clone(),
        );

        env::log_str(
            &json!({
                "event": "add_offer",
                "params": {
                    "buyer_id": buyer_id,
                    "nft_contract_id": nft_contract_id,
                    "token_id": token_id,
                    "ft_token_id": ft_token_id,
                    "price": price,
                }
            })
            .to_string(),
        );
    }

    fn internal_add_offer(
        &mut self,
        nft_contract_id: AccountId,
        token_id: TokenId,
        ft_token_id: AccountId,
        price: U128,
        buyer_id: AccountId,
    ) {

        let contract_account_id_token_id = make_triple(&nft_contract_id, &buyer_id, &token_id);
        self.offers.insert(
            &contract_account_id_token_id,
            &OfferData {
                buyer_id: buyer_id.clone().into(),
                nft_contract_id: nft_contract_id.into(),
                token_id: token_id.clone(),
                ft_token_id: ft_token_id.into(),
                price: price.into(),
            },
        );

        let mut token_ids = self.by_owner_id.get(&buyer_id).unwrap_or_else(|| {
            UnorderedSet::new(
                StorageKey::ByOwnerIdInner {
                    account_id_hash: hash_account_id(&buyer_id),
                }
                .try_to_vec()
                .unwrap(),
            )
        });
        token_ids.insert(&contract_account_id_token_id);
        self.by_owner_id.insert(&buyer_id, &token_ids);
    }

    fn internal_delete_offer(
        &mut self,
        nft_contract_id: AccountId,
        buyer_id: AccountId,
        token_id: TokenId,
    ) -> Option<OfferData> {
        let contract_account_id_token_id = make_triple(&nft_contract_id, &buyer_id, &token_id);
        let offer_data = self.offers.remove(&contract_account_id_token_id);

        match offer_data {
            Some(offer) => {
                let by_owner_id = self
                    .by_owner_id
                    .get(&offer.buyer_id);
                if let Some(mut by_owner_id) = by_owner_id {
                    by_owner_id.remove(&contract_account_id_token_id);
                    if by_owner_id.is_empty() {
                        self.by_owner_id.remove(&offer.buyer_id);
                    } else {
                        self.by_owner_id.insert(&offer.buyer_id, &by_owner_id);
                    }
                }
                return Some(offer);
            }
            None => return None,
        };
    }

    #[payable]
    pub fn delete_offer(
        &mut self,
        nft_contract_id: AccountId,
        token_id: TokenId,
    ) {
        assert_one_yocto();

        let buyer_id = env::predecessor_account_id();
        let contract_account_id_token_id = make_triple(&nft_contract_id, &buyer_id, &token_id);

        let offer_data = self
            .offers
            .get(&contract_account_id_token_id)
            .expect("Error: Offer does not exist");

        assert_eq!(offer_data.token_id.clone(), token_id);

        assert_eq!(
            offer_data.buyer_id, buyer_id,
            "Error: Caller not offer's buyer"
        );

        self.internal_delete_offer(
            nft_contract_id.clone().into(),
            buyer_id.clone(),
            token_id.clone(),
        )
        .expect("Error: Offer not found");

        Promise::new(offer_data.buyer_id).transfer(offer_data.price);

        env::log_str(
            &json!({
                "event": "delete_offer",
                "params": {
                    "nft_contract_id": nft_contract_id,
                    "buyer_id": buyer_id,
                    "token_id": token_id,
                }
            })
            .to_string(),
        );
    }

    pub fn get_offer(
        &self,
        nft_contract_id: AccountId,
        buyer_id: AccountId,
        token_id: TokenId,
    ) -> OfferDataJson {

        let contract_account_id_token_id = make_triple(&nft_contract_id, &buyer_id, &token_id);

        let offer_data = self
            .offers
            .get(&contract_account_id_token_id)
            .expect("Error: Offer does not exist");

        assert_eq!(offer_data.token_id.clone(), token_id);

        OfferDataJson {
            buyer_id: offer_data.buyer_id,
            nft_contract_id: offer_data.nft_contract_id,
            token_id: offer_data.token_id,
            ft_token_id: offer_data.ft_token_id,
            price: U128(offer_data.price),
        }
    }

    fn internal_accept_offer(
        &mut self,
        nft_contract_id: AccountId,
        buyer_id: AccountId,
        token_id: TokenId,
        seller_id: AccountId,
        approval_id: u64,
        price: u128,
    ) -> Promise {
        let contract_account_id_token_id = make_triple(&nft_contract_id, &buyer_id, &token_id);

        self.internal_delete_market_data(&nft_contract_id, &token_id);

        let offer_data = self
            .offers
            .get(&contract_account_id_token_id)
            .expect("Error: Offer does not exist");

        assert_eq!(offer_data.token_id.clone(), token_id);
        assert_eq!(offer_data.price, price);

        let offer_data = self
            .internal_delete_offer(
                nft_contract_id.clone().into(),
                buyer_id.clone(),
                token_id.clone(),
            )
            .expect("Error: Offer does not exist");

        ext_contract::nft_transfer_payout(
            offer_data.buyer_id.clone(),
            token_id.clone(),
            Some(approval_id),
            Some(U128::from(offer_data.price)),
            Some(10u32), // max length payout
            nft_contract_id,
            1,
            GAS_FOR_NFT_TRANSFER,
        )
        .then(ext_self::resolve_offer(
            seller_id,
            offer_data,
            token_id,
            env::current_account_id(),
            NO_DEPOSIT,
            GAS_FOR_ROYALTIES,
        ))
    }

    #[private]
    pub fn resolve_offer(
        &mut self,
        seller_id: AccountId,
        offer_data: OfferData,
        token_id: TokenId,
    ) -> U128 {
        let payout_option = promise_result_as_success().and_then(|value| {
            // None means a bad payout from bad NFT contract
            let parsed_payout = near_sdk::serde_json::from_slice::<PayoutHashMap>(&value);
            if parsed_payout.is_err() {
                near_sdk::serde_json::from_slice::<Payout>(&value)
                    .ok()
                    .and_then(|payout| {
                        let mut remainder = offer_data.price;
                        for &value in payout.payout.values() {
                            remainder = remainder.checked_sub(value.0)?;
                        }
                        if remainder <= 100 {
                            Some(payout.payout)
                        } else {
                            None
                        }
                    })
            } else {
                parsed_payout.ok().and_then(|payout| {
                    let mut remainder = offer_data.price;
                    for &value in payout.values() {
                        remainder = remainder.checked_sub(value.0)?;
                    }
                    if remainder <= 100 {
                        Some(payout)
                    } else {
                        None
                    }
                })
            }
        });

        let payout = if let Some(payout_option) = payout_option {
            payout_option
        } else {
            if !is_promise_success() {
                if offer_data.ft_token_id == near_account() {
                    Promise::new(offer_data.buyer_id.clone()).transfer(u128::from(offer_data.price));
                }
                // leave function and return all FTs in ft_resolve_transfer
                env::log_str(
                    &json!({
                        "event": "resolve_purchase_fail",
                        "params": {
                            "owner_id": seller_id,
                            "nft_contract_id": offer_data.nft_contract_id,
                            "token_id": token_id,
                            "ft_token_id": offer_data.ft_token_id,
                            "price": offer_data.price.to_string(),
                            "buyer_id": offer_data.buyer_id,
                            "is_offer": true,
                        }
                    })
                    .to_string(),
                );
            } else if offer_data.ft_token_id == near_account() {
                let treasury_fee =
                    offer_data.price as u128 * self.transaction_fee as u128 / 10_000u128;
					Promise::new(seller_id.clone()).transfer(offer_data.price - treasury_fee);
                if treasury_fee > 0 {
                    Promise::new(self.treasury_id.clone()).transfer(treasury_fee);
                }

                env::log_str(
                    &json!({
                        "event": "resolve_purchase",
                        "params": {
                            "owner_id": seller_id,
                            "nft_contract_id": &offer_data.nft_contract_id,
                            "token_id": &token_id,
                            "ft_token_id": offer_data.ft_token_id,
                            "price": offer_data.price.to_string(),
                            "buyer_id": offer_data.buyer_id,
                            "is_offer": true,
                        }
                    })
                    .to_string(),
                );
            }
            
            return offer_data.price.into();
        };

        // Payout (transfer to royalties and seller)
        if offer_data.ft_token_id == near_account() {
            // 5% fee for treasury
            let treasury_fee =
                offer_data.price as u128 * self.transaction_fee as u128 / 10_000u128;

            for (receiver_id, amount) in payout {
                if receiver_id == seller_id {
                    Promise::new(receiver_id).transfer(amount.0 - treasury_fee);
                    if treasury_fee != 0 {
                        Promise::new(self.treasury_id.clone()).transfer(treasury_fee);
                    }
                } else {
                    Promise::new(receiver_id).transfer(amount.0);
                }
            }

            env::log_str(
                &json!({
                    "event": "resolve_purchase",
                    "params": {
                        "owner_id": seller_id,
                        "nft_contract_id": &offer_data.nft_contract_id,
                        "token_id": &token_id,
                        "ft_token_id": offer_data.ft_token_id,
                        "price": offer_data.price.to_string(),
                        "buyer_id": offer_data.buyer_id,
                        "is_offer": true,
                    }
                })
                .to_string(),
            );

            return offer_data.price.into();
        } else {
            U128(0)
        }
    }

    // Auction bids
    #[payable]
    pub fn add_bid(
        &mut self,
        nft_contract_id: AccountId,
        ft_token_id: AccountId,
        token_id: TokenId,
        amount: U128,
    ) {
        let contract_and_token_id = format!("{}{}{}", &nft_contract_id, DELIMETER, token_id);
        let mut market_data = self
            .market
            .get(&contract_and_token_id)
            .expect("Error: Token id does not exist");

        let bidder_id = env::predecessor_account_id();

        let current_time = env::block_timestamp();
		if market_data.started_at.is_some() {
            assert!(
                current_time >= market_data.started_at.unwrap(),
                "Error: Sale has not started yet"
            );
        }

        if market_data.ended_at.is_some() {
            assert!(
                current_time <= market_data.ended_at.unwrap(),
                "Error: Sale has ended"
            );
        }
		
		assert_ne!(market_data.owner_id, bidder_id, "Error: Owner cannot bid their own token");

        assert!(
            env::attached_deposit() >= amount.into(),
            "Error: attached deposit is less than amount"
        );

        assert_eq!(ft_token_id.to_string(), "near", "Error: Only support NEAR");
		
		let storage_amount = self.storage_minimum_balance().0;
        let owner_paid_storage = self.storage_deposits.get(&bidder_id).unwrap_or(0);
        let signer_storage_required =
            (self.get_supply_by_owner_id(bidder_id.clone()).0 + 1) as u128 * storage_amount;

        assert!(
            owner_paid_storage >= signer_storage_required,
            "Insufficient storage paid: {}, for {} bid at {} rate of per bid",
            owner_paid_storage,
            signer_storage_required / storage_amount,
            storage_amount,
        );

        let new_bid = Bid {
            bidder_id: bidder_id.clone(),
            price: amount.into(),
        };

        let mut bids = market_data.bids.unwrap_or(Vec::new());

        if !bids.is_empty() {
            let current_bid = &bids[bids.len() - 1];

            assert!(
                amount.0 > current_bid.price.0,
                "Error: Can't pay less than or equal to current bid price: {:?}",
                current_bid.price
            );

            assert!(
                amount.0 >= market_data.price,
                "Error: Can't pay less than starting price: {:?}",
                U128(market_data.price)
            );

            // Retain all elements except account_id
            bids.retain(|bid| {
              if bid.bidder_id == bidder_id {
                // refund
                Promise::new(bid.bidder_id.clone()).transfer(bid.price.0);
              }

              bid.bidder_id != bidder_id
            });
        } else {
            assert!(
                amount.0 >= market_data.price,
                "Error: Can't pay less than starting price: {}",
                market_data.price
            );
        }

        bids.push(new_bid);
        market_data.bids = Some(bids);
        self.market.insert(&contract_and_token_id, &market_data);

        env::log_str(
            &json!({
                "event": "add_bid",
                "params": {
                    "bidder_id": bidder_id,
                    "nft_contract_id": nft_contract_id,
                    "token_id": token_id,
                    "ft_token_id": ft_token_id,
                    "amount": amount,
                }
            })
            .to_string(),
        );
    }

    #[payable]
    pub fn accept_bid(&mut self, nft_contract_id: AccountId, token_id: TokenId) {
        assert_one_yocto();
        let contract_and_token_id = format!("{}{}{}", &nft_contract_id, DELIMETER, token_id);
        let mut market_data = self
            .market
            .get(&contract_and_token_id)
            .expect("Error: Token id does not exist");

        assert_eq!(
            market_data.owner_id,
            env::predecessor_account_id(),
            "Error: Only seller can call accept_bid"
        );

        let mut bids = market_data.bids.unwrap();
		
		assert!(!bids.is_empty(), "Astro: Cannot accept bid with empty bid");
		
        let selected_bid = bids.remove(bids.len() - 1);
		
		// refund all except selected bids
        for bid in &bids {
          // refund
          Promise::new(bid.bidder_id.clone()).transfer(bid.price.0);
        }
        bids.clear();
		
        market_data.bids = Some(bids);
        self.market.insert(&contract_and_token_id, &market_data);

        self.internal_process_purchase(
            market_data.nft_contract_id,
            token_id,
            selected_bid.bidder_id.clone(),
            selected_bid.price.clone().0,
        );
    }
	
	
	fn internal_cancel_bid(&mut self, nft_contract_id: AccountId, token_id: TokenId, account_id: AccountId) {
      let contract_and_token_id = format!("{}{}{}", &nft_contract_id, DELIMETER, token_id);
      let mut market_data = self
        .market
        .get(&contract_and_token_id)
        .expect("Error: Token id does not exist");

      let mut bids = market_data.bids.unwrap();

      assert!(
        !bids.is_empty(),
        "Error: Bids data does not exist"
      );

      // Retain all elements except account_id
      bids.retain(|bid| {
        if bid.bidder_id == account_id {
          // refund
          Promise::new(bid.bidder_id.clone()).transfer(bid.price.0);
        }

        bid.bidder_id != account_id
      });

      market_data.bids = Some(bids);
      self.market.insert(&contract_and_token_id, &market_data);

      env::log_str(
        &json!({
          "type": "cancel_bid",
          "params": {
            "bidder_id": account_id, "nft_contract_id": nft_contract_id, "token_id": token_id
          }
        })
        .to_string(),
      );
    }

    #[payable]
    pub fn cancel_bid(&mut self, nft_contract_id: AccountId, token_id: TokenId, account_id: AccountId) {
      assert_one_yocto();
      let contract_and_token_id = format!("{}{}{}", &nft_contract_id, DELIMETER, token_id);
      let market_data = self
        .market
        .get(&contract_and_token_id)
        .expect("Error: Token id does not exist");

      let bids = market_data.bids.unwrap();

      assert!(
        !bids.is_empty(),
        "Error: Bids data does not exist"
      );

      for x in 0..bids.len() {
        if bids[x].bidder_id == account_id {
          assert!(
            [bids[x].bidder_id.clone(), self.owner_id.clone()]
              .contains(&env::predecessor_account_id()),
              "Error: Bidder or owner only"
          );
        }
      }

      self.internal_cancel_bid(nft_contract_id, token_id, account_id,);
    }


    // Market Data functions

    #[payable]
    pub fn update_market_data(
        &mut self,
        nft_contract_id: AccountId,
        token_id: TokenId,
        ft_token_id: AccountId,
        price: U128,
    ) {
        assert_one_yocto();
        let contract_and_token_id = format!("{}{}{}", nft_contract_id, DELIMETER, token_id);
        let mut market_data = self
            .market
            .get(&contract_and_token_id)
            .expect("Error: Token id does not exist ");

        assert_eq!(
            market_data.owner_id,
            env::predecessor_account_id(),
            "Error: Seller only"
        );

        assert_eq!(
            ft_token_id, market_data.ft_token_id,
            "Error: ft_token_id differs"
        ); // sanity check

        assert!(
            price.0 < MAX_PRICE,
            "Error: price higher than {}",
            MAX_PRICE
        );

        market_data.price = price.into();
        self.market.insert(&contract_and_token_id, &market_data);

        env::log_str(
            &json!({
                "event": "update_market_data",
                "params": {
                    "owner_id": market_data.owner_id,
                    "nft_contract_id": nft_contract_id,
                    "token_id": token_id,
                    "ft_token_id": ft_token_id,
                    "price": price,
                }
            })
            .to_string(),
        );
    }

    fn internal_add_market_data(
        &mut self,
        owner_id: AccountId,
        approval_id: u64,
        nft_contract_id: AccountId,
        token_id: TokenId,
        ft_token_id: AccountId,
        price: U128,
        started_at: Option<U64>,
        ended_at: Option<U64>,
        is_auction: Option<bool>,
    ) {
        let contract_and_token_id = format!("{}{}{}", nft_contract_id, DELIMETER, token_id);

        let bids: Option<Bids> = match is_auction {
            Some(u) => {
                if u {
                    Some(Vec::new())
                } else {
                    None
                }
            }
            None => None,
        };

        let current_time: u64 = env::block_timestamp();

        if started_at.is_some() {
            assert!(started_at.unwrap().0 >= current_time);

            if ended_at.is_some() {
                assert!(started_at.unwrap().0 < ended_at.unwrap().0);
            }
        }

        if ended_at.is_some() {
            assert!(ended_at.unwrap().0 >= current_time);
        }

        assert!(
            price.0 < MAX_PRICE,
            "Error: price higher than {}",
            MAX_PRICE
        );

        self.market.insert(
            &contract_and_token_id,
            &MarketData {
                owner_id: owner_id.clone().into(),
                approval_id,
                nft_contract_id: nft_contract_id.clone().into(),
                token_id: token_id.clone(),
                ft_token_id: ft_token_id.clone(),
                price: price.into(),
                bids: bids,
                started_at: match started_at {
                    Some(x) => Some(x.0),
                    None => None,
                },
                ended_at: match ended_at {
                    Some(x) => Some(x.0),
                    None => None,
                },
                is_auction: is_auction,
            },
        );

        let mut token_ids = self.by_owner_id.get(&owner_id).unwrap_or_else(|| {
            UnorderedSet::new(
                StorageKey::ByOwnerIdInner {
                    account_id_hash: hash_account_id(&owner_id),
                }
                .try_to_vec()
                .unwrap(),
            )
        });

        token_ids.insert(&contract_and_token_id);

        self.by_owner_id.insert(&owner_id, &token_ids);

        env::log_str(
            &json!({
                "event": "add_market_data",
                "params": {
                    "owner_id": owner_id,
                    "approval_id": approval_id,
                    "nft_contract_id": nft_contract_id,
                    "token_id": token_id,
                    "ft_token_id": ft_token_id,
                    "price": price,
                    "started_at": started_at,
                    "ended_at": ended_at,
                    "is_auction": is_auction
                }
            })
            .to_string(),
        );
    }

    fn internal_delete_market_data(
        &mut self,
        nft_contract_id: &AccountId,
        token_id: &TokenId,
    ) -> Option<MarketData> {
        let contract_and_token_id = format!("{}{}{}", &nft_contract_id, DELIMETER, token_id);
        let market_data: Option<MarketData> =
            if let Some(market_data) = self.market.get(&contract_and_token_id) {
                self.market.remove(&contract_and_token_id);

                if let Some(ref bids) = market_data.bids {
                    for bid in bids {
                        Promise::new(bid.bidder_id.clone()).transfer(bid.price.0);
                    }
                };

                Some(market_data)
            } else {
                None
            };

        market_data.map(|market_data| {
            let by_owner_id = self
                .by_owner_id
                .get(&market_data.owner_id);
            if let Some(mut by_owner_id) = by_owner_id {
                by_owner_id.remove(&contract_and_token_id);
                if by_owner_id.is_empty() {
                self.by_owner_id.remove(&market_data.owner_id);
                } else {
                self.by_owner_id.insert(&market_data.owner_id, &by_owner_id);
                }
            }
            market_data
        })
    }

    #[payable]
    pub fn delete_market_data(&mut self, nft_contract_id: AccountId, token_id: TokenId) {
        assert_one_yocto();
        let contract_and_token_id = format!("{}{}{}", nft_contract_id, DELIMETER, token_id);

        let market_data: MarketData = self.market.get(&contract_and_token_id).expect("Error: Market data does not exist");

        assert!(
            [market_data.owner_id.clone(), self.owner_id.clone()]
                .contains(&env::predecessor_account_id()),
            "Error: Seller or owner only"
        );

        self.internal_delete_market_data(&nft_contract_id, &token_id);

        env::log_str(
            &json!({
                "event": "delete_market_data",
                "params": {
                    "owner_id": market_data.owner_id,
                    "nft_contract_id": nft_contract_id,
                    "token_id": token_id,
                }
            })
            .to_string(),
        );
    }

    // Storage

    #[payable]
    pub fn storage_deposit(&mut self, account_id: Option<AccountId>) {
        let storage_account_id = account_id
            .map(|a| a.into())
            .unwrap_or_else(env::predecessor_account_id);
        let deposit = env::attached_deposit();
        assert!(
            deposit >= STORAGE_ADD_MARKET_DATA,
            "Requires minimum deposit of {}",
            STORAGE_ADD_MARKET_DATA
        );

        let mut balance: u128 = self.storage_deposits.get(&storage_account_id).unwrap_or(0);
        balance += deposit;
        self.storage_deposits.insert(&storage_account_id, &balance);
    }

    #[payable]
    pub fn storage_withdraw(&mut self) {
        assert_one_yocto();
        let owner_id = env::predecessor_account_id();
        let mut amount = self.storage_deposits.remove(&owner_id).unwrap_or(0);
        let market_data_owner = self.by_owner_id.get(&owner_id);
        let len = market_data_owner.map(|s| s.len()).unwrap_or_default();
        let diff = u128::from(len) * STORAGE_ADD_MARKET_DATA;
        amount -= diff;
        if amount > 0 {
            Promise::new(owner_id.clone()).transfer(amount);
        }
        if diff > 0 {
            self.storage_deposits.insert(&owner_id, &diff);
        }
    }

    pub fn storage_minimum_balance(&self) -> U128 {
        U128(STORAGE_ADD_MARKET_DATA)
    }

    pub fn storage_balance_of(&self, account_id: AccountId) -> U128 {
        self.storage_deposits.get(&account_id).unwrap_or(0).into()
    }

    // View

    pub fn get_market_data(self, nft_contract_id: AccountId, token_id: TokenId) -> MarketDataJson {
        let contract_and_token_id = format!("{}{}{}", nft_contract_id, DELIMETER, token_id);
        let market_data: MarketData = self.market.get(&contract_and_token_id).expect("Error: Market data does not exist");
            
        let price = market_data.price;

        MarketDataJson {
            owner_id: market_data.owner_id,
            approval_id: market_data.approval_id.into(),
            nft_contract_id: market_data.nft_contract_id,
            token_id: market_data.token_id,
            ft_token_id: market_data.ft_token_id, // "near" for NEAR token
            price: price.into(),
            bids: market_data.bids,
            started_at: market_data.started_at.map(|x| x.into()),
            ended_at: market_data.ended_at.map(|x| x.into()),
            is_auction: market_data.is_auction,
        }
    }

    pub fn approved_ft_token_ids(&self) -> Vec<AccountId> {
        self.approved_ft_token_ids.to_vec()
    }

    pub fn approved_nft_contract_ids(&self) -> Vec<AccountId> {
        self.approved_nft_contract_ids.to_vec()
    }

    pub fn get_owner(&self) -> AccountId {
        self.owner_id.clone()
    }

    pub fn get_treasury(&self) -> AccountId {
        self.treasury_id.clone()
    }

    pub fn get_supply_by_owner_id(&self, account_id: AccountId) -> U64 {
        self.by_owner_id
            .get(&account_id)
            .map_or(0, |by_owner_id| by_owner_id.len())
            .into()
    }

    // private fn

    fn assert_owner(&self) {
        assert_eq!(
            env::predecessor_account_id(),
            self.owner_id,
            "Error: Owner only"
        )
    }
}

pub fn hash_account_id(account_id: &AccountId) -> CryptoHash {
    let mut hash = CryptoHash::default();
    hash.copy_from_slice(&env::sha256(account_id.as_bytes()));
    hash
}

pub fn hash_contract_account_id_token_id(
    contract_account_id_token_id: &ContractAccountIdTokenId,
) -> CryptoHash {
    let mut hash = CryptoHash::default();
    hash.copy_from_slice(&env::sha256(contract_account_id_token_id.as_bytes()));
    hash
}

pub fn to_sec(timestamp: Timestamp) -> TimestampSec {
    (timestamp / 10u64.pow(9)) as u32
}

#[ext_contract(ext_self)]
trait ExtSelf {
    fn resolve_purchase(
        &mut self,
        buyer_id: AccountId,
        market_data: MarketData,
        price: U128,
    ) -> Promise;

    fn resolve_offer(
        &mut self,
        seller_id: AccountId,
        offer_data: OfferData,
        token_id: TokenId,
    ) -> Promise;
}

fn add_accounts(accounts: Option<Vec<AccountId>>, set: &mut UnorderedSet<AccountId>) {
    accounts.map(|ids| {
        ids.iter().for_each(|id| {
            set.insert(id);
        })
    });
}

fn remove_accounts(accounts: Option<Vec<AccountId>>, set: &mut UnorderedSet<AccountId>) {
    accounts.map(|ids| {
        ids.iter().for_each(|id| {
            set.remove(id);
        })
    });
}

fn make_triple(nft_contract_id: &AccountId, buyer_id: &AccountId, token: &str) -> String {
    format!(
        "{}{}{}{}{}",
        nft_contract_id, DELIMETER, buyer_id, DELIMETER, token
    )
}