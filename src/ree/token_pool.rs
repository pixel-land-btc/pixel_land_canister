use super::ExchangeError;
use candid::{CandidType, Deserialize};
use ic_stable_structures::{Storable, storable::Bound};
use ree_types::{CoinId, InputCoin, OutputCoin, Pubkey, Txid, Utxo};
use serde::Serialize;

pub const MIN_BTC_VALUE: u64 = 10000;

#[derive(Clone, CandidType, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TokenMeta {
    pub id: CoinId,
    pub symbol: String,
    pub exchange_rate: u64,
    pub min_amount: u128,
}

impl TokenMeta {
    pub fn btc() -> Self {
        Self {
            id: CoinId::btc(),
            symbol: "BTC".to_string(),
            exchange_rate: 1,
            min_amount: 546,
        }
    }
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
pub struct CanvasToken {
    pub states: Vec<TokenState>,
    pub meta: TokenMeta,
    pub pubkey: Pubkey,
    pub tweaked: Pubkey,
    pub addr: String,
}

impl CanvasToken {
    pub fn attrs(&self) -> String {
        format!("exchange_rate:{}", self.meta.exchange_rate)
    }
}

#[derive(CandidType, Clone, Debug, Deserialize, Eq, PartialEq, Serialize, Default)]
pub struct TokenState {
    pub id: Option<Txid>, 
    pub nonce: u64,       
    pub btc_balance: u64,
    pub exchange_rate: Option<u64>, // 此次交易时使用的汇率（价格）
    pub timestamp: u64,             // 交易时间戳
}

impl Storable for TokenState {
    const BOUND: Bound = Bound::Unbounded;

    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        let mut bytes = vec![];
        let _ = ciborium::ser::into_writer(self, &mut bytes);
        std::borrow::Cow::Owned(bytes)
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        let dire = ciborium::de::from_reader(bytes.as_ref()).expect("failed to decode TokenState");
        dire
    }
}

impl Storable for CanvasToken {
    const BOUND: Bound = Bound::Unbounded;

    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        let mut bytes = vec![];
        let _ = ciborium::ser::into_writer(self, &mut bytes);
        std::borrow::Cow::Owned(bytes)
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        let dire = ciborium::de::from_reader(bytes.as_ref()).expect("failed to decode CanvasToken");
        dire
    }
}

impl CanvasToken {
    pub fn token_id(&self) -> CoinId {
        self.meta.id
    }

    // Assigns a unique derivation path to each token based on its token ID
    // This ensures different tokens have different addresses and use different private keys
    pub fn derivation_path(&self) -> Vec<Vec<u8>> {
        vec![self.token_id().to_string().as_bytes().to_vec()]
    }

    // Calculate how many tokens can be bought with the given BTC amount using current rate
    pub fn calculate_buy_amount(&self, btc_amount: u64) -> u128 {
        let rate = self.get_current_exchange_rate();
        (btc_amount as u128) * (rate as u128)
    }

    // Calculate how much BTC can be obtained by selling the given token amount using current rate
    pub fn calculate_sell_amount(&self, token_amount: u128) -> u64 {
        let rate = self.get_current_exchange_rate();
        (token_amount / (rate as u128)) as u64
    }

    // Calculate buy amount with specific exchange rate
    pub fn calculate_buy_amount_with_rate(&self, btc_amount: u64, exchange_rate: u64) -> u128 {
        (btc_amount as u128) * (exchange_rate as u128)
    }

    // Calculate sell amount with specific exchange rate
    pub fn calculate_sell_amount_with_rate(&self, token_amount: u128, exchange_rate: u64) -> u64 {
        (token_amount / (exchange_rate as u128)) as u64
    }

    // Get current exchange rate (from latest state or fallback to meta)
    pub fn get_current_exchange_rate(&self) -> u64 {
        self.states
            .last()
            .and_then(|state| state.exchange_rate)
            .unwrap_or(self.meta.exchange_rate)
    }

    // Validates a buy token transaction (BTC -> Token mint)
    // If valid, generates the new token state that would result from executing the transaction
    // Returns the new state and token amount to mint
    pub(crate) fn validate_buy_token(
        &self,
        txid: Txid,
        nonce: u64,
        _token_utxo_spent: Vec<String>,
        _token_utxo_received: Vec<Utxo>,
        input_coins: Vec<InputCoin>,
        output_coins: Vec<OutputCoin>,
        exchange_rate: u64,  // 新增：交易时使用的汇率
    ) -> Result<(TokenState, u128), ExchangeError> {
        // Verify transaction structure (1 input coin BTC, 1 output coin Token)
        (input_coins.len() == 1 && output_coins.len() == 1)
            .then(|| ())
            .ok_or(ExchangeError::InvalidSignPsbtArgs(
                "invalid input/output_coins, buy_token requires 1 BTC input and 1 Token output".to_string(),
            ))?;

        let btc_input = &input_coins[0].coin;
        let token_output = &output_coins[0].coin;

        // Verify input coin is BTC
        (btc_input.id == CoinId::btc())
            .then(|| ())
            .ok_or(ExchangeError::InvalidSignPsbtArgs(
                "invalid input_coin, buy_token requires BTC".to_string(),
            ))?;

        // Verify output coin is the correct token
        (token_output.id == self.token_id())
            .then(|| ())
            .ok_or(ExchangeError::InvalidSignPsbtArgs(
                "invalid output_coin, wrong token type".to_string(),
            ))?;

        // Get the current token state or use default if empty
        let mut state = self.states.last().cloned().unwrap_or_default();

        // Verify nonce matches to prevent replay attacks
        (state.nonce == nonce)
            .then(|| ())
            .ok_or(ExchangeError::TokenStateExpired(state.nonce))?;

        // Verify minimum BTC amount
        let btc_amount: u64 = btc_input.value.try_into().map_err(|_| ExchangeError::Overflow)?;
        (btc_amount >= MIN_BTC_VALUE)
            .then(|| ())
            .ok_or(ExchangeError::TooSmallFunds)?;

        // Calculate expected token amount using provided exchange rate
        let expected_token_amount = self.calculate_buy_amount_with_rate(btc_amount, exchange_rate);
        
        // Verify the output token amount matches calculation
        (token_output.value == expected_token_amount)
            .then(|| ())
            .ok_or(ExchangeError::InvalidSignPsbtArgs(
                "token output amount mismatch with exchange rate".to_string(),
            ))?;

        // Update BTC balance (add received BTC)
        let new_btc_balance = state.btc_balance
            .checked_add(btc_amount)
            .ok_or(ExchangeError::Overflow)?;

        // Update the state
        state.btc_balance = new_btc_balance;
        state.nonce += 1;
        state.id = Some(txid);
        state.exchange_rate = Some(exchange_rate);
        state.timestamp = ic_cdk::api::time();

        Ok((state, expected_token_amount))
    }

