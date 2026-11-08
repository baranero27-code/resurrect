# Resurrect — audit package

Prepared for external reviewers. Everything here is verifiable against `src/lib.rs` (Anchor 0.31, ~560 lines) and the tests in this folder.

## 1. What the program is
A launchpad for fixed-supply SPL tokens on Solana. Each launch runs a sealed-bid (commit–reveal) batch auction with a single clearing price, then seeds a constant-product pool that the program owns and that has **no withdrawal path**. Every launch is denominated in a creator-chosen "pair asset" (SPL Token or Token-2022). Swap fees are split between the creator (≤20%) and token holders who stake.

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

`Launch` fields: creator, mint, vault, pair_mint, pair_token_program, escrow, pool_pair_account, fee_vault, name, symbol, start/end/reveal_end timestamps, creator_fee_bps, pair_decimals, min_bid, bids, revealed, total_revealed, clearing_price_q32 (u128, pair base units per token base unit × 2³²), settled, pool_pair, pool_tokens (virtual reserves), fees_accrued, creator_fees_owed/claimed, require_human, gatekeeper_network, stake_vault, total_staked, reward_index (u128, Q64 per staked base unit), rewards_pending, rewards_distributed, bump.

## 3. Instructions and preconditions
| Ix | Who | Checks |
|---|---|---|
| `create_launch(name, symbol, batch_secs, creator_fee_bps, require_human)` | creator | mint: supply == 1e15, decimals 6, mint & freeze authority None; pair ≠ mint; pair extension policy (§5); batch 60–900 s; fee ≤ 2000 bps; transfers 100% of supply creator → vault |
| `commit_bid(commitment, deposit)` | bidder | now < end_ts; deposit ≥ min_bid; if require_human: valid Civic pass (§6); pair transfer bidder → escrow; **escrow delta must equal deposit** |
| `reveal_bid(amount, salt)` | bidder | end_ts ≤ now < reveal_end_ts; not revealed; min_bid ≤ amount ≤ deposit; sha256(bidder‖amount_le‖salt) == commitment |
| `settle()` | anyone | now ≥ reveal_end_ts; not settled. If total_revealed == 0: settled, pool stays empty. Else price = ⌈total_revealed·2³² / 8e14⌉; escrow → pool (total_revealed); pool_tokens = 2e14 |
| `claim()` | bidder | settled; not claimed. Refund = deposit − amount (revealed) or deposit (unrevealed). Tokens = amount·2³² / price (floor) |
| `swap_pair_for_tokens(in, min_out)` | anyone | settled & pool_tokens > 0; fee = in/100; out = cp(net); out ≥ min_out; **pool delta == net** |
| `swap_tokens_for_pair(in, min_out)` | anyone | gross = cp(in); fee = gross/100; out = gross − fee ≥ min_out |
| `claim_creator_fees()` | creator (has_one) | owed > 0 |
| `stake(amount)` / `unstake(amount)` | holder | settled (stake); amount ≤ staked (unstake); rewards settled to `owed` before any change |
| `claim_rewards()` | staker | owed > 0 after settling index |

## 4. Invariants we claim (please try to break them)
1. **No liquidity withdrawal.** The only instructions that move tokens out of `pool` are the two swaps, and each one moves out exactly `cp_out(...)` (+fee) of the opposite side in the same transaction. There is no close, no admin, no migration.
2. **Creator never receives launch tokens from the program** except via `claim` as an ordinary bidder or via swaps.
3. **Σ claimable tokens ≤ 8e14.** Clearing price is rounded up, so Σ floor(amountᵢ·2³²/price) ≤ Σ amountᵢ·2³²/price ≤ 8e14.
4. **Escrow solvency.** After settle, escrow balance = Σ(depositᵢ − revealed amountᵢ) + Σ unrevealed depositsᵢ = exactly the refunds still owed.
5. **Fee vault solvency.** fee_vault balance ≥ creator_fees_owed + Σ pending holder rewards + rewards_pending. Rewards are paid from the Q64 index with floor rounding; dust stays in the vault.
6. **Virtual reserves never exceed real balances.** Incoming pair transfers are checked by balance delta; extensions that could take a cut are rejected at create. Extra tokens sent directly to pool/escrow are ignored (not stealable, not counted).
7. **One bid per wallet per launch** (PDA uniqueness). With `require_human`, one bid per Civic Uniqueness pass holder.

## 5. Pair asset policy (Token-2022)
Rejected at `create_launch`: `TransferFeeConfig`, `TransferHook`, `NonTransferable`, `ConfidentialTransferMint`, `ConfidentialTransferFeeConfig`, `ConfidentialMintBurn`. Allowed: metadata/pointers, `PermanentDelegate`, `MintCloseAuthority`, `DefaultAccountState`, interest-bearing, pausable, group extensions.
Known residual risk: a `PermanentDelegate` issuer (e.g. a regulated tokenized-stock issuer) can move pair tokens out of our PDAs; a `Pausable` issuer can halt transfers. This is issuer risk inherent to those assets and is disclosed in the UI; it cannot be mitigated by this program. Reviewers: confirm no other extension can break invariant 6.

## 6. Proof of personhood
`commit_bid` optionally takes a Civic Gateway token account. Checks: account owner == `gatem74V238djXdzWnJf94Wo1DcnuGkfijbf3AuBhfs`; borsh layout (version, Option parent, owner_wallet, Option identity, gatekeeper_network, issuing_gatekeeper, state, Option expire_time); owner_wallet == bidder; network == launch.gatekeeper_network (Uniqueness `uniqobk8oGh4XBLMqM68K8M2zNu3CdYX7q5go7whQiv`); state == Active; not expired. The PDA address itself is not re-derived — ownership by the Gateway program plus field checks is what we rely on. Reviewers: confirm a non-token account owned by Gateway cannot satisfy the parse.

## 7. Arithmetic
All reserve math in u128. `cp_out`: reserve_in·reserve_out ≤ ~1e15·2e14 = 2e29 ≪ 2¹²⁸. Reward index: holders·2⁶⁴/total_staked with holders ≤ 2⁶⁴ → ≤ 2¹²⁸/total_staked, fine for total_staked ≥ 1. Overflow-checked adds on all accumulators; subtractions are guarded by preceding `require!`s.

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
