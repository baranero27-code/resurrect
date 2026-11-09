//! Resurrect — sealed-batch launchpad (devnet v3: any-asset pairing · holder rewards · proof of personhood)
//!
//! Every launch is paired with a `pair_mint`: SOL (as wrapped SOL), USDC, a tokenized stock,
//! an ETF, a commodity token… any SPL Token or Token-2022 mint. Bids, the pool, fees and
//! holder rewards are all denominated in that asset.
//!
//! Lifecycle of a launch:
//!   create_launch  -> creator deposits the full fixed supply into the program vault.
//!                     Escrow / pool / fee vault token accounts for the pair asset are created,
//!                     all owned by the launch PDA. The creator keeps nothing.
//!   commit_bid     -> bidders escrow pair tokens and commit hash(bidder, amount, salt). One bid per wallet.
//!   reveal_bid     -> after the batch closes, bidders reveal (amount, salt)
//!   settle         -> one clearing price for everyone; 80% of supply goes to bidders pro-rata,
//!                     20% of supply + ALL raised pair tokens seed a constant-product pool owned
//!                     by the program. There is no withdraw instruction: liquidity cannot be pulled.
//!   claim          -> bidder receives tokens + refund of escrow above their bid
//!   swap           -> trade against the pool, 1% fee (in the pair asset) accrues to the fee vault
//!   Fee split of every swap fee: 35% staked holders · 25% RSRCT buyback & burn (on chain, through the RSRCT pool)
//!   · 20% verified traders (weekly, pro-rata by volume) · 0–10% creator · 10% treasury. Creator's unused share → holders.
//!   claim_creator_fees / collect_treasury / buyback_and_burn / claim_trader_reward / sweep_epoch move each share out.
//!   stake / unstake / claim_rewards -> holders deposit launch tokens (no lockup, no fee) and earn the holder
//!                     share of every swap fee, paid in the pair asset, pull-based
//!   Launches may require a Civic "Uniqueness" pass (one per human) to bid: one human, one wallet, one bid.
use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hashv;
use anchor_spl::token::{self, Token, Transfer};
use anchor_spl::token_interface::{self as ti, Mint, TokenAccount, TokenInterface, TransferChecked};
use anchor_spl::token_2022::spl_token_2022::{self, extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions}};

declare_id!("5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t");

pub const SUPPLY: u64 = 1_000_000_000_000_000; // 1,000,000,000 tokens with 6 decimals
pub const SALE_BPS: u64 = 8_000;               // 80% of supply sold in the batch
pub const FEE_BPS: u64 = 100;                  // 1% swap fee
#[cfg(feature = "mainnet")] pub const MIN_BATCH_SECS: i64 = 15 * 60;   // mainnet: batches are always 15 minutes
#[cfg(not(feature = "mainnet"))] pub const MIN_BATCH_SECS: i64 = 60; // devnet: short batches for testing
pub const MAX_BATCH_SECS: i64 = 15 * 60;
pub const REVEAL_SECS: i64 = 10 * 60;
pub const MAX_CREATOR_FEE_BPS: u16 = 1_000;    // creator may take at most 10% of fees
/// Fee split (basis points of every swap fee). Creator takes 0–10%; whatever the creator leaves on the table goes to holders.
pub const HOLDER_BPS: u64 = 3_500;    // staked holders, pull-based, paid in the pair asset
pub const BUYBACK_BPS: u64 = 2_500;   // RSRCT buyback & burn, executed on chain through the RSRCT pool
pub const TRADER_BPS: u64 = 2_000;    // verified traders, pro-rata by weekly volume
pub const TREASURY_BPS: u64 = 1_000;  // audits, infra
#[cfg(not(feature = "localtest"))] pub const EPOCH_SECS: i64 = 7 * 24 * 3600;
#[cfg(feature = "localtest")] pub const EPOCH_SECS: i64 = 90;
/// Who may initialise/update the global config (treasury wallet, RSRCT mint). Replace for mainnet.
#[cfg(not(feature = "localtest"))] pub const ADMIN: Pubkey = anchor_lang::solana_program::pubkey!("HEtZUFGTduAwAq5nuMjkSpYn1zMXNPNtDAEi2tnW5S7X");
#[cfg(feature = "localtest")] pub const ADMIN: Pubkey = anchor_lang::solana_program::pubkey!("49vFcHtckto46AnhrhtjjDftN9UBLSSuB2YhoQT5PERb");
/// Civic Gateway program — proof-of-personhood passes live here
pub const GATEWAY_PROGRAM: Pubkey = anchor_lang::solana_program::pubkey!("gatem74V238djXdzWnJf94Wo1DcnuGkfijbf3AuBhfs");
/// Civic "Uniqueness" gatekeeper network: one pass per human (same key on devnet and mainnet)
pub const UNIQUENESS_NETWORK: Pubkey = anchor_lang::solana_program::pubkey!("uniqobk8oGh4XBLMqM68K8M2zNu3CdYX7q5go7whQiv");

#[program]
pub mod resurrect {
    use super::*;

