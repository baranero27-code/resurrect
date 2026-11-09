# Resurrect program (v4 — any-asset pairing · on-chain fee split · holder, trader & buyback rewards · proof of personhood)

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
7. **Fee split (constants):** every swap fee is split in the swap instruction itself —
   `HOLDER_BPS` 3500 · `BUYBACK_BPS` 2500 · `TRADER_BPS` 2000 · creator 0–1000 (chosen at launch) · `TREASURY_BPS` 1000.
   The creator's unused share (10% − creator share) and rounding dust go to holders.
   - `claim_creator_fees` — creator pulls their share.
   - `collect_treasury` — anyone pushes the treasury share to the treasury wallet's token account (address in `Config`).
   - `buyback_and_burn` — anyone; swaps the launch's accrued buyback share through the **RSRCT launch's own pool** (paying its 1% fee like any trader) and burns the RSRCT received, in one instruction. Requires `Config.rsrct_mint` to be set and the RSRCT launch to be paired with the same asset.
   - Trader rewards: weekly epochs from `settle_ts`. Each swap adds its trader share to `EpochRecord[epoch].pool`; if the trader passes a valid Civic pass, the swap size is added to `TraderStats[epoch].volume` and the epoch's verified volume. After the epoch ends, `claim_trader_reward(epoch)` pays pool × volume ÷ epoch_volume. `sweep_epoch(epoch)` routes a pool with zero verified volume to holders.
   - `init_config(treasury)` (ADMIN only, once) / `set_config(treasury, rsrct_mint)` (config authority).
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
`launch`, `vault`, `stake` (coin), `escrow`, `pool`, `fees` (pair asset) → `[seed, mint]`; `bid` → `["bid", launch, bidder]`; stake position → `["stake", launch, owner]`; `config` → `["config"]`; `epoch` → `["epoch", launch, u32 le]`; `trader` → `["trader", launch, trader, u32 le]`.

## Tested (local validator, `e2e5.js` — v4; `e2e4.js` — v3 lifecycle; `exttest.js` — extension policy)
- v4: non-admin config init rejected; creator fee > 10% rejected; split sums exactly to the fee; verified vs unverified volume; wrong-epoch swap rejected; treasury collection; buyback through the RSRCT pool with supply decreasing by exactly the burned amount; trader claim before epoch end rejected, after end pays the whole pool to the only verified trader; unverified trader and double claims rejected; sweep of a pool with no verified volume to holders; holder stake/claim of the 40% share.
- Launch paired with SOL with `require_human`: bid without a pass → rejected; bid with another wallet's pass → rejected;
  bid with own pass → accepted. Launch paired with a mock Token-2022 stock (8 dec): create → 3 sealed bids → duplicate bid
  rejected → reveal → settle → claims with refunds → buy/sell → fee accrual → creator fee claim → double claim rejected →
  stake → swap → rewards claim → unstake.

## Not yet
- Independent audit — **required before any mainnet deployment**
- Upgrade authority → multisig, then freeze

## Upgrade authority → multisig → frozen (mainnet runbook)
```bash
# 1. after deploying to mainnet with the same program keypair:
solana program show 5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t -u m
# 2. hand the upgrade authority to a Squads vault (create the vault at app.squads.so first)
solana program set-upgrade-authority 5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t \
  --new-upgrade-authority <SQUADS_VAULT_PDA> --skip-new-upgrade-authority-signer-check -u m -k <current-authority.json>
# 3. after the audit, make the program immutable (irreversible):
solana program set-upgrade-authority 5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t --final -u m   # signed through Squads
```
Until step 3, the upgrade authority is the single trust assumption of the system and must be disclosed on the site.

## Builds
- Features: `mainnet` (batch fixed at 15 min), `localtest` (ADMIN = local payer, 90-second epochs — **never deploy**).
- `resurrect.so` — devnet build (batches 60–900 s), what is deployed on devnet.
- `resurrect-mainnet.so` — built with `--features mainnet`: batch length is fixed at 15 minutes. This is the binary to deploy on mainnet.
```
cargo +solana-152 build --release --target sbpf-solana-solana --manifest-path programs/resurrect/Cargo.toml -Zbuild-std=std,panic_abort [--features mainnet]
llvm-objcopy --strip-all target/sbpf-solana-solana/release/resurrect.so resurrect.so
```
