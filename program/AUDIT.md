# Resurrect — audit package

Prepared for external reviewers. Everything here is verifiable against `src/lib.rs` (Anchor 0.31, ~560 lines) and the tests in this folder.

## 1. What the program is
A launchpad for fixed-supply SPL tokens on Solana. Each launch runs a sealed-bid (commit–reveal) batch auction with a single clearing price, then seeds a constant-product pool that the program owns and that has **no withdrawal path**. Every launch is denominated in a creator-chosen "pair asset" (SPL Token or Token-2022). Every swap fee is split on chain: 35% staked holders · 25% RSRCT buyback & burn · 20% verified traders (weekly epochs) · 0–10% creator · 10% treasury; the creator's unused share goes to holders.

Program id (devnet): `5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t`. Upgradeable (BPF upgradeable loader). Mainnet plan: deploy, run 2 weeks, move upgrade authority to a Squads multisig, then set it to `None` once audited.

## 2. Accounts
| Account | Seeds | Owner/authority | Holds |
|---|---|---|---|
| `Launch` | `["launch", mint]` | program | all launch state (below) |
| `vault` | `["vault", mint]` | Launch PDA | launch tokens not yet claimed/sold (SPL Token) |
| `escrow` | `["escrow", mint]` | Launch PDA | pair tokens deposited by bidders until settle/claim |
| `pool` | `["pool", mint]` | Launch PDA | pair side of the AMM |
| `fees` | `["fees", mint]` | Launch PDA | accrued fees: creator share + unclaimed holder rewards |
| `stake` vault | `["stake", mint]` | Launch PDA | launch tokens staked by holders |
| `Bid` | `["bid", launch, bidder]` | program | commitment, deposit, revealed amount, flags |
| `StakeAccount` | `["stake", launch, owner]` | program | amount, reward index snapshot, owed, claimed |
| `Config` | `["config"]` | program | authority, treasury wallet, rsrct_mint |
| `EpochRecord` | `["epoch", launch, u32]` | program | trader pool, verified volume, claimed, swept |
| `TraderStats` | `["trader", launch, trader, u32]` | program | verified volume, raw volume, claimed, paid |

`Launch` fields: creator, mint, vault, pair_mint, pair_token_program, escrow, pool_pair_account, fee_vault, name, symbol, start/end/reveal_end timestamps, creator_fee_bps, pair_decimals, min_bid, bids, revealed, total_revealed, clearing_price_q32 (u128, pair base units per token base unit × 2³²), settled, pool_pair, pool_tokens (virtual reserves), fees_accrued, creator_fees_owed/claimed, require_human, gatekeeper_network, stake_vault, total_staked, reward_index (u128, Q64 per staked base unit), rewards_pending, rewards_distributed, bump.

## 3. Instructions and preconditions
| Ix | Who | Checks |
|---|---|---|
| `create_launch(name, symbol, batch_secs, creator_fee_bps, require_human)` | creator | mint: supply == 1e15, decimals 6, mint & freeze authority None; pair ≠ mint; pair extension policy (§5); batch 60–900 s; fee ≤ 2000 bps; transfers 100% of supply creator → vault |
| `commit_bid(commitment, deposit)` | bidder | now < end_ts; deposit ≥ min_bid; if require_human: valid Civic pass (§6); pair transfer bidder → escrow; **escrow delta must equal deposit** |
| `reveal_bid(amount, salt)` | bidder | end_ts ≤ now < reveal_end_ts; not revealed; min_bid ≤ amount ≤ deposit; sha256(bidder‖amount_le‖salt) == commitment |
| `settle()` | anyone | now ≥ reveal_end_ts; not settled. If total_revealed == 0: settled, pool stays empty. Else price = ⌈total_revealed·2³² / 5e14⌉; escrow → pool (total_revealed); pool_tokens = 5e14 (pool opens at the clearing price) |
| `claim()` | bidder | settled; not claimed. Refund = deposit − amount (revealed) or deposit (unrevealed). Tokens = amount·2³² / price (floor) |
| `swap_pair_for_tokens(in, min_out)` | anyone | settled & pool_tokens > 0; fee = in/100; out = cp(net); out ≥ min_out; **pool delta == net** |
| `swap_tokens_for_pair(in, min_out)` | anyone | gross = cp(in); fee = gross/100; out = gross − fee ≥ min_out |
| `claim_creator_fees()` | creator (has_one) | owed > 0 |
| `collect_treasury()` | anyone | treasury_owed > 0; destination ATA owner == Config.treasury |
| `buyback_and_burn()` | anyone | Config.rsrct_mint set & == rsrct_launch.mint; same pair; rsrct settled; buyback_accrued > 0; swap math identical to a trade; burn from rsrct vault with rsrct launch PDA |
| `claim_trader_reward(epoch)` | trader | epoch ended; !claimed; volume > 0; epoch volume > 0; share = pool·vol/epoch_vol |
| `sweep_epoch(epoch)` | anyone | epoch ended; epoch volume == 0; pool > 0; !swept → holders |
| `init_config` / `set_config` | ADMIN / authority | address / has_one |
| `stake(amount)` / `unstake(amount)` | holder | settled (stake); amount ≤ staked (unstake); rewards settled to `owed` before any change |
| `claim_rewards()` | staker | owed > 0 after settling index |