    pub fn create_launch(ctx: Context<CreateLaunch>, name: String, symbol: String, batch_secs: i64, creator_fee_bps: u16, require_human: bool) -> Result<()> {
        require!(name.len() <= 32 && symbol.len() >= 1 && symbol.len() <= 10, ErrorCode::BadParams);
        require!(batch_secs >= MIN_BATCH_SECS && batch_secs <= MAX_BATCH_SECS, ErrorCode::BadParams);
        require!(creator_fee_bps <= MAX_CREATOR_FEE_BPS, ErrorCode::BadParams);
        let mint = &ctx.accounts.mint;
        // fixed supply: the whole supply must already exist and authorities must be gone
        require!(mint.supply == SUPPLY, ErrorCode::BadSupply);
        require!(mint.mint_authority.is_none(), ErrorCode::MintAuthorityPresent);
        require!(mint.freeze_authority.is_none(), ErrorCode::FreezeAuthorityPresent);
        require!(mint.decimals == 6, ErrorCode::BadParams);
        let pair = &ctx.accounts.pair_mint;
        require!(pair.key() != mint.key(), ErrorCode::BadParams);
        check_pair_mint_extensions(&pair.to_account_info())?;
        // move 100% of supply into the program vault — creator keeps nothing
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.creator_token.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
                authority: ctx.accounts.creator.to_account_info(),
            }),
            SUPPLY,
        )?;
        let now = Clock::get()?.unix_timestamp;
        // minimum bid: 0.01 units of the pair asset (1 base unit for assets with < 2 decimals)
        let min_bid = if pair.decimals >= 2 { 10u64.pow(pair.decimals as u32 - 2) } else { 1 };
        let l = &mut ctx.accounts.launch;
        l.creator = ctx.accounts.creator.key();
        l.mint = mint.key();
        l.vault = ctx.accounts.vault.key();
        l.pair_mint = pair.key();
        l.pair_decimals = pair.decimals;
        l.pair_token_program = ctx.accounts.pair_token_program.key();
        l.escrow = ctx.accounts.escrow.key();
        l.pool_pair_account = ctx.accounts.pool_pair.key();
        l.fee_vault = ctx.accounts.fee_vault.key();
        l.name = name;
        l.symbol = symbol;
        l.start_ts = now;
        l.end_ts = now + batch_secs;
        l.reveal_end_ts = l.end_ts + REVEAL_SECS;
        l.creator_fee_bps = creator_fee_bps;
        l.min_bid = min_bid;
        l.bids = 0;
        l.revealed = 0;
        l.total_revealed = 0;
        l.clearing_price_q32 = 0; // pair base units per token base unit * 2^32, set at settle
        l.settled = false;
        l.pool_pair = 0;
        l.pool_tokens = 0;
        l.fees_accrued = 0;
        l.creator_fees_owed = 0;
        l.creator_fees_claimed = 0;
        l.require_human = require_human;
        l.gatekeeper_network = UNIQUENESS_NETWORK;
        l.stake_vault = ctx.accounts.stake_vault.key();
        l.total_staked = 0;
        l.reward_index = 0;
        l.rewards_pending = 0;
        l.rewards_distributed = 0;
        l.settle_ts = 0; l.buyback_accrued = 0; l.buyback_total = 0; l.trader_accrued = 0; l.trader_total = 0; l.treasury_owed = 0; l.treasury_total = 0; l.burned = 0;
        l.bump = ctx.bumps.launch;
        emit!(LaunchCreated { launch: l.key(), mint: l.mint, pair_mint: l.pair_mint, end_ts: l.end_ts });
        Ok(())
    }

    /// Escrow `deposit` pair tokens and commit to hash(bidder, amount, salt). Deposit must cover the bid.
    pub fn commit_bid(ctx: Context<CommitBid>, commitment: [u8; 32], deposit: u64) -> Result<()> {
        let l = &ctx.accounts.launch;
        let now = Clock::get()?.unix_timestamp;
        require!(now < l.end_ts, ErrorCode::BatchClosed);
        require!(deposit >= l.min_bid, ErrorCode::BidTooSmall);
        if l.require_human {
            let gt = ctx.accounts.gateway_token.as_ref().ok_or(ErrorCode::HumanPassRequired)?;
            verify_human(gt, &ctx.accounts.bidder.key(), &l.gatekeeper_network, Clock::get()?.unix_timestamp)?;
        }
        let before = ctx.accounts.escrow.amount;
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.bidder_pair.to_account_info(), &ctx.accounts.escrow.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.bidder.to_account_info(), deposit, l.pair_decimals, None)?;
        ctx.accounts.escrow.reload()?;
        // trust what actually arrived, never the requested amount
        require!(ctx.accounts.escrow.amount - before == deposit, ErrorCode::TransferShortfall);
        let l = &mut ctx.accounts.launch;
        let b = &mut ctx.accounts.bid;
        b.launch = l.key();
        b.bidder = ctx.accounts.bidder.key();
        b.commitment = commitment;
        b.deposit = deposit;
        b.amount = 0;
        b.revealed = false;
        b.claimed = false;
        b.bump = ctx.bumps.bid;
        l.bids += 1;
        Ok(())
    }

    /// Reveal after the batch closes. amount <= deposit. Unrevealed bids are excluded and refunded on claim.
    pub fn reveal_bid(ctx: Context<RevealBid>, amount: u64, salt: [u8; 32]) -> Result<()> {
        let l = &mut ctx.accounts.launch;
        let now = Clock::get()?.unix_timestamp;
        require!(now >= l.end_ts && now < l.reveal_end_ts, ErrorCode::NotRevealWindow);
        let b = &mut ctx.accounts.bid;
        require!(!b.revealed, ErrorCode::AlreadyRevealed);
        require!(amount >= l.min_bid && amount <= b.deposit, ErrorCode::BadReveal);
        let h = hashv(&[b.bidder.as_ref(), &amount.to_le_bytes(), &salt]);
        require!(h.to_bytes() == b.commitment, ErrorCode::BadReveal);
        b.amount = amount;
        b.revealed = true;
        l.revealed += 1;
        l.total_revealed = l.total_revealed.checked_add(amount).ok_or(ErrorCode::Overflow)?;
        Ok(())
    }

    /// Anyone can settle once the reveal window closes. One clearing price; pool is seeded and sealed.
    pub fn settle(ctx: Context<Settle>) -> Result<()> {
        let l = &ctx.accounts.launch;
        let now = Clock::get()?.unix_timestamp;
        require!(now >= l.reveal_end_ts, ErrorCode::NotRevealWindow);
        require!(!l.settled, ErrorCode::AlreadySettled);
        let sale_tokens = SUPPLY / 10_000 * SALE_BPS;
        let pool_tokens = SUPPLY - sale_tokens;
        if l.total_revealed == 0 {
            // nobody showed up: nothing is sold, the vault simply holds the supply; pool stays empty
            let l = &mut ctx.accounts.launch;
            l.settled = true;
            return Ok(());
        }
        // clearing price: pair base units per token base unit, fixed point 2^32
        let num = (l.total_revealed as u128) << 32;
        let price_q32 = (num + (sale_tokens as u128) - 1) / (sale_tokens as u128); // ceil: allocations never exceed sale supply
        require!(price_q32 > 0, ErrorCode::BadParams);
        let raised = l.total_revealed;
        let seeds: &[&[u8]] = &[b"launch", l.mint.as_ref(), &[l.bump]];
        // all raised pair tokens move to the pool; 20% of supply is the pool's token side
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.escrow.to_account_info(), &ctx.accounts.pool_pair.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.launch.to_account_info(), raised, l.pair_decimals, Some(seeds))?;
        let l = &mut ctx.accounts.launch;
        l.clearing_price_q32 = price_q32;
        l.settle_ts = Clock::get()?.unix_timestamp;
        l.pool_pair = raised;
        l.pool_tokens = pool_tokens;
        l.settled = true;
        emit!(Settled { launch: l.key(), clearing_price_q32: price_q32, raised, pool_tokens });
        Ok(())
    }

    /// Bidder receives tokens = amount / price and a refund of (deposit - amount). Unrevealed: full refund.
    pub fn claim(ctx: Context<Claim>) -> Result<()> {
        let l = &ctx.accounts.launch;
        require!(l.settled, ErrorCode::NotSettled);
        let b = &mut ctx.accounts.bid;
        require!(!b.claimed, ErrorCode::AlreadyClaimed);
        b.claimed = true;
        let won = b.revealed && l.clearing_price_q32 > 0;
        let refund = if won { b.deposit - b.amount } else { b.deposit };
        let seeds: &[&[u8]] = &[b"launch", l.mint.as_ref(), &[l.bump]];
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.escrow.to_account_info(), &ctx.accounts.bidder_pair.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.launch.to_account_info(), refund, l.pair_decimals, Some(seeds))?;
        if won {
            let tokens = (((b.amount as u128) << 32) / l.clearing_price_q32) as u64;
            token::transfer(
                CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), Transfer {
                    from: ctx.accounts.vault.to_account_info(),
                    to: ctx.accounts.bidder_token.to_account_info(),
                    authority: ctx.accounts.launch.to_account_info(),
                }, &[seeds]),
                tokens,
            )?;
        }
        Ok(())
    }

    /// Buy tokens with the pair asset from the sealed pool. 1% fee on the pair side accrues to the fee vault.
    pub fn swap_pair_for_tokens(mut ctx: Context<Swap>, pair_in: u64, min_tokens_out: u64, _epoch: u32) -> Result<()> {
        let l = &ctx.accounts.launch;
        require!(l.settled && l.pool_tokens > 0, ErrorCode::NotSettled);
        require!(pair_in > 0, ErrorCode::BadParams);
        let fee = pair_in * FEE_BPS / 10_000;
        let net = pair_in - fee;
        let out = cp_out(net as u128, l.pool_pair as u128, l.pool_tokens as u128);
        require!(out >= min_tokens_out && out > 0, ErrorCode::Slippage);
        let dec = l.pair_decimals;
        let before = ctx.accounts.pool_pair.amount;
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.trader_pair.to_account_info(), &ctx.accounts.pool_pair.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.trader.to_account_info(), net, dec, None)?;
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.trader_pair.to_account_info(), &ctx.accounts.fee_vault.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.trader.to_account_info(), fee, dec, None)?;
        ctx.accounts.pool_pair.reload()?;
        require!(ctx.accounts.pool_pair.amount - before == net, ErrorCode::TransferShortfall);
        let seeds: &[&[u8]] = &[b"launch", l.mint.as_ref(), &[l.bump]];
        token::transfer(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.vault.to_account_info(), to: ctx.accounts.trader_token.to_account_info(),
                authority: ctx.accounts.launch.to_account_info(),
            }, &[seeds]), out)?;
        after_swap(&mut ctx, fee, net, _epoch)?;
        let l = &mut ctx.accounts.launch;
        l.pool_pair += net;
        l.pool_tokens -= out;
        emit!(Swapped { launch: l.key(), trader: ctx.accounts.trader.key(), pair_in, tokens_out: out, tokens_in: 0, pair_out: 0 });
        Ok(())
    }

    /// Sell tokens to the pool for the pair asset. 1% fee on the pair side.
    pub fn swap_tokens_for_pair(mut ctx: Context<Swap>, tokens_in: u64, min_pair_out: u64, _epoch: u32) -> Result<()> {
        let l = &ctx.accounts.launch;
        require!(l.settled && l.pool_tokens > 0, ErrorCode::NotSettled);
        require!(tokens_in > 0, ErrorCode::BadParams);
        let gross = cp_out(tokens_in as u128, l.pool_tokens as u128, l.pool_pair as u128);
        let fee = gross * FEE_BPS / 10_000;
        let out = gross - fee;
        require!(out >= min_pair_out && out > 0, ErrorCode::Slippage);
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.trader_token.to_account_info(), to: ctx.accounts.vault.to_account_info(),
                authority: ctx.accounts.trader.to_account_info(),
            }), tokens_in)?;
        let dec = l.pair_decimals;
        let seeds: &[&[u8]] = &[b"launch", l.mint.as_ref(), &[l.bump]];
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.pool_pair.to_account_info(), &ctx.accounts.trader_pair.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.launch.to_account_info(), out, dec, Some(seeds))?;
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.pool_pair.to_account_info(), &ctx.accounts.fee_vault.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.launch.to_account_info(), fee, dec, Some(seeds))?;
        after_swap(&mut ctx, fee, gross, _epoch)?;
        let l = &mut ctx.accounts.launch;
        l.pool_pair -= gross;
        l.pool_tokens += tokens_in;
        emit!(Swapped { launch: l.key(), trader: ctx.accounts.trader.key(), pair_in: 0, tokens_out: 0, tokens_in, pair_out: out });
        Ok(())
    }

    /// Creator withdraws their accrued share of swap fees (creator_fee_bps of every fee). Everything else
    /// stays in the fee vault for holder rewards. The creator can never touch the pool.
    pub fn claim_creator_fees(ctx: Context<ClaimCreatorFees>) -> Result<()> {
        let l = &ctx.accounts.launch;
        let owed = l.creator_fees_owed;
        require!(owed > 0, ErrorCode::NothingToClaim);
        let seeds: &[&[u8]] = &[b"launch", l.mint.as_ref(), &[l.bump]];
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.fee_vault.to_account_info(), &ctx.accounts.creator_pair.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.launch.to_account_info(), owed, l.pair_decimals, Some(seeds))?;
        let l = &mut ctx.accounts.launch;
        l.creator_fees_owed = 0;
        l.creator_fees_claimed += owed;
        Ok(())
    }


    /// One-time global config: treasury wallet and (later) the RSRCT mint. Admin only.
    pub fn init_config(ctx: Context<InitConfig>, treasury: Pubkey) -> Result<()> {
        let c = &mut ctx.accounts.config;
        c.authority = ctx.accounts.admin.key(); c.treasury = treasury; c.rsrct_mint = Pubkey::default(); c.bump = ctx.bumps.config; Ok(())
    }
    pub fn set_config(ctx: Context<SetConfig>, treasury: Pubkey, rsrct_mint: Pubkey) -> Result<()> {
        let c = &mut ctx.accounts.config; c.treasury = treasury; c.rsrct_mint = rsrct_mint; Ok(())
    }

    /// Anyone: move the treasury share accrued on this launch to the treasury wallet's token account.
    pub fn collect_treasury(ctx: Context<CollectTreasury>) -> Result<()> {
        let l = &ctx.accounts.launch; let owed = l.treasury_owed; require!(owed > 0, ErrorCode::NothingToClaim);
        let (mint_key, bump, dec) = (l.mint, l.bump, l.pair_decimals); let seeds: &[&[u8]] = &[b"launch", mint_key.as_ref(), &[bump]];
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.fee_vault.to_account_info(), &ctx.accounts.treasury_pair.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.launch.to_account_info(), owed, dec, Some(seeds))?;
        let l = &mut ctx.accounts.launch; l.treasury_owed = 0; l.treasury_total += owed; Ok(())
    }

    /// Anyone: spend this launch's accrued buyback share on RSRCT through the RSRCT launch's own pool, and burn what comes out.
    /// Works when the RSRCT launch is paired with the same asset as this launch. Until RSRCT exists the share simply accrues.
    pub fn buyback_and_burn(ctx: Context<BuybackAndBurn>) -> Result<()> {
        let src = &ctx.accounts.launch; let r = &ctx.accounts.rsrct_launch;
        require!(ctx.accounts.config.rsrct_mint != Pubkey::default() && r.mint == ctx.accounts.config.rsrct_mint, ErrorCode::NoRsrct);
        require!(r.pair_mint == src.pair_mint && r.settled && r.pool_tokens > 0, ErrorCode::BadParams);
        let amount = src.buyback_accrued; require!(amount > 0, ErrorCode::NothingToClaim);
        // swap through the RSRCT pool exactly like a trader would (1% fee to the RSRCT launch), then burn the output
        let fee = amount * FEE_BPS / 10_000; let net = amount - fee;
        let out = cp_out(net as u128, r.pool_pair as u128, r.pool_tokens as u128); require!(out > 0, ErrorCode::Slippage);
        let dec = src.pair_decimals; let (sm, sb) = (src.mint, src.bump); let sseeds: &[&[u8]] = &[b"launch", sm.as_ref(), &[sb]];
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.fee_vault.to_account_info(), &ctx.accounts.rsrct_pool_pair.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.launch.to_account_info(), net, dec, Some(sseeds))?;
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.fee_vault.to_account_info(), &ctx.accounts.rsrct_fee_vault.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.launch.to_account_info(), fee, dec, Some(sseeds))?;
        let (rm, rb) = (r.mint, r.bump); let rseeds: &[&[u8]] = &[b"launch", rm.as_ref(), &[rb]];
        token::burn(CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), token::Burn {
            mint: ctx.accounts.rsrct_mint.to_account_info(), from: ctx.accounts.rsrct_vault.to_account_info(), authority: ctx.accounts.rsrct_launch.to_account_info() }, &[rseeds]), out)?;
        let src = &mut ctx.accounts.launch; src.buyback_accrued = 0; src.buyback_total += amount;
        let r = &mut ctx.accounts.rsrct_launch; r.pool_pair += net; r.pool_tokens -= out; r.burned += out; accrue_fee(r, fee)?;
        emit!(Burned { launch: src.key(), pair_spent: amount, rsrct_burned: out });
        Ok(())
    }

    /// Verified trader claims their share of an ended weekly epoch's trader pool: pool × my_volume ÷ epoch_volume.
    pub fn claim_trader_reward(ctx: Context<ClaimTraderReward>, epoch: u32) -> Result<()> {
        let l = &ctx.accounts.launch; let now = Clock::get()?.unix_timestamp;
        require!(now >= epoch_end(l, epoch), ErrorCode::EpochOpen);
        let e = &ctx.accounts.epoch_record; let t = &ctx.accounts.trader_stats;
        require!(!t.claimed && t.volume > 0 && e.volume > 0, ErrorCode::NothingToClaim);
        let share = ((e.pool as u128) * (t.volume as u128) / (e.volume as u128)) as u64; require!(share > 0, ErrorCode::NothingToClaim);
        let (mint_key, bump, dec) = (l.mint, l.bump, l.pair_decimals); let seeds: &[&[u8]] = &[b"launch", mint_key.as_ref(), &[bump]];
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.fee_vault.to_account_info(), &ctx.accounts.trader_pair.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.launch.to_account_info(), share, dec, Some(seeds))?;
        let t = &mut ctx.accounts.trader_stats; t.claimed = true; t.paid = share;
        let e = &mut ctx.accounts.epoch_record; e.claimed += share;
        let l = &mut ctx.accounts.launch; l.trader_total += share;
        Ok(())
    }

    /// If an ended epoch had a trader pool but no verified volume, hand the pool to staked holders instead of leaving it stuck.
    pub fn sweep_epoch(ctx: Context<SweepEpoch>, epoch: u32) -> Result<()> {
        let l = &ctx.accounts.launch; let now = Clock::get()?.unix_timestamp;
        require!(now >= epoch_end(l, epoch), ErrorCode::EpochOpen);
        let e = &mut ctx.accounts.epoch_record; require!(e.volume == 0 && e.pool > 0 && !e.swept, ErrorCode::NothingToClaim);
        let p = e.pool; e.swept = true;
        let l = &mut ctx.accounts.launch; route_to_holders(l, p)?; Ok(())
    }

    /// Deposit launch tokens into the rewards vault to earn the holder share of swap fees (paid in the pair asset).
    /// No lockup, no fee: unstake any time.
    pub fn stake(ctx: Context<Stake>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::BadParams);
        let l = &mut ctx.accounts.launch;
        require!(l.settled, ErrorCode::NotSettled);
        let s = &mut ctx.accounts.stake;
        settle_rewards(l, s);
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.staker_token.to_account_info(), to: ctx.accounts.stake_vault.to_account_info(),
                authority: ctx.accounts.staker.to_account_info(),
            }), amount)?;
        if s.owner == Pubkey::default() { s.owner = ctx.accounts.staker.key(); s.launch = l.key(); s.bump = ctx.bumps.stake; }
        s.amount = s.amount.checked_add(amount).ok_or(ErrorCode::Overflow)?;
        l.total_staked = l.total_staked.checked_add(amount).ok_or(ErrorCode::Overflow)?;
        // if fees piled up while nobody was staked, the first stakers start earning them from now on
        if l.rewards_pending > 0 { let p = l.rewards_pending; l.rewards_pending = 0; l.reward_index += ((p as u128) << 64) / l.total_staked as u128; }
        Ok(())
    }

    pub fn unstake(ctx: Context<Stake>, amount: u64) -> Result<()> {
        let l = &mut ctx.accounts.launch;
        let s = &mut ctx.accounts.stake;
        require!(amount > 0 && amount <= s.amount, ErrorCode::BadParams);
        settle_rewards(l, s);
        let (mint_key, bump) = (l.mint, l.bump);
        let seeds: &[&[u8]] = &[b"launch", mint_key.as_ref(), &[bump]];
        token::transfer(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.stake_vault.to_account_info(), to: ctx.accounts.staker_token.to_account_info(),
                authority: ctx.accounts.launch.to_account_info(),
            }, &[seeds]), amount)?;
        let l = &mut ctx.accounts.launch; let s = &mut ctx.accounts.stake;
        s.amount -= amount;
        l.total_staked -= amount;
        Ok(())
    }

    /// Pull accrued holder rewards (pair asset) out of the fee vault.
    pub fn claim_rewards(ctx: Context<ClaimRewards>) -> Result<()> {
        let l = &mut ctx.accounts.launch;
        let s = &mut ctx.accounts.stake;
        settle_rewards(l, s);
        let owed = s.owed;
        require!(owed > 0, ErrorCode::NothingToClaim);
        let (mint_key, bump, dec) = (l.mint, l.bump, l.pair_decimals);
        let seeds: &[&[u8]] = &[b"launch", mint_key.as_ref(), &[bump]];
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.fee_vault.to_account_info(), &ctx.accounts.staker_pair.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.launch.to_account_info(), owed, dec, Some(seeds))?;
        let l = &mut ctx.accounts.launch; let s = &mut ctx.accounts.stake;
        s.owed = 0; s.claimed += owed;
        l.rewards_distributed += owed;
        emit!(RewardsClaimed { launch: l.key(), staker: s.owner, amount: owed });
        Ok(())
    }
}

