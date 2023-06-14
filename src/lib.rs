use std::collections::HashSet;

use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::UnorderedMap;
use near_sdk::json_types::U64;
use near_sdk::{env, near_bindgen, AccountId, Balance, BorshStorageKey, PanicOnDefault, Promise};
use serde::Serialize;

pub type Timestamp = u64; // ms
pub type TicketId = String;
pub type RewardId = u64;
pub type Points = u64;
pub const ONE_DAY: u64 = 86400000;

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct ArkanaCoreContract {
    owner: AccountId,
    daily_claim_points: u64,
    spin_wheel_price: u64,
    users: UnorderedMap<AccountId, User>,
    rewards: UnorderedMap<RewardId, Reward>,
    last_reward_id: RewardId,
    reward_tickets: UnorderedMap<RewardId, UnorderedMap<u64, Ticket>>,
    membership_contracts: HashSet<AccountId>,
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct Ticket {
    owner_id: AccountId,
    amount: u64,
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct Reward {
    title: String,
    price: Points,
    ended_at: Timestamp,
    total_tickets: u64,
    winner: Option<AccountId>,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize)]
pub struct User {
    points: u64,
    last_daily_claim: Timestamp,
    last_free_spinwheel: Timestamp,
    spinwheel_wr: u8,
}

#[derive(BorshSerialize, BorshStorageKey)]
enum StorageKey {
    Users,
    Rewards,
    RewardTickets,
}

#[near_bindgen]
impl ArkanaCoreContract {
    #[init]
    pub fn new(owner: AccountId, daily_claim_points: u64, spin_wheel_price: u64) -> Self {
        Self {
            owner,
            daily_claim_points,
            spin_wheel_price,
            users: UnorderedMap::new(StorageKey::Users),
            reward_tickets: UnorderedMap::new(StorageKey::RewardTickets),
            rewards: UnorderedMap::new(StorageKey::Rewards),
            last_reward_id: 0,
            membership_contracts: HashSet::new(),
        }
    }

    #[payable]
    pub fn create_reward(&mut self, title: String, price: U64, ended_at: U64) -> RewardId {
        let initial_storage_usage = env::storage_usage();
        let predecessor_id = env::predecessor_account_id();

        if predecessor_id != self.owner {
            panic!("Unauthorized");
        }

        self.rewards.insert(
            &(self.last_reward_id + 1),
            &Reward {
                title,
                price: price.0,
                ended_at: ended_at.0,
                total_tickets: 0,
                winner: None,
            },
        );

        self.last_reward_id += 1;

        refund_deposit(env::storage_usage() - initial_storage_usage, 0);

        self.last_reward_id
    }

    #[payable]
    pub fn buy_ticket(&mut self, reward_id: RewardId, amount: u64) {
        let predecessor_id = env::predecessor_account_id();

        let reward = self.rewards.get(&reward_id).unwrap();

        let mut user = self.users.get(&predecessor_id).unwrap();

        if user.points < reward.price * amount {
            panic!("Points insufficient");
        }

        user.points -= reward.price * amount;
    }

    pub fn finalize_reward(&mut self, reward_id: RewardId) {}

    #[payable]
    pub fn register_account(&mut self) {
        let initial_storage_usage = env::storage_usage();

        let predecessor_id = env::predecessor_account_id();
        if self.users.get(&predecessor_id).is_some() {
            panic!("Account already registered");
        }

        self.users.insert(
            &predecessor_id,
            &User {
                points: 0,
                last_daily_claim: 0,
                last_free_spinwheel: 0,
                spinwheel_wr: 0,
            },
        );

        refund_deposit(env::storage_usage() - initial_storage_usage, 0);
    }

    pub fn daily_claim_point(&mut self) -> Points {
        let account_id = env::predecessor_account_id();

        let mut user = self.users.get(&account_id).expect("User does not exist");

        let current_timestamp = env::block_timestamp_ms();
        let delta_ms = current_timestamp - user.last_daily_claim;

        if delta_ms < ONE_DAY {
            panic!(
                "Cannot claim, please wait {} seconds",
                milli_to_seconds(delta_ms)
            );
        }

        user.points += self.daily_claim_points;

        self.users.insert(&account_id, &user);

        user.points
    }

    #[payable]
    pub fn play_spin_wheel(&mut self, is_free: bool) -> Points {
        let predecessor_id = env::predecessor_account_id();

        let mut user = self.users.get(&predecessor_id).unwrap();

        if is_free {
            let current_timestamp = env::block_timestamp_ms();
            let delta_ms = current_timestamp - user.last_free_spinwheel;

            if delta_ms < ONE_DAY {
                panic!(
                    "Cannot play spin wheel for free, please wait {} seconds",
                    milli_to_seconds(delta_ms)
                );
            }
        } else {
            if user.points < self.spin_wheel_price {
                panic!("Cannot play, user points insufficient");
            }

            user.points -= self.spin_wheel_price;
        }

        let points = [1, 3, 7, 9, 12, 15];
        let weights = [
            50u16,
            80u16,
            70u16,
            20u16 + (user.spinwheel_wr as u16 * 3) / 10,
            10u16 + (user.spinwheel_wr as u16 * 2) / 10,
            2u16 + (user.spinwheel_wr as u16 * 1) / 10,
        ];

        let mut cumulative_weights: [u16; 6] = [0; 6];

        cumulative_weights[0] = weights[0];
        for i in 1..weights.len() {
            cumulative_weights[i] = weights[i] + cumulative_weights[i - 1];
        }

        let total_weights: u16 = weights[5]; // last index
        let random_number = get_random_number(0) as u16 % total_weights;
        let mut result = 0;

        for i in 0..weights.len() {
            if cumulative_weights[i] >= random_number {
                result = points[i];
                break;
            }
        }

        if result > 5 {
            user.spinwheel_wr = 0;
        } else {
            user.spinwheel_wr += 1;
        }

        user.points += result;

        self.users.insert(&predecessor_id, &user);

        result
    }

    pub fn add_membership_nft_contract(&mut self, contract_id: AccountId) {
        let predecessor_id = env::predecessor_account_id();

        if predecessor_id != self.owner {
            panic!("Unauthorized");
        }

        self.membership_contracts.insert(contract_id);
    }

    pub fn remove_membership_nft_contract(&mut self, contract_id: AccountId) {
        let predecessor_id = env::predecessor_account_id();

        if predecessor_id != self.owner {
            panic!("Unauthorized");
        }

        self.membership_contracts.remove(&contract_id);
    }

    pub fn generate_points(&mut self, account_id: AccountId, points: Points) -> Points {
        let predecessor_id = env::predecessor_account_id();

        if !self.membership_contracts.contains(&predecessor_id) {
            panic!("Unauthorized");
        }

        let mut user = self.users.get(&account_id).unwrap();

        user.points += points;

        self.users.insert(&account_id, &user);

        points
    }

    // View Functions
    pub fn get_user(&self, account_id: AccountId) -> User {
        self.users.get(&account_id).expect("User does not exist")
    }
}

fn refund_deposit(storage_used: u64, extra_spend: Balance) {
    let required_cost = env::storage_byte_cost() * Balance::from(storage_used);
    let attached_deposit = env::attached_deposit() - extra_spend;

    assert!(
        required_cost <= attached_deposit,
        "Must attach {} yoctoNEAR to cover storage",
        required_cost,
    );

    let refund = attached_deposit - required_cost;
    if refund > 1 {
        Promise::new(env::predecessor_account_id()).transfer(refund);
    }
}

fn get_random_number(shift_amount: u32) -> u32 {
    let mut seed = env::random_seed();
    let seed_len = seed.len();
    let mut arr: [u8; 4] = Default::default();
    seed.rotate_left(shift_amount as usize % seed_len);
    arr.copy_from_slice(&seed[..4]);
    u32::from_le_bytes(arr)
}

fn milli_to_seconds(ms: u64) -> u64 {
    ms / 1000
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(test)]
mod tests {
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::testing_env;

    use super::*;

    // Allows for modifying the environment of the mocked blockchain
    fn get_context(predecessor_account_id: AccountId) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder
            .current_account_id(accounts(0))
            .signer_account_id(predecessor_account_id.clone())
            .predecessor_account_id(predecessor_account_id);
        builder
    }

    #[test]
    fn set_get_message() {
        let mut context = get_context(accounts(1));
        // Initialize the mocked blockchain
        testing_env!(context.build());

        // Set the testing environment for the subsequent calls
        testing_env!(context.predecessor_account_id(accounts(1)).build());

        let mut contract = StatusMessage::default();
        contract.set_status("hello".to_string());
        assert_eq!(
            "hello".to_string(),
            contract.get_status(accounts(1)).unwrap()
        );
    }

    #[test]
    fn get_nonexistent_message() {
        let contract = StatusMessage::default();
        assert_eq!(None, contract.get_status("francis.near".parse().unwrap()));
    }
}
