# Mainnet day-0 checklist

Everything below is in order. Do not skip steps; several are irreversible.

## A. Before deploying anything
1. **Audit done**, findings fixed, re-reviewed. (`program/AUDIT.md` is the package.)
2. Change the constants in `program/src/lib.rs` and rebuild with `--features mainnet`:
   - `ADMIN` → your mainnet admin wallet (hardware wallet). This key can only init/update `Config` (treasury wallet, RSRCT mint); it cannot touch pools or fees.
   - Confirm `RAYDIUM_CPMM` / `RAYDIUM_FEE_RECEIVER` mainnet ids, `MIN_BATCH_SECS = 900`.
3. Paid RPC (Helius / Triton / QuickNode). Put it behind `?rpc=` default for mainnet in `app.html`. Public mainnet RPC will rate-limit the app immediately.
4. Domain live: `resurrect.fun` → GitHub Pages (CNAME file in repo + DNS A/AAAA records), HTTPS on. Update `og:url` / `og:image` to the domain.
5. X account exists, bio links the domain, first post drafted (`social/x-posts.md`), trailer uploaded.

## B. Deploy (same program keypair as devnet, so the program id stays `5iHS…VM7t`)
6. Fund the deploy wallet: ~6 SOL (rent for ~750 KB programdata ≈ 5.3 SOL, buffer is refunded) + 1 SOL for configs.
7. Deploy `program/resurrect-mainnet.so` (CLI or `dev/deploy.html` with `?cluster=mainnet`). Verify `solana program show` → executable, authority = deploy wallet.
8. `init_config(treasury = <treasury wallet>)` signed by `ADMIN`. Treasury should be a multisig (Squads) from day one.
9. **Upgrade authority → Squads multisig** immediately (`program/README.md` runbook). Publish the multisig address on the site (the footer reads it live).

## C. RSRCT
10. Launch **RSRCT through Resurrect itself**: Auction mode, Resurrect venue, creator fee 0%, pairing SOL, humans-only ON. Announce the batch time 24h ahead. Nobody — including you — gets an allocation; bid like everyone else if you want some.
11. After settle: `set_config(rsrct_mint = <RSRCT mint>)`. From this moment 25% of every fee on every launch buys and burns RSRCT (per-call cap: 1% of the RSRCT pool's real reserve).

## D. Open the doors
12. **First thing on mainnet:** open any launch paired with a non-SOL asset (e.g. TSLAx or USDC) and try a bid/buy with "pay with SOL" on, using a tiny amount (0.01 SOL). Jupiter auto-swap cannot be tested on devnet, so this is the first live check of that path; if it fails, uncheck the switch and buy with the pair asset directly (the on-chain path is unaffected). Then smoke-test one tiny Auction launch and one Open launch with real but small SOL (0.1). Check: bids, reveal, settle, claim, swap, stake, trader epoch record, treasury collection, graduation to Raydium (Open + Raydium with 10 SOL depth — the pool shows up on DexScreener within a minute).
13. Post 7 (launch day) from `social/x-posts.md`, with the domain. Pin it.
14. Watch the first batches live; be ready to answer "where is the salt" (browser localStorage — same device to reveal).

## E. Within the first weeks
15. Freeze the program (`--final`) once the audit is closed and the multisig agrees. Irreversible; announce it.
16. Jupiter DEX integration + DexScreener listing for the **Resurrect pool** venue so coins that stay with us are also visible everywhere.
17. Rotate the treasury into the multisig if it isn't already; publish monthly what it paid for.
