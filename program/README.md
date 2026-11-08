# Resurrect program (devnet MVP)

Program id (devnet): `5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t`

Source: `src/lib.rs` (Anchor 0.31). Binary: `resurrect.so` (built with platform-tools v1.52, stripped).
Browser client: `../js/rsrct.js` (no Anchor TS dependency; instruction encoding by hand).

## Lifecycle
1. `create_launch` — creator transfers the entire fixed supply (1,000,000,000 × 10⁶) into the program vault.
   Mint must have 6 decimals, supply exactly 1B, mint & freeze authority revoked. Creator keeps nothing.
2. `commit_bid` — escrow SOL ≥ bid, store `sha256(bidder ‖ amount_le ‖ salt)`. One bid PDA per wallet per launch.
3. `reveal_bid` — after the batch closes (≤15 min), within a 10‑minute window. Hash must match.
4. `settle` — anyone. Clearing price = ⌈total revealed SOL / 800M tokens⌉ (Q32). All raised SOL + 20% of supply seed a
   constant‑product pool owned by the program. There is no withdraw/close instruction for the pool.
5. `claim` — tokens at the clearing price + refund of (deposit − bid). Unrevealed bids: full refund, no tokens.
6. `swap_sol_for_tokens` / `swap_tokens_for_sol` — 1% fee on the SOL side → fee vault PDA.

## Not in this MVP (planned)
- Proof of personhood gate (Civic Pass / World ID) on bids and swaps
- Fee distribution to holders / buyback / trader airdrops (fee vault accrues only)
- Non‑SOL pairing assets
- Independent audit — **required before any mainnet deployment**