fn accrue_fee(l: &mut Launch, fee: u64) -> Result<()> {
    l.fees_accrued = l.fees_accrued.checked_add(fee).ok_or(ErrorCode::Overflow)?;
    let f = fee as u128;
    let creator = (f * l.creator_fee_bps as u128 / 10_000) as u64;
    let buyback = (f * BUYBACK_BPS as u128 / 10_000) as u64;
    let traders = (f * TRADER_BPS as u128 / 10_000) as u64;
    let treasury = (f * TREASURY_BPS as u128 / 10_000) as u64;
    let holders = fee - creator - buyback - traders - treasury; // 35% + whatever the creator left of their 10%, plus rounding dust
    l.creator_fees_owed = l.creator_fees_owed.checked_add(creator).ok_or(ErrorCode::Overflow)?;
    l.buyback_accrued = l.buyback_accrued.checked_add(buyback).ok_or(ErrorCode::Overflow)?;
    l.trader_accrued = l.trader_accrued.checked_add(traders).ok_or(ErrorCode::Overflow)?;
    l.treasury_owed = l.treasury_owed.checked_add(treasury).ok_or(ErrorCode::Overflow)?;
    route_to_holders(l, holders)
}

fn route_to_holders(l: &mut Launch, amount: u64) -> Result<()> {
    if l.total_staked > 0 { l.reward_index += ((amount as u128) << 64) / l.total_staked as u128; }
    else { l.rewards_pending = l.rewards_pending.checked_add(amount).ok_or(ErrorCode::Overflow)?; }
    Ok(())
}

