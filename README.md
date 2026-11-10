# Resurrect

Fair memecoin launches on Solana. No bots, no bundlers, no rugs, no insider KOLs.

- **Site:** https://resurrect.fun/ — the ICU (manifesto) · `launchpad.html` (preview) · `app.html` (live on devnet)
- **Program (devnet):** `5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t` — source in [`program/`](program/)

## What the program enforces
| | |
|---|---|
| Launch | **Auction**: 15-minute sealed batch (commit–reveal), everyone pays one clearing price, pool opens at that price. **Open**: bonding curve, trading from the first second, protected start (90 s fixed opening price, 5-min 1.5%-per-wallet cap, creator locked out, fee 5%→1% over 15 min, surcharge to holders). |
| Bids (Auction) | One bid per wallet. Optional **proof of personhood** (Civic Uniqueness pass, checked on chain) → one human, one wallet, one bid. |
| Supply | 1,000,000,000 fixed. Mint & freeze authority revoked before the launch is accepted. |
| Creator | **0% allocation, no first buy.** Creator can only bid like everyone else. |
| Pairing | Any SPL Token or Token-2022 asset: SOL, USDC, xStocks, Ondo, Sunrise, commodities — 2,400+ in the picker. |
| Liquidity | Auction: 50% of supply + everything raised → constant-product pool **owned by the program. No withdraw instruction exists.** Open: whole supply on the curve, virtual reserve never paid out. Optional **Raydium** venue: pool graduates to Raydium with the LP burned. |
| Fees | 1% per swap, in the pairing asset, split on chain: 35% staked holders (+ unused creator share) · 25% RSRCT buyback & burn · 20% verified traders (weekly) · 0–10% creator · 10% treasury. |

## Repo layout
```
index.html        the ICU — manifesto page
launchpad.html    product preview (simulated coins; hero/analytics numbers read live from devnet)
app.html          live devnet app: create, bid, reveal, settle, claim, trade, stake, claim rewards
js/rsrct.js       hand-encoded web3.js client for the program (no Anchor TS)
js/rsrct-bundle.js  esbuild bundle: @solana/web3.js + spl-token + client
js/assets.json    2,418 pairing assets (mint, symbol, category, icon)
program/          Anchor program source, stripped .so, e2e test, README
dev/deploy.html   browser-based deploy/upgrade console (program secret stays in localStorage, never in git)
```

## Status
Devnet. Unaudited. No real funds. Mainnet requires an independent audit and moving the upgrade authority to a multisig.


**X:** https://x.com/resurrectpad
