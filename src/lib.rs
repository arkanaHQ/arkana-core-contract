use std::collections::HashSet;

use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{TreeMap, UnorderedMap};
use near_sdk::json_types::U64;
use near_sdk::{env, near_bindgen, AccountId, BorshStorageKey, PanicOnDefault};
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
    membership_contracts: HashSet<AccountId>,
    spinwheel_wr: u8,
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct Reward {
    title: String,
    price: Points,
    ended_at: Timestamp,
    total_tickets: u64,
    winner: Option<AccountId>,
    tickets: TreeMap<u64, AccountId>,
}

#[derive(Serialize)]
pub struct RewardOutput {
    title: String,
    price: U64,
    ended_at: U64,
    total_tickets: U64,
    winner: Option<AccountId>,
}

#[derive(BorshDeserialize, BorshSerialize, Serialize)]
pub struct User {
    points: u64,
    last_daily_claim: Timestamp,
    last_free_spinwheel: Timestamp,
}

#[derive(Serialize)]
pub struct UserOutput {
    points: U64,
    last_daily_claim: U64,
    last_free_spinwheel: U64,
}

#[derive(BorshSerialize, BorshStorageKey)]
enum StorageKey {
    Users,
    Rewards,
    Tickets { reward_id: RewardId },
}

#[near_bindgen]
impl ArkanaCoreContract {
    #[init]
    pub fn new(owner: AccountId, daily_claim_points: U64, spin_wheel_price: U64) -> Self {
        Self {
            owner,
            daily_claim_points: daily_claim_points.0,
            spin_wheel_price: spin_wheel_price.0,
            users: UnorderedMap::new(StorageKey::Users),
            rewards: UnorderedMap::new(StorageKey::Rewards),
            last_reward_id: 0,
            membership_contracts: HashSet::new(),
            spinwheel_wr: 0,
        }
    }

    #[payable]
    pub fn create_reward(&mut self, title: String, price: U64, ended_at: U64) -> RewardId {
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
                tickets: TreeMap::new(StorageKey::Tickets {
                    reward_id: (self.last_reward_id + 1),
                }),
            },
        );

        self.last_reward_id += 1;

        self.last_reward_id
    }

    #[payable]
    pub fn buy_ticket(&mut self, reward_id: U64, amount: U64) -> (U64, U64) {
        let predecessor_id = env::predecessor_account_id();

        let mut reward = self.rewards.get(&reward_id.0).unwrap();

        let current_timestamp = env::block_timestamp_ms();

        assert!(current_timestamp < reward.ended_at, "Reward has ended");

        let mut user = self.users.get(&predecessor_id).unwrap();

        if user.points < reward.price * amount.0 {
            panic!("Points insufficient");
        }

        user.points -= reward.price * amount.0;

        reward
            .tickets
            .insert(&reward.total_tickets, &predecessor_id);
        reward.total_tickets += amount.0;

        self.users.insert(&predecessor_id, &user);
        self.rewards.insert(&reward_id.0, &reward);

        (reward_id, amount)
    }

    pub fn finalize_reward(&mut self, reward_id: U64) -> AccountId {
        let mut reward = self.rewards.get(&reward_id.0).unwrap();

        let current_timestamp = env::block_timestamp_ms();

        assert!(reward.winner.is_none(), "Reward finalized");

        if reward.ended_at > current_timestamp {
            panic!("Reward has not ended");
        }

        let random_number = get_random_number(0) as u64 % reward.total_tickets;

        let key_winner = reward.tickets.floor_key(&random_number).unwrap();
        let winner = reward.tickets.get(&key_winner).unwrap();

        reward.winner = Some(winner.clone());
        reward.tickets.clear();

        return winner;
    }

    #[payable]
    pub fn register_account(&mut self) {
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
            },
        );
    }

    pub fn daily_claim_point(&mut self) -> Points {
        let account_id = env::predecessor_account_id();

        let mut user = self.users.get(&account_id).expect("User does not exist");

        let current_timestamp = env::block_timestamp_ms();
        let delta_ms = current_timestamp - user.last_daily_claim;

        if delta_ms < ONE_DAY {
            panic!(
                "Cannot claim, please wait {} seconds",
                milli_to_seconds(ONE_DAY - delta_ms)
            );
        }

        user.points += self.daily_claim_points;
        user.last_daily_claim = current_timestamp;

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
                    milli_to_seconds(ONE_DAY - delta_ms)
                );
            }
            user.last_free_spinwheel = current_timestamp;
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
            20u16 + (self.spinwheel_wr as u16 * 3) / 10,
            10u16 + (self.spinwheel_wr as u16 * 2) / 10,
            2u16 + (self.spinwheel_wr as u16 * 1) / 10,
        ];

        let mut cumulative_weights: [u16; 6] = [0; 6];

        cumulative_weights[0] = weights[0];
        for i in 1..weights.len() {
            cumulative_weights[i] = weights[i] + cumulative_weights[i - 1];
        }

        let total_weights: u16 = cumulative_weights[5]; // last index
        let random_number = get_random_number(0) as u16 % total_weights;
        let mut result = 0;

        for i in 0..weights.len() {
            if cumulative_weights[i] >= random_number {
                result = points[i];
                break;
            }
        }

        if result > 5 {
            self.spinwheel_wr = 0;
        } else {
            self.spinwheel_wr += 1;
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

    pub fn generate_points(&mut self, account_id: AccountId, points: U64) -> U64 {
        let predecessor_id = env::predecessor_account_id();

        if !self.membership_contracts.contains(&predecessor_id) {
            panic!("Unauthorized");
        }

        let mut user = self.users.get(&account_id).unwrap();

        user.points += points.0;

        self.users.insert(&account_id, &user);

        U64(user.points)
    }

    // View Functions
    pub fn get_user(&self, account_id: AccountId) -> UserOutput {
        let user = self.users.get(&account_id).expect("User does not exist");
        UserOutput {
            points: U64(user.points),
            last_daily_claim: U64(user.last_daily_claim),
            last_free_spinwheel: U64(user.last_free_spinwheel),
        }
    }

    pub fn get_reward(&self, reward_id: U64) -> RewardOutput {
        let reward = self.rewards.get(&reward_id.0).unwrap();

        RewardOutput {
            title: reward.title,
            price: U64(reward.price),
            ended_at: U64(reward.ended_at),
            total_tickets: U64(reward.total_tickets),
            winner: reward.winner,
        }
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
}