    // Validates a sell token transaction (Token burn -> BTC)
    // If valid, generates the new token state that would result from executing the transaction
    // Returns the new state and BTC amount to pay
    pub(crate) fn validate_sell_token(
        &self,
        txid: Txid,
        nonce: u64,
        _token_utxo_spent: Vec<String>,
        _token_utxo_received: Vec<Utxo>,
        input_coins: Vec<InputCoin>,
        output_coins: Vec<OutputCoin>,
        exchange_rate: u64,  // 新增：交易时使用的汇率
    ) -> Result<(TokenState, u64), ExchangeError> {
        // Verify transaction structure (1 input coin Token, 1 output coin BTC)
        (input_coins.len() == 1 && output_coins.len() == 1)
            .then(|| ())
            .ok_or(ExchangeError::InvalidSignPsbtArgs(
                "invalid input/output_coins, sell_token requires 1 Token input and 1 BTC output".to_string(),
            ))?;

        let token_input = &input_coins[0].coin;
        let btc_output = &output_coins[0].coin;

        // Verify input coin is the correct token
        (token_input.id == self.token_id())
            .then(|| ())
            .ok_or(ExchangeError::InvalidSignPsbtArgs(
                "invalid input_coin, wrong token type".to_string(),
            ))?;

        // Verify output coin is BTC
        (btc_output.id == CoinId::btc())
            .then(|| ())
            .ok_or(ExchangeError::InvalidSignPsbtArgs(
                "invalid output_coin, sell_token requires BTC output".to_string(),
            ))?;

        // Get the current token state
        let mut state = self.states.last().cloned().ok_or(ExchangeError::EmptyToken)?;

        // Verify nonce matches to prevent replay attacks
        (state.nonce == nonce)
            .then(|| ())
            .ok_or(ExchangeError::TokenStateExpired(state.nonce))?;

        // Calculate expected BTC amount using provided exchange rate
        let token_amount = token_input.value;
        let expected_btc_amount = self.calculate_sell_amount_with_rate(token_amount, exchange_rate);

        // Verify minimum BTC amount
        (expected_btc_amount >= MIN_BTC_VALUE)
            .then(|| ())
            .ok_or(ExchangeError::TooSmallFunds)?;

        // Verify the output BTC amount matches calculation
        let btc_amount: u64 = btc_output.value.try_into().map_err(|_| ExchangeError::Overflow)?;
        (btc_amount == expected_btc_amount)
            .then(|| ())
            .ok_or(ExchangeError::InvalidSignPsbtArgs(
                "BTC output amount mismatch with exchange rate".to_string(),
            ))?;

        // Verify sufficient BTC balance for payment
        (state.btc_balance >= expected_btc_amount)
            .then(|| ())
            .ok_or(ExchangeError::InsufficientBtc)?;

        // Update BTC balance (subtract paid BTC)
        let new_btc_balance = state.btc_balance
            .checked_sub(expected_btc_amount)
            .ok_or(ExchangeError::Overflow)?;

        // Update the state
        state.btc_balance = new_btc_balance;
        state.nonce += 1;
        state.id = Some(txid);
        state.exchange_rate = Some(exchange_rate);
        state.timestamp = ic_cdk::api::time();

        Ok((state, expected_btc_amount))
    }

    // Rollback the token state to before the specified transaction
    // Removes the state created by txid and all subsequent states
    pub(crate) fn rollback(&mut self, txid: Txid) -> Result<(), ExchangeError> {
        let idx = self
            .states
            .iter()
            .position(|state| state.id == Some(txid))
            .ok_or(ExchangeError::InvalidState("txid not found".to_string()))?;
        if idx == 0 {
            self.states.clear();
            return Ok(());
        }
        self.states.truncate(idx);
        Ok(())
    }

    // Finalize a transaction by making its state the new base state
    // Removes all states before the specified transaction
    pub(crate) fn finalize(&mut self, txid: Txid) -> Result<(), ExchangeError> {
        let idx = self
            .states
            .iter()
            .position(|state| state.id == Some(txid))
            .ok_or(ExchangeError::InvalidState("txid not found".to_string()))?;
        if idx == 0 {
            return Ok(());
        }
        self.states.rotate_left(idx);
        self.states.truncate(self.states.len() - idx);
        Ok(())
    }

    // Adds a new TokenState to the chain after a transaction is executed
    pub(crate) fn commit(&mut self, state: TokenState) {
        self.states.push(state);
    }
}