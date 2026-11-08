//! Resurrect — sealed-batch launchpad (devnet v2: any-asset pairing)
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
//!   claim_creator_fees -> creator pulls their share of accrued fees; the rest stays for holders
use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hashv;
use anchor_spl::token::{self, Token, Transfer};
use anchor_spl::token_interface::{self as ti, Mint, TokenAccount, TokenInterface, TransferChecked};

declare_id!("5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t");

pub const SUPPLY: u64 = 1_000_000_000_000_000; // 1,000,000,000 tokens with 6 decimals
pub const SALE_BPS: u64 = 8_000;               // 80% of supply sold in the batch
pub const FEE_BPS: u64 = 100;                  // 1% swap fee
pub const MIN_BATCH_SECS: i64 = 60;
pub const MAX_BATCH_SECS: i64 = 15 * 60;
pub const REVEAL_SECS: i64 = 10 * 60;
pub const MAX_CREATOR_FEE_BPS: u16 = 2_000;    // creator may take at most 20% of fees

#[program]
pub mod resurrect {
    use super::*;

    pub fn create_launch(ctx: Context<CreateLaunch>, name: String, symbol: String, batch_secs: i64, creator_fee_bps: u16) -> Result<()> {
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
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.bidder_pair.to_account_info(), &ctx.accounts.escrow.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.bidder.to_account_info(), deposit, l.pair_decimals, None)?;
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
    pub fn swap_pair_for_tokens(ctx: Context<Swap>, pair_in: u64, min_tokens_out: u64) -> Result<()> {
        let l = &ctx.accounts.launch;
        require!(l.settled && l.pool_tokens > 0, ErrorCode::NotSettled);
        require!(pair_in > 0, ErrorCode::BadParams);
        let fee = pair_in * FEE_BPS / 10_000;
        let net = pair_in - fee;
        let out = cp_out(net as u128, l.pool_pair as u128, l.pool_tokens as u128);
        require!(out >= min_tokens_out && out > 0, ErrorCode::Slippage);
        let dec = l.pair_decimals;
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.trader_pair.to_account_info(), &ctx.accounts.pool_pair.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.trader.to_account_info(), net, dec, None)?;
        pair_transfer(&ctx.accounts.pair_token_program, &ctx.accounts.trader_pair.to_account_info(), &ctx.accounts.fee_vault.to_account_info(),
            &ctx.accounts.pair_mint.to_account_info(), &ctx.accounts.trader.to_account_info(), fee, dec, None)?;
        let seeds: &[&[u8]] = &[b"launch", l.mint.as_ref(), &[l.bump]];
        token::transfer(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.vault.to_account_info(), to: ctx.accounts.trader_token.to_account_info(),
                authority: ctx.accounts.launch.to_account_info(),
            }, &[seeds]), out)?;
        let l = &mut ctx.accounts.launch;
        l.pool_pair += net;
        l.pool_tokens -= out;
        accrue_fee(l, fee)?;
        emit!(Swapped { launch: l.key(), trader: ctx.accounts.trader.key(), pair_in, tokens_out: out, tokens_in: 0, pair_out: 0 });
        Ok(())
    }

    /// Sell tokens to the pool for the pair asset. 1% fee on the pair side.
    pub fn swap_tokens_for_pair(ctx: Context<Swap>, tokens_in: u64, min_pair_out: u64) -> Result<()> {
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
        let l = &mut ctx.accounts.launch;
        l.pool_pair -= gross;
        l.pool_tokens += tokens_in;
        accrue_fee(l, fee)?;
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
}

fn accrue_fee(l: &mut Launch, fee: u64) -> Result<()> {
    l.fees_accrued = l.fees_accrued.checked_add(fee).ok_or(ErrorCode::Overflow)?;
    let creator_cut = (fee as u128 * l.creator_fee_bps as u128 / 10_000) as u64;
    l.creator_fees_owed = l.creator_fees_owed.checked_add(creator_cut).ok_or(ErrorCode::Overflow)?;
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
}
