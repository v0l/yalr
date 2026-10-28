UPDATE user_balances
SET lifetime_deposited_msat = COALESCE(
    (SELECT SUM(amount_msat) FROM balance_transactions t
     WHERE t.user_id = user_balances.user_id
       AND t.amount_msat > 0
       AND t.transaction_type NOT IN ('refund', 'refund_reversal')),
    0);
