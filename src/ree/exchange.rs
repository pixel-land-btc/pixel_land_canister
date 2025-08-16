use super::ExecuteTxGuard;
use super::token_pool;
use ic_cdk_macros::{query, update};
use ree_types::orchestrator_interfaces::ensure_testnet4_orchestrator;
use ree_types::{
    Intention, bitcoin::psbt::Psbt, exchange_interfaces::*,
};

#[query]
pub fn get_pool_list() -> GetPoolListResponse {
    let tokens = super::get_canvas_tokens();
    tokens
        .iter()
        .map(|t| PoolBasic {
            name: t.meta.symbol.clone(),
            address: t.addr.clone(),
        })
        .collect()
}

#[query]
pub fn get_pool_info(args: GetPoolInfoArgs) -> GetPoolInfoResponse {
    let GetPoolInfoArgs { pool_address } = args;
    let t = super::get_canvas_token(&pool_address)?;

    Some(PoolInfo {
        key: t.pubkey.clone(),
        name: t.meta.symbol.clone(),
        key_derivation_path: vec![t.meta.id.to_bytes()],
        address: t.addr.clone(),
        nonce: t.states.last().map(|s| s.nonce).unwrap_or_default(),
        btc_reserved: t.states.last().map(|s| s.btc_balance).unwrap_or_default(),
        coin_reserved: vec![],
        utxos: vec![], 
        attributes: t.attrs(),
    })
}

#[query]
fn get_minimal_tx_value(_args: GetMinimalTxValueArgs) -> GetMinimalTxValueResponse {
    token_pool::MIN_BTC_VALUE
}

#[update(guard = "ensure_testnet4_orchestrator")]
pub fn rollback_tx(args: RollbackTxArgs) -> RollbackTxResponse {
    let result = super::TX_RECORDS.with_borrow_mut(|m| {
        let maybe_unconfirmed_record = m.get(&(args.txid.clone(), false));
        let maybe_confirmed_record = m.get(&(args.txid.clone(), true));
        let record = maybe_confirmed_record
            .or(maybe_unconfirmed_record)
            .ok_or(format!("No record found for txid: {}", args.txid))?;

        ic_cdk::println!(
            "rollback txid: {} with tokens: {:?}",
            args.txid,
            record.pools
        );

        // Roll back each affected token to its state before this transaction
        record.pools.iter().for_each(|token_address| {
            super::CANVAS_TOKENS.with_borrow_mut(|tokens| {
                if let Some(mut token) = tokens.get(token_address) {
                    if let Err(e) = token.rollback(args.txid) {
                        ic_cdk::println!("Rollback failed: {:?}", e);
                    } else {
                        tokens.insert(token_address.clone(), token);
                    }
                } else {
                    ic_cdk::println!("Token not found: {}", token_address);
                }
            });
        });

        m.remove(&(args.txid.clone(), false));
        m.remove(&(args.txid.clone(), true));

        Ok(())
    });

    result
}

#[update(guard = "ensure_testnet4_orchestrator")]

pub fn new_block(args: NewBlockArgs) -> NewBlockResponse {
    let NewBlockArgs {
        block_height,
        block_hash: _,
        block_timestamp: _,
        confirmed_txids,
    } = args.clone();

    super::BLOCKS.with_borrow_mut(|m| {
        m.insert(block_height, args);
        ic_cdk::println!("new block {} inserted into blocks", block_height,);
    });

    for txid in confirmed_txids {
        super::TX_RECORDS.with_borrow_mut(|m| {
            if let Some(record) = m.remove(&(txid.clone(), false)) {
                m.insert((txid.clone(), true), record.clone());
                ic_cdk::println!("confirm txid: {} with tokens: {:?}", txid, record.pools);
            }
        });
    }

    // Calculate the height below which blocks are considered fully confirmed (beyond reorg risk)
    let confirmed_height = if block_height >= 6 { block_height - 6 } else { 0 };

    // Finalize transactions in confirmed blocks
    super::BLOCKS.with_borrow(|m| {
        m.iter()
            .take_while(|(height, _)| *height <= confirmed_height)
            .for_each(|(height, block_info)| {
                ic_cdk::println!("finalizing txs in block: {}", height);
                block_info.confirmed_txids.iter().for_each(|txid| {
                    super::TX_RECORDS.with_borrow_mut(|m| {
                        if let Some(record) = m.get(&(txid.clone(), true)) {
                            ic_cdk::println!(
                                "finalize txid: {} with tokens: {:?}",
                                txid,
                                record.pools
                            );
                            // Make transaction state permanent in each affected token
                            record.pools.iter().for_each(|token_address| {
                                super::CANVAS_TOKENS.with_borrow_mut(|t| {
                                    if let Some(mut token) = t.get(token_address) {
                                        if let Err(e) = token.finalize(txid.clone()) {
                                            ic_cdk::println!("Finalize failed: {:?}", e);
                                        } else {
                                            t.insert(token_address.clone(), token);
                                        }
                                    } else {
                                        ic_cdk::println!("Token not found: {}", token_address);
                                    }
                                });
                            });
                            m.remove(&(txid.clone(), true));
                        }
                    });
                });
            });
    });

    // Clean up old block data that's no longer needed
    super::BLOCKS.with_borrow_mut(|m| {
        let heights_to_remove: Vec<u32> = m
            .iter()
            .take_while(|(height, _)| *height <= confirmed_height)
            .map(|(height, _)| height)
            .collect();
        for height in heights_to_remove {
            ic_cdk::println!("removing block: {}", height);
            m.remove(&height);
        }
    });
    Ok(())
}