## 4. Invariants we claim (please try to break them)
1. **No liquidity withdrawal.** The only instructions that move tokens out of `pool` are the two swaps, and each one moves out exactly `cp_out(...)` (+fee) of the opposite side in the same transaction. There is no close, no admin, no migration.
2. **Creator never receives launch tokens from the program** except via `claim` as an ordinary bidder or via swaps.
3. **Σ claimable tokens ≤ 5e14.** Clearing price is rounded up, so Σ floor(amountᵢ·2³²/price) ≤ Σ amountᵢ·2³²/price ≤ 5e14.
3b. **Price continuity.** Auction: pool = (raised, 5e14) ⇒ spot = clearing. Open → Raydium: tokens deposited = pool_tokens·real/(virtual+real), remainder burned ⇒ Raydium opening price = last curve price.
4. **Escrow solvency.** After settle, escrow balance = Σ(depositᵢ − revealed amountᵢ) + Σ unrevealed depositsᵢ = exactly the refunds still owed.
5. **Fee vault solvency.** fee_vault balance ≥ creator_fees_owed + Σ pending holder rewards + rewards_pending. Rewards are paid from the Q64 index with floor rounding; dust stays in the vault.
6. **Virtual reserves never exceed real balances.** Incoming pair transfers are checked by balance delta; extensions that could take a cut are rejected at create. Extra tokens sent directly to pool/escrow are ignored (not stealable, not counted).
7. **One bid per wallet per launch** (PDA uniqueness). With `require_human`, one bid per Civic Uniqueness pass holder.
8. **Split conservation.** creator + buyback + trader + treasury + holders == fee for every swap (holders is the remainder). Σ(epoch pools) == trader_accrued.
9. **Buyback can only burn.** The only sink for `buyback_accrued` is `buyback_and_burn`, whose only outputs are a transfer into the RSRCT pool/fee vault and a `burn` from the RSRCT vault; `pool_tokens` and `burned` are updated by exactly `out`.
10. **Trader pools are closed.** An epoch's pool leaves the fee vault only via claims proportional to verified volume (Σ ≤ pool) or a sweep to holders when verified volume is zero.

## 4b. Raydium graduation (venue = 1)
`graduate_raydium` moves `real_reserve` (pool_pair − virtual_pair) and `pool_tokens` from our PDAs into a new Raydium CP-Swap pool and burns the LP. Because Raydium requires a System-owned signer as pool creator and pulls deposits from token accounts it owns, the assets transit the caller's ATAs within the instruction; we assert post-CPI that both ATAs hold exactly their pre-call balances (invariant 11). Reviewers: (a) can the caller front-run with a pre-existing pool at the same PDA to make Raydium's `initialize` fail or behave differently? (b) can an ATA with a delegate or close authority set by the caller leak funds between our transfer and Raydium's pull within the same instruction? (c) is reading the LP balance from raw account data (offset 64) safe for the SPL Token LP mint Raydium creates?
11. **Graduation conservation.** After `graduate_raydium`: caller ATA balances == pre-call balances; our pool account == virtual_pair's real counterpart (0); vault == tokens still owed to bidders; LP mint supply == 0.

