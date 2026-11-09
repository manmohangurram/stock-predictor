-- Three monthly purses, not one with an earmark: stocks and mutual funds in the long-term pot keep
-- separate cash, each with its own allowance and its own expiry. NULL means the pot's main purse.
ALTER TABLE cash_ledger ADD COLUMN bucket TEXT;
CREATE INDEX IF NOT EXISTS idx_cash_ledger_bucket ON cash_ledger (mode, bucket);
