# Resurrect program (devnet v3 — any-asset pairing · holder rewards · proof of personhood)

Program id (devnet): `5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t`

Source: `src/lib.rs` (Anchor 0.31). Binary: `resurrect.so` (built with platform-tools v1.52, stripped).
Browser client: `../js/rsrct.js` (no Anchor TS dependency; instruction encoding by hand).

## Pairing asset
Every launch names a `pair_mint`: wrapped SOL, USDC, a tokenized stock (xStocks, Ondo, Sunrise…), an ETF, a commodity token.
Any **SPL Token or Token-2022** mint works — the program uses `transfer_checked` through the token interface and stores the
pair's token program on the launch. Bids, the pool, fees and holder rewards are all denominated in that asset.
SOL is handled as wrapped SOL; the client wraps and unwraps inside the same transaction so wallets only ever see plain SOL.
Minimum bid = 0.01 units of the pair asset.

## Lifecycle
1. `create_launch(name, symbol, batch_secs, creator_fee_bps)` — creator transfers the entire fixed supply (1,000,000,000 × 10⁶)
   into the program vault. Mint must have 6 decimals, supply exactly 1B, mint & freeze authority revoked. Creates escrow, pool
   and fee-vault token accounts for the pair asset, all owned by the launch PDA. Creator keeps nothing.
2. `commit_bid(commitment, deposit)` — escrow pair tokens ≥ bid, store `sha256(bidder ‖ amount_le ‖ salt)`. One bid PDA per wallet per launch.
3. `reveal_bid(amount, salt)` — after the batch closes (≤15 min), within a 10-minute window. Hash must match.
4. `settle` — anyone. Clearing price = ⌈total revealed / 800M tokens⌉ (Q32, pair base units per token base unit). All raised
   pair tokens + 20% of supply seed a constant-product pool owned by the program. There is no withdraw/close instruction for the pool.
5. `claim` — tokens at the clearing price + refund of (deposit − bid). Unrevealed bids: full refund, no tokens.
6. `swap_pair_for_tokens` / `swap_tokens_for_pair` — 1% fee on the pair side → fee vault.
7. `claim_creator_fees` — creator withdraws `creator_fee_bps` (≤20%) of accrued fees. Nothing lets the creator touch the pool.
8. `stake(amount)` / `unstake(amount)` / `claim_rewards` — holders deposit launch tokens into the stake vault (no lockup, no fee)
   and earn the holder share (100% − creator share) of every swap fee, paid in the pair asset. Accounting is a per-token reward
   index (Q64) updated on every swap; stakers pull what they are owed. Fees that accrue while nobody is staked are held and
   released into the index when the first stake arrives.

## Proof of personhood
`create_launch(..., require_human: bool)`. When set, `commit_bid` requires the bidder's **Civic Gateway token** on the
Uniqueness gatekeeper network (`uniqobk8oGh4XBLMqM68K8M2zNu3CdYX7q5go7whQiv`, one pass per human). The program checks that the
account is owned by the Gateway program (`gatem74V238djXdzWnJf94Wo1DcnuGkfijbf3AuBhfs`), belongs to the bidder, is on the
expected network, is Active and not expired. Combined with the one-bid-PDA-per-wallet rule: one human, one wallet, one bid.
Passes are issued by Civic (getpass.civic.com); the launchpad never sees identity data.

## PDAs (all seeded by the launched coin's mint)
`launch`, `vault`, `stake` (coin), `escrow`, `pool`, `fees` (pair asset) → `[seed, mint]`; `bid` → `["bid", launch, bidder]`; stake position → `["stake", launch, owner]`.

## Tested (local validator, `e2e4.js`)
- Launch paired with SOL with `require_human`: bid without a pass → rejected; bid with another wallet's pass → rejected;
  bid with own pass → accepted. Launch paired with a mock Token-2022 stock (8 dec): create → 3 sealed bids → duplicate bid
  rejected → reveal → settle → claims with refunds → buy/sell → fee accrual → creator fee claim → double claim rejected →
  stake → swap → rewards claim → unstake.

## Not yet
- Independent audit — **required before any mainnet deployment**
- Upgrade authority → multisig, then freeze
