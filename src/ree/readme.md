
BuyTokenOffer {
    nonce: 1,              // 当前状态版本
    token_amount: 100000,  // 能买到的代币数量
    current_btc_balance: 50000 // 当前池子BTC余额
    }

  execute_tx(ExecuteTxArgs {
      psbt_hex: "...",   
      txid: "tx123",    // 计算生成的psbt 位置
      intention_set: [...], 
      intention_index: 0,   
  })?;