## 5. Pair asset policy (Token-2022)
Rejected at `create_launch`: `TransferFeeConfig`, `TransferHook`, `NonTransferable`, `ConfidentialTransferMint`, `ConfidentialTransferFeeConfig`, `ConfidentialMintBurn`. Allowed: metadata/pointers, `PermanentDelegate`, `MintCloseAuthority`, `DefaultAccountState`, interest-bearing, pausable, group extensions.
Known residual risk: a `PermanentDelegate` issuer (e.g. a regulated tokenized-stock issuer) can move pair tokens out of our PDAs; a `Pausable` issuer can halt transfers. This is issuer risk inherent to those assets and is disclosed in the UI; it cannot be mitigated by this program. Reviewers: confirm no other extension can break invariant 6.

## 6. Proof of personhood
`commit_bid` optionally takes a Civic Gateway token account. Checks: account owner == `gatem74V238djXdzWnJf94Wo1DcnuGkfijbf3AuBhfs`; borsh layout (version, Option parent, owner_wallet, Option identity, gatekeeper_network, issuing_gatekeeper, state, Option expire_time); owner_wallet == bidder; network == launch.gatekeeper_network (Uniqueness `uniqobk8oGh4XBLMqM68K8M2zNu3CdYX7q5go7whQiv`); state == Active; not expired. The PDA address itself is not re-derived — ownership by the Gateway program plus field checks is what we rely on. Reviewers: confirm a non-token account owned by Gateway cannot satisfy the parse.

## 7. Arithmetic
All reserve math in u128. `cp_out`: reserve_in·reserve_out ≤ ~1e15·1e15 = 2e29 ≪ 2¹²⁸. Reward index: holders·2⁶⁴/total_staked with holders ≤ 2⁶⁴ → ≤ 2¹²⁸/total_staked, fine for total_staked ≥ 1. Overflow-checked adds on all accumulators; subtractions are guarded by preceding `require!`s.

## 8. Tests (local validator)
- `e2e4.js`: SOL-paired launch with `require_human` (no pass → `HumanPassRequired`; other wallet's pass → rejected; own pass → ok), Token-2022 stock-paired launch; full lifecycle incl. duplicate bid, double claim, double fee claim, staking math (pending == exactly 95% of fees), unstake.
- `exttest.js`: transfer-fee mint rejected; metadata-pointer mint accepted; balance-delta commit.
- Devnet: launch `$TRENCH` ran through bid → reveal → settle → claim → swap → stake.

## 9. Out of scope / known gaps
- No on-chain price oracle; the pool starts at the clearing price by construction.
- Reveal window (10 min) and batch length (60–900 s) are constants; mainnet will fix batch at 900 s.
- Fees accrued while nobody is staked go to the first stakers (documented incentive, not a bug).
- Unrevealed bids are refunded, never slashed — griefing by committing and not revealing costs the griefer nothing but gains nothing either (their deposit never influences the price).
- No RSRCT token / buyback yet.

## 10. Questions we want answered
1. Any path for the creator or upgrade authority (pre-freeze) to extract pool funds other than a malicious upgrade?
2. Rounding: can repeated tiny swaps drain more than the fee from the pool?
3. Is `Option<UncheckedAccount>` None-detection (program id placeholder) exploitable when `require_human` is set?
4. Any Token-2022 extension beyond §5 that breaks invariant 6?
5. Epoch boundaries: a swap whose client-computed epoch lags the chain fails with `WrongEpoch` (fail-closed). Can a trader split volume across epochs to gain more than pro-rata?
6. `buyback_and_burn` is permissionless; each call now spends at most 1% of the RSRCT pool's real reserve (`BUYBACK_MAX_BPS_OF_POOL`), the rest stays accrued. Is 1% the right cap, and is repeated calling in one block a concern?
7. Rounding in `claim_trader_reward` (floor) leaves dust in the fee vault forever — acceptable?