fn epoch_of(l: &Launch, now: i64) -> u32 { ((now - l.settle_ts).max(0) / EPOCH_SECS) as u32 }
fn epoch_end(l: &Launch, epoch: u32) -> i64 { l.settle_ts + (epoch as i64 + 1) * EPOCH_SECS }

/// Fee split + weekly trader accounting. The trader share of this fee lands in the current epoch's pool.
/// Volume only counts toward the trader pool when the trader proves personhood (valid Civic pass passed in).
fn after_swap(ctx: &mut Context<Swap>, fee: u64, volume: u64, epoch_arg: u32) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(epoch_arg == epoch_of(&ctx.accounts.launch, now), ErrorCode::WrongEpoch);
    let before = ctx.accounts.launch.trader_accrued;
    accrue_fee(&mut ctx.accounts.launch, fee)?;
    let added = ctx.accounts.launch.trader_accrued - before;
    let l = &ctx.accounts.launch; let ep = epoch_of(l, now);
    let e = &mut ctx.accounts.epoch_record;
    if e.launch == Pubkey::default() { e.launch = l.key(); e.epoch = ep; e.bump = ctx.bumps.epoch_record; }
    require!(e.epoch == ep, ErrorCode::WrongEpoch);
    e.pool = e.pool.checked_add(added).ok_or(ErrorCode::Overflow)?;
    let verified = match ctx.accounts.gateway_token.as_ref() { Some(gt) => verify_human(gt, &ctx.accounts.trader.key(), &l.gatekeeper_network, now).is_ok(), None => false };
    let t = &mut ctx.accounts.trader_stats;
    if t.trader == Pubkey::default() { t.launch = l.key(); t.trader = ctx.accounts.trader.key(); t.epoch = ep; t.bump = ctx.bumps.trader_stats; }
    require!(t.epoch == ep, ErrorCode::WrongEpoch);
    if verified { t.volume = t.volume.checked_add(volume).ok_or(ErrorCode::Overflow)?; e.volume = e.volume.checked_add(volume).ok_or(ErrorCode::Overflow)?; }
    t.raw_volume = t.raw_volume.checked_add(volume).ok_or(ErrorCode::Overflow)?;
    Ok(())
}

