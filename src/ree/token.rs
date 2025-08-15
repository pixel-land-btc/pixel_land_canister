use super::{ExchangeError, token_pool::TokenMeta};
use candid::{CandidType, Deserialize};
use ic_cdk_macros::{query, update};
use ree_types::{CoinId, bitcoin::Network, schnorr::request_ree_pool_address};
use serde::Serialize;

#[derive(Eq, PartialEq, CandidType, Clone, Debug, Deserialize, Serialize)]
pub struct BuyTokenOffer {
    pub nonce: u64,
    pub token_amount: u128,        
    pub current_btc_balance: u64,  
}

#[derive(Eq, PartialEq, CandidType, Clone, Debug, Deserialize, Serialize)]
pub struct SellTokenOffer {
    pub nonce: u64,
    pub btc_amount: u64,           
    pub current_btc_balance: u64, 
}

#[derive(Eq, PartialEq, CandidType, Clone, Debug, Deserialize, Serialize)]
pub struct CanvasTokenInfo {
    pub address: String,
    pub symbol: String,
    pub exchange_rate: u64,
    pub token_id: CoinId,
}

// 
#[query]
pub fn pre_buy_token(
    token_address: String,
    btc_amount: u64,
) -> Result<BuyTokenOffer, ExchangeError> {
    if btc_amount < super::token_pool::MIN_BTC_VALUE {
        return Err(ExchangeError::TooSmallFunds);
    }
    
    let token = super::get_canvas_token(&token_address).ok_or(ExchangeError::InvalidToken)?;
    let state = token.states.last().cloned().unwrap_or_default();
    
    let token_amount = token.calculate_buy_amount(btc_amount);
    
    Ok(BuyTokenOffer {
        nonce: state.nonce,
        token_amount,
        current_btc_balance: state.btc_balance,
    })
}

#[query]
pub fn pre_sell_token(
    token_address: String, 
    token_amount: u128
) -> Result<SellTokenOffer, ExchangeError> {
    let token = super::get_canvas_token(&token_address).ok_or(ExchangeError::InvalidToken)?;
    let state = token.states.last().ok_or(ExchangeError::EmptyToken)?;
    
    let btc_amount = token.calculate_sell_amount(token_amount);
    
    if btc_amount < super::token_pool::MIN_BTC_VALUE {
        return Err(ExchangeError::TooSmallFunds);
    }
    
    if state.btc_balance < btc_amount {
        return Err(ExchangeError::InsufficientBtc);
    }
    
    Ok(SellTokenOffer {
        nonce: state.nonce,
        btc_amount,
        current_btc_balance: state.btc_balance,
    })
}

#[update]
// init_canvas_token creates a new canvas token with fixed exchange rate
// This allows users to mint tokens by sending BTC and burn tokens to get BTC back
pub async fn init_canvas_token(
    block: u64,
    tx: u64,
    symbol: String,
    exchange_rate: u64,
) -> Result<CanvasTokenInfo, String> {
    let caller = ic_cdk::api::caller();
    if !ic_cdk::api::is_controller(&caller) {
        return Err("Not authorized".to_string());
    }

    if exchange_rate == 0 {
        return Err("Exchange rate must be greater than 0".to_string());
    }

    let id = CoinId::rune(block, tx as u32);
    let meta = TokenMeta {
        id,
        symbol: symbol.clone(),
        exchange_rate,
        min_amount: 1,
    };

    let (untweaked, tweaked, addr) = request_ree_pool_address(
        super::SCHNORR_KEY_NAME,
        vec![id.to_string().as_bytes().to_vec()],
        Network::Testnet4,
    )
    .await?;

    let canvas_token = super::token_pool::CanvasToken {
        meta: meta.clone(),
        pubkey: untweaked.clone(),
        tweaked,
        addr: addr.to_string(),
        states: vec![],
    };
    
    super::CANVAS_TOKENS.with_borrow_mut(|p| {
        p.insert(addr.to_string(), canvas_token);
    });
    
    Ok(CanvasTokenInfo {
        address: addr.to_string(),
        symbol,
        exchange_rate,
        token_id: id,
    })
}

#[update]
pub async fn reset_blocks() -> Result<(), String> {
    let caller = ic_cdk::api::caller();
    if !ic_cdk::api::is_controller(&caller) {
        return Err("Not authorized".to_string());
    }
    super::BLOCKS.with_borrow_mut(|b| {
        b.clear_new();
    });
    Ok(())
}

#[update]
pub async fn reset_tx_records() -> Result<(), String> {
    let caller = ic_cdk::api::caller();
    if !ic_cdk::api::is_controller(&caller) {
        return Err("Not authorized".to_string());
    }
    super::TX_RECORDS.with_borrow_mut(|t| {
        t.clear_new();
    });
    Ok(())
}

#[query]
pub fn query_tx_records() -> Result<Vec<super::TxRecordInfo>, String> {
    let res = super::TX_RECORDS.with_borrow(|t| {
        t.iter()
            .map(|((txid, confirmed), records)| super::TxRecordInfo {
                txid: txid.to_string(),
                confirmed,
                records: records.pools.clone(),
            })
            .collect()
    });

    Ok(res)
}

#[query]
pub fn query_blocks() -> Result<Vec<super::BlockInfo>, String> {
    let res = super::BLOCKS.with_borrow(|b| {
        b.iter()
            .map(|(_, block)| super::BlockInfo {
                height: block.block_height,
                hash: block.block_hash.clone(),
            })
            .collect()
    });

    Ok(res)
}

#[query]
pub fn blocks_tx_records_count() -> Result<(u64, u64), String> {
    let tx_records_count = super::TX_RECORDS.with_borrow(|t| t.len());
    let blocks_count = super::BLOCKS.with_borrow(|b| b.len());
    Ok((blocks_count, tx_records_count))
}