#[update(guard = "ensure_testnet4_orchestrator")]
// Accepts transaction execution requests from the orchestrator
// Verifies the submitted PSBT (Partially Signed Bitcoin Transaction)
// If validation passes, signs the token's UTXOs and updates the exchange token state
// Only the orchestrator can call this function (ensured by the guard)
pub async fn execute_tx(args: ExecuteTxArgs) -> ExecuteTxResponse {
    let ExecuteTxArgs {
        psbt_hex,
        txid,
        intention_set,
        intention_index,
        zero_confirmed_tx_queue_length: _zero_confirmed_tx_queue_length,
    } = args;

    // Decode and deserialize the PSBT
    let raw = hex::decode(&psbt_hex).map_err(|_| "invalid psbt".to_string())?;
    let mut psbt = Psbt::deserialize(raw.as_slice()).map_err(|_| "invalid psbt".to_string())?;

    // Extract the intention details
    let intention = intention_set.intentions[intention_index as usize].clone();
    let Intention {
        exchange_id: _,
        action: _,
        action_params,
        pool_address,
        nonce,
        pool_utxo_spent,
        pool_utxo_received,
        input_coins,
        output_coins,
    } = intention;

    // Extract exchange rate from action_params
    // For now, use a default rate - this should be passed via action_params in the future
    // TODO: Parse exchange_rate from action_params based on actual type
    let exchange_rate: u64 = 100; // Default rate

    let _guard = ExecuteTxGuard::new(pool_address.clone())
        .ok_or(format!("Token {0} Executing", pool_address).to_string())?;

    // Get the canvas token from storage
    let canvas_token = super::CANVAS_TOKENS
        .with_borrow(|m| m.get(&pool_address).expect("already checked in pre_*; qed"));

    // Process the transaction based on the action type
    match intention.action.as_ref() {
        "buy_token" => {
            // Validate the buy token transaction and get the new token state
            let (new_state, _token_amount) = canvas_token
                .validate_buy_token(
                    txid,
                    nonce,
                    pool_utxo_spent,
                    pool_utxo_received,
                    input_coins,
                    output_coins,
                    exchange_rate,
                )
                .map_err(|e| e.to_string())?;

            // For buy_token, we don't need to sign anything since we're receiving BTC
            // The token minting is handled by the system

            // Update the canvas token with the new state
            super::CANVAS_TOKENS.with_borrow_mut(|m| {
                let mut token = m
                    .get(&pool_address)
                    .expect("already checked in pre_buy_token; qed");
                token.commit(new_state);
                m.insert(pool_address.clone(), token);
            });
        }
        "sell_token" => {
            // Validate the sell token transaction and get the new token state
            let (new_state, _btc_amount) = canvas_token
                .validate_sell_token(
                    txid,
                    nonce,
                    pool_utxo_spent,
                    pool_utxo_received,
                    input_coins,
                    output_coins,
                    exchange_rate,
                )
                .map_err(|e| e.to_string())?;

            // For sell_token, we need to sign the BTC output to pay the user
            // Note: This is a simplified example - in practice, you'd need to track UTXOs properly
            // Sign with the canvas token's key
            // ree_pool_sign(
            //     &mut psbt,
            //     vec![...], // UTXOs to sign
            //     crate::SCHNORR_KEY_NAME,
            //     canvas_token.derivation_path(),
            // )
            // .await
            // .map_err(|e| e.to_string())?;

            // Update the canvas token with the new state
            super::CANVAS_TOKENS.with_borrow_mut(|m| {
                let mut token = m
                    .get(&pool_address)
                    .expect("already checked in pre_sell_token; qed");
                token.commit(new_state);
                m.insert(pool_address.clone(), token);
            });
        }
        _ => {
            return Err("invalid method".to_string());
        }
    }

    super::TX_RECORDS.with_borrow_mut(|m| {
        ic_cdk::println!("new unconfirmed txid: {} in token: {} ", txid, pool_address);
        let mut record = m.get(&(txid.clone(), false)).unwrap_or_default();
        if !record.pools.contains(&pool_address) {
            record.pools.push(pool_address.clone());
        }
        m.insert((txid.clone(), false), record);
    });

    Ok(psbt.serialize_hex())
}