fn settle_rewards(l: &Launch, s: &mut StakeAccount) {
    if s.amount > 0 && l.reward_index > s.index {
        s.owed += (((l.reward_index - s.index) * s.amount as u128) >> 64) as u64;
    }
    s.index = l.reward_index;
}

/// Civic gateway token (borsh layout from the solana-gateway crate)
#[derive(AnchorDeserialize)]
pub struct GatewayToken {
    pub version: u8,
    pub parent_gateway_token: Option<Pubkey>,
    pub owner_wallet: Pubkey,
    pub owner_identity: Option<Pubkey>,
    pub gatekeeper_network: Pubkey,
    pub issuing_gatekeeper: Pubkey,
    pub state: u8, // 0 Active, 1 Frozen, 2 Revoked
    pub expire_time: Option<i64>,
}

fn verify_human(gt: &AccountInfo, owner: &Pubkey, network: &Pubkey, now: i64) -> Result<()> {
    require!(gt.owner == &GATEWAY_PROGRAM, ErrorCode::HumanPassInvalid);
    let data = gt.try_borrow_data()?;
    let mut slice: &[u8] = &data;
    let t = GatewayToken::deserialize(&mut slice).map_err(|_| ErrorCode::HumanPassInvalid)?;
    require!(&t.owner_wallet == owner, ErrorCode::HumanPassInvalid);
    require!(&t.gatekeeper_network == network, ErrorCode::HumanPassInvalid);
    require!(t.state == 0, ErrorCode::HumanPassInvalid);
    if let Some(exp) = t.expire_time { require!(exp > now, ErrorCode::HumanPassExpired); }
    Ok(())
}

