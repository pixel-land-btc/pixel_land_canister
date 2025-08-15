pub mod exchange;
pub mod token;
pub mod token_pool;

// 公开导出主要类型和接口
pub use self::token::{BuyTokenOffer, SellTokenOffer, CanvasTokenInfo};
pub use self::token_pool::{CanvasToken, TokenMeta, TokenState};

use candid::CandidType;
use ic_stable_structures::{
    DefaultMemoryImpl, StableBTreeMap,
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
};
use ree_types::{
    TxRecord, Txid,
    exchange_interfaces::{
        NewBlockInfo,
    },
};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashSet;
use thiserror::Error;

pub const SCHNORR_KEY_NAME: &str = "key_1";

#[derive(Debug, Error, CandidType, Clone)]
pub enum ExchangeError {
    #[error("overflow")]
    Overflow,
    #[error("invalid token")]
    InvalidToken,
    #[error("too small funds")]
    TooSmallFunds,
    #[error("invalid txid")]
    InvalidTxid,
    #[error("the token has not been initialized or has been removed")]
    EmptyToken,
    #[error("invalid token state: {0}")]
    InvalidState(String),
    #[error("invalid sign_psbt args: {0}")]
    InvalidSignPsbtArgs(String),
    #[error("token state expired, current = {0}")]
    TokenStateExpired(u64),
    #[error("insufficient btc balance for sell")]
    InsufficientBtc,
}

#[derive(Eq, PartialEq, CandidType, Clone, Debug, Deserialize, Serialize)]
pub struct TxRecordInfo {
    pub txid: String,
    pub confirmed: bool,
    pub records: Vec<String>,
}

#[derive(Eq, PartialEq, CandidType, Clone, Debug, Deserialize, Serialize)]
pub struct BlockInfo {
    pub height: u32,
    pub hash: String,
}

type Memory = VirtualMemory<DefaultMemoryImpl>;

thread_local! {
  static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
      RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

  // CANVAS_TOKENS stores all canvas token configurations
  // It's a mapping from token_address (String) to CanvasToken information
  pub static CANVAS_TOKENS: RefCell<StableBTreeMap<String, token_pool::CanvasToken, Memory>> = RefCell::new(
      StableBTreeMap::init(
          MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0))),
      )
  );

  // BLOCKS stores the canonical blockchain observed by the exchange
  // It's used for finalizing transactions
  // Key: Block height (u32)
  pub static BLOCKS: RefCell<StableBTreeMap<u32, NewBlockInfo, Memory>> = RefCell::new(
      StableBTreeMap::init(
          MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1))),
      )
  );

  // TX_RECORDS tracks which token states are affected by each transaction
  // Key: (Txid, bool) where bool=true for confirmed transactions, bool=false for unconfirmed transactions
  pub static TX_RECORDS: RefCell<StableBTreeMap<(Txid, bool), TxRecord, Memory>> = RefCell::new(
      StableBTreeMap::init(
          MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2))),
      )
  );

  pub static EXECUTING_TOKENS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

// 公开的辅助函数
pub fn get_canvas_tokens() -> Vec<token_pool::CanvasToken> {
    CANVAS_TOKENS.with_borrow(|p| p.iter().map(|p| p.1.clone()).collect::<Vec<_>>())
}

pub fn get_canvas_token(addr: &String) -> Option<token_pool::CanvasToken> {
    CANVAS_TOKENS.with_borrow(|p| p.get(addr))
}

// 公开的守护结构
#[must_use]
pub struct ExecuteTxGuard(String);

impl ExecuteTxGuard {
    pub fn new(token_address: String) -> Option<Self> {
        EXECUTING_TOKENS.with(|executing_tokens| {
            if executing_tokens.borrow().contains(&token_address) {
                return None;
            }
            executing_tokens.borrow_mut().insert(token_address.clone());
            return Some(ExecuteTxGuard(token_address));
        })
    }
}

impl Drop for ExecuteTxGuard {
    fn drop(&mut self) {
        EXECUTING_TOKENS.with_borrow_mut(|executing_tokens| {
            executing_tokens.remove(&self.0);
        });
    }
}