/// Pair assets must transfer 1:1 and unconditionally. Token-2022 extensions that take a cut, can block transfers,
/// or make accounts non-transferable would break the escrow/pool accounting, so they are rejected up front.
fn check_pair_mint_extensions(mint_ai: &AccountInfo) -> Result<()> {
    if mint_ai.owner == &anchor_spl::token::ID { return Ok(()); } // classic SPL Token: no extensions
    let data = mint_ai.try_borrow_data()?;
    let st = StateWithExtensions::<spl_token_2022::state::Mint>::unpack(&data).map_err(|_| ErrorCode::BadPairMint)?;
    for ext in st.get_extension_types().map_err(|_| ErrorCode::BadPairMint)? {
        match ext {
            ExtensionType::TransferFeeConfig | ExtensionType::TransferHook | ExtensionType::NonTransferable
            | ExtensionType::ConfidentialTransferMint | ExtensionType::ConfidentialTransferFeeConfig
            | ExtensionType::ConfidentialMintBurn => return err!(ErrorCode::UnsupportedPairExtension),
            _ => {}
        }
    }
    Ok(())
}

/// transfer_checked for the pair asset — works for SPL Token and Token-2022 mints
fn pair_transfer<'info>(prog: &Interface<'info, TokenInterface>, from: &AccountInfo<'info>, to: &AccountInfo<'info>, mint: &AccountInfo<'info>,
    authority: &AccountInfo<'info>, amount: u64, decimals: u8, signer: Option<&[&[u8]]>) -> Result<()> {
    if amount == 0 { return Ok(()); }
    let accs = TransferChecked { from: from.clone(), to: to.clone(), mint: mint.clone(), authority: authority.clone() };
    match signer {
        Some(s) => ti::transfer_checked(CpiContext::new_with_signer(prog.to_account_info(), accs, &[s]), amount, decimals),
        None => ti::transfer_checked(CpiContext::new(prog.to_account_info(), accs), amount, decimals),
    }
}

fn cp_out(amount_in: u128, reserve_in: u128, reserve_out: u128) -> u64 {
    // constant product x*y=k
    let new_in = reserve_in + amount_in;
    let new_out = (reserve_in * reserve_out) / new_in;
    (reserve_out - new_out) as u64
}

// ---------------- accounts ----------------
#[account]
#[derive(InitSpace)]
pub struct Launch {
    pub creator: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub pair_mint: Pubkey,
    pub pair_token_program: Pubkey,
    pub escrow: Pubkey,
    pub pool_pair_account: Pubkey,
    pub fee_vault: Pubkey,
    #[max_len(32)] pub name: String,
    #[max_len(10)] pub symbol: String,
    pub start_ts: i64,
    pub end_ts: i64,
    pub reveal_end_ts: i64,
    pub creator_fee_bps: u16,
    pub pair_decimals: u8,
    pub min_bid: u64,
    pub bids: u32,
    pub revealed: u32,
    pub total_revealed: u64,
    pub clearing_price_q32: u128,
    pub settled: bool,
    pub pool_pair: u64,
    pub pool_tokens: u64,
    pub fees_accrued: u64,
    pub creator_fees_owed: u64,
    pub creator_fees_claimed: u64,
    pub require_human: bool,
    pub gatekeeper_network: Pubkey,
    pub stake_vault: Pubkey,
    pub total_staked: u64,
    pub reward_index: u128,      // pair base units per staked base unit, Q64
    pub rewards_pending: u64,    // holder share accrued while nobody was staked
    pub rewards_distributed: u64,
    pub settle_ts: i64,          // epoch origin for trader rewards
    pub buyback_accrued: u64,    // waiting to be spent on RSRCT
    pub buyback_total: u64,
    pub trader_accrued: u64,     // lifetime trader share accrued (per-epoch pools live in EpochRecord)
    pub trader_total: u64,       // paid out to traders
    pub treasury_owed: u64,
    pub treasury_total: u64,
    pub burned: u64,             // RSRCT launch only: tokens burned by buybacks
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Config { pub authority: Pubkey, pub treasury: Pubkey, pub rsrct_mint: Pubkey, pub bump: u8 }

#[account]
#[derive(InitSpace)]
pub struct EpochRecord { pub launch: Pubkey, pub epoch: u32, pub pool: u64, pub volume: u64, pub claimed: u64, pub swept: bool, pub bump: u8 }

#[account]
#[derive(InitSpace)]
pub struct TraderStats { pub launch: Pubkey, pub trader: Pubkey, pub epoch: u32, pub volume: u64, pub raw_volume: u64, pub claimed: bool, pub paid: u64, pub bump: u8 }

#[account]
#[derive(InitSpace)]
pub struct StakeAccount {
    pub launch: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub index: u128,
    pub owed: u64,
    pub claimed: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Bid {
    pub launch: Pubkey,
    pub bidder: Pubkey,
    pub commitment: [u8; 32],
    pub deposit: u64,
    pub amount: u64,
    pub revealed: bool,
    pub claimed: bool,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct CreateLaunch<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    /// the coin being launched (plain SPL Token, 6 decimals, fixed supply)
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    /// the asset the coin is paired with (SPL Token or Token-2022)
    pub pair_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(init, payer = creator, space = 8 + Launch::INIT_SPACE, seeds = [b"launch", mint.key().as_ref()], bump)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(init, payer = creator, token::mint = mint, token::authority = launch, token::token_program = token_program, seeds = [b"vault", mint.key().as_ref()], bump)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(init, payer = creator, token::mint = pair_mint, token::authority = launch, token::token_program = pair_token_program, seeds = [b"escrow", mint.key().as_ref()], bump)]
    pub escrow: Box<InterfaceAccount<'info, TokenAccount>>,
    /// pool reserve of the pair asset — no instruction can withdraw from it except swaps
    #[account(init, payer = creator, token::mint = pair_mint, token::authority = launch, token::token_program = pair_token_program, seeds = [b"pool", mint.key().as_ref()], bump)]
    pub pool_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(init, payer = creator, token::mint = pair_mint, token::authority = launch, token::token_program = pair_token_program, seeds = [b"fees", mint.key().as_ref()], bump)]
    pub fee_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// holders deposit launch tokens here to earn the holder share of fees
    #[account(init, payer = creator, token::mint = mint, token::authority = launch, token::token_program = token_program, seeds = [b"stake", mint.key().as_ref()], bump)]
    pub stake_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = creator_token.mint == mint.key() && creator_token.owner == creator.key())]
    pub creator_token: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub pair_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct CommitBid<'info> {
    #[account(mut)]
    pub bidder: Signer<'info>,
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Box<Account<'info, Launch>>,
    /// one bid per wallet per launch
    #[account(init, payer = bidder, space = 8 + Bid::INIT_SPACE, seeds = [b"bid", launch.key().as_ref(), bidder.key().as_ref()], bump)]
    pub bid: Box<Account<'info, Bid>>,
    #[account(address = launch.pair_mint)]
    pub pair_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = launch.escrow)]
    pub escrow: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = bidder_pair.mint == launch.pair_mint && bidder_pair.owner == bidder.key())]
    pub bidder_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = launch.pair_token_program)]
    pub pair_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    /// CHECK: Civic gateway token for the bidder; verified in the handler when the launch requires a human pass
    pub gateway_token: Option<UncheckedAccount<'info>>,
}

#[derive(Accounts)]
pub struct Stake<'info> {
    #[account(mut)]
    pub staker: Signer<'info>,
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(init_if_needed, payer = staker, space = 8 + StakeAccount::INIT_SPACE, seeds = [b"stake", launch.key().as_ref(), staker.key().as_ref()], bump)]
    pub stake: Box<Account<'info, StakeAccount>>,
    #[account(mut, address = launch.stake_vault)]
    pub stake_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = staker_token.mint == launch.mint && staker_token.owner == staker.key())]
    pub staker_token: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimRewards<'info> {
    #[account(mut)]
    pub staker: Signer<'info>,
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(mut, seeds = [b"stake", launch.key().as_ref(), staker.key().as_ref()], bump = stake.bump, constraint = stake.owner == staker.key())]
    pub stake: Box<Account<'info, StakeAccount>>,
    #[account(address = launch.pair_mint)]
    pub pair_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = launch.fee_vault)]
    pub fee_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = staker_pair.mint == launch.pair_mint && staker_pair.owner == staker.key())]
    pub staker_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = launch.pair_token_program)]
    pub pair_token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
pub struct RevealBid<'info> {
    pub bidder: Signer<'info>,
    #[account(mut)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(mut, seeds = [b"bid", launch.key().as_ref(), bidder.key().as_ref()], bump = bid.bump, has_one = bidder)]
    pub bid: Box<Account<'info, Bid>>,
}

#[derive(Accounts)]
pub struct Settle<'info> {
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(address = launch.pair_mint)]
    pub pair_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = launch.escrow)]
    pub escrow: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, address = launch.pool_pair_account)]
    pub pool_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = launch.pair_token_program)]
    pub pair_token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(mut)]
    pub bidder: Signer<'info>,
    #[account(seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(mut, seeds = [b"bid", launch.key().as_ref(), bidder.key().as_ref()], bump = bid.bump, has_one = bidder)]
    pub bid: Box<Account<'info, Bid>>,
    #[account(address = launch.pair_mint)]
    pub pair_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = launch.escrow)]
    pub escrow: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = bidder_pair.mint == launch.pair_mint && bidder_pair.owner == bidder.key())]
    pub bidder_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, address = launch.vault)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = bidder_token.mint == launch.mint && bidder_token.owner == bidder.key())]
    pub bidder_token: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    #[account(address = launch.pair_token_program)]
    pub pair_token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
#[instruction(amount: u64, min_out: u64, epoch: u32)]
pub struct Swap<'info> {
    #[account(mut)]
    pub trader: Signer<'info>,
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(address = launch.pair_mint)]
    pub pair_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = launch.pool_pair_account)]
    pub pool_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, address = launch.fee_vault)]
    pub fee_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, address = launch.vault)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = trader_token.mint == launch.mint && trader_token.owner == trader.key())]
    pub trader_token: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = trader_pair.mint == launch.pair_mint && trader_pair.owner == trader.key())]
    pub trader_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    #[account(address = launch.pair_token_program)]
    pub pair_token_program: Interface<'info, TokenInterface>,
    /// current weekly epoch (client passes epoch = floor((now - settle_ts) / 7d))
    #[account(init_if_needed, payer = trader, space = 8 + EpochRecord::INIT_SPACE, seeds = [b"epoch", launch.key().as_ref(), &epoch.to_le_bytes()], bump)]
    pub epoch_record: Box<Account<'info, EpochRecord>>,
    #[account(init_if_needed, payer = trader, space = 8 + TraderStats::INIT_SPACE, seeds = [b"trader", launch.key().as_ref(), trader.key().as_ref(), &epoch.to_le_bytes()], bump)]
    pub trader_stats: Box<Account<'info, TraderStats>>,
    pub system_program: Program<'info, System>,
    /// CHECK: optional Civic pass; if valid, this trader's volume counts toward the weekly trader pool
    pub gateway_token: Option<UncheckedAccount<'info>>,
}

#[derive(Accounts)]
pub struct InitConfig<'info> {
    #[account(mut, address = ADMIN)]
    pub admin: Signer<'info>,
    #[account(init, payer = admin, space = 8 + Config::INIT_SPACE, seeds = [b"config"], bump)]
    pub config: Account<'info, Config>,
    pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct SetConfig<'info> {
    pub authority: Signer<'info>,
    #[account(mut, seeds = [b"config"], bump = config.bump, has_one = authority)]
    pub config: Account<'info, Config>,
}
#[derive(Accounts)]
pub struct CollectTreasury<'info> {
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(address = launch.pair_mint)]
    pub pair_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = launch.fee_vault)]
    pub fee_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = treasury_pair.mint == launch.pair_mint && treasury_pair.owner == config.treasury)]
    pub treasury_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = launch.pair_token_program)]
    pub pair_token_program: Interface<'info, TokenInterface>,
}
#[derive(Accounts)]
pub struct BuybackAndBurn<'info> {
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(address = launch.pair_mint)]
    pub pair_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = launch.fee_vault)]
    pub fee_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, seeds = [b"launch", rsrct_launch.mint.as_ref()], bump = rsrct_launch.bump, constraint = rsrct_launch.key() != launch.key())]
    pub rsrct_launch: Box<Account<'info, Launch>>,
    #[account(mut, address = rsrct_launch.mint)]
    pub rsrct_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = rsrct_launch.vault)]
    pub rsrct_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, address = rsrct_launch.pool_pair_account)]
    pub rsrct_pool_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, address = rsrct_launch.fee_vault)]
    pub rsrct_fee_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    #[account(address = launch.pair_token_program)]
    pub pair_token_program: Interface<'info, TokenInterface>,
}
#[derive(Accounts)]
#[instruction(epoch: u32)]
pub struct ClaimTraderReward<'info> {
    #[account(mut)]
    pub trader: Signer<'info>,
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(mut, seeds = [b"epoch", launch.key().as_ref(), &epoch.to_le_bytes()], bump = epoch_record.bump)]
    pub epoch_record: Box<Account<'info, EpochRecord>>,
    #[account(mut, seeds = [b"trader", launch.key().as_ref(), trader.key().as_ref(), &epoch.to_le_bytes()], bump = trader_stats.bump)]
    pub trader_stats: Box<Account<'info, TraderStats>>,
    #[account(address = launch.pair_mint)]
    pub pair_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = launch.fee_vault)]
    pub fee_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = trader_pair.mint == launch.pair_mint && trader_pair.owner == trader.key())]
    pub trader_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = launch.pair_token_program)]
    pub pair_token_program: Interface<'info, TokenInterface>,
}
#[derive(Accounts)]
#[instruction(epoch: u32)]
pub struct SweepEpoch<'info> {
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(mut, seeds = [b"epoch", launch.key().as_ref(), &epoch.to_le_bytes()], bump = epoch_record.bump)]
    pub epoch_record: Box<Account<'info, EpochRecord>>,
}

#[derive(Accounts)]
pub struct ClaimCreatorFees<'info> {
    pub creator: Signer<'info>,
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump, has_one = creator)]
    pub launch: Box<Account<'info, Launch>>,
    #[account(address = launch.pair_mint)]
    pub pair_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = launch.fee_vault)]
    pub fee_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, constraint = creator_pair.mint == launch.pair_mint && creator_pair.owner == creator.key())]
    pub creator_pair: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = launch.pair_token_program)]
    pub pair_token_program: Interface<'info, TokenInterface>,
}

#[event] pub struct LaunchCreated { pub launch: Pubkey, pub mint: Pubkey, pub pair_mint: Pubkey, pub end_ts: i64 }
#[event] pub struct Settled { pub launch: Pubkey, pub clearing_price_q32: u128, pub raised: u64, pub pool_tokens: u64 }
#[event] pub struct Burned { pub launch: Pubkey, pub pair_spent: u64, pub rsrct_burned: u64 }
#[event] pub struct RewardsClaimed { pub launch: Pubkey, pub staker: Pubkey, pub amount: u64 }
#[event] pub struct Swapped { pub launch: Pubkey, pub trader: Pubkey, pub pair_in: u64, pub tokens_out: u64, pub tokens_in: u64, pub pair_out: u64 }

#[error_code]
pub enum ErrorCode {
    #[msg("bad parameters")] BadParams,
    #[msg("supply must be exactly 1,000,000,000 with 6 decimals")] BadSupply,
    #[msg("mint authority must be revoked")] MintAuthorityPresent,
    #[msg("freeze authority must be revoked")] FreezeAuthorityPresent,
    #[msg("batch is closed")] BatchClosed,
    #[msg("bid below minimum")] BidTooSmall,
    #[msg("not in reveal window")] NotRevealWindow,
    #[msg("already revealed")] AlreadyRevealed,
    #[msg("reveal does not match commitment")] BadReveal,
    #[msg("already settled")] AlreadySettled,
    #[msg("not settled yet")] NotSettled,
    #[msg("already claimed")] AlreadyClaimed,
    #[msg("slippage")] Slippage,
    #[msg("overflow")] Overflow,
    #[msg("nothing to claim")] NothingToClaim,
    #[msg("this launch requires a proof-of-personhood pass to bid")] HumanPassRequired,
    #[msg("proof-of-personhood pass is missing, revoked or for another wallet")] HumanPassInvalid,
    #[msg("proof-of-personhood pass has expired")] HumanPassExpired,
    #[msg("pair mint could not be parsed")] BadPairMint,
    #[msg("pair asset uses a Token-2022 extension that is not supported (transfer fee, transfer hook, non-transferable or confidential)")] UnsupportedPairExtension,
    #[msg("fewer tokens arrived than expected")] TransferShortfall,
    #[msg("RSRCT is not configured yet or this is not the RSRCT launch")] NoRsrct,
    #[msg("epoch account does not match the current epoch")] WrongEpoch,
    #[msg("epoch has not ended yet")] EpochOpen,
}
