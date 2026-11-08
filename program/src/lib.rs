//! Resurrect — sealed-batch launchpad (devnet MVP)
//!
//! Lifecycle of a launch:
//!   create_launch  -> creator deposits the full fixed supply into the program vault
//!   commit_bid     -> bidders escrow SOL and commit hash(amount, salt). One bid per wallet.
//!   reveal_bid     -> after the batch closes, bidders reveal (amount, salt)
//!   settle         -> one clearing price for everyone; 80% of supply goes to bidders pro‑rata,
//!                     20% of supply + ALL raised SOL seed a constant‑product pool owned by the
//!                     program. There is no withdraw instruction: liquidity cannot be pulled.
//!   claim          -> bidder receives tokens + refund of escrow above their bid
//!   swap           -> trade against the pool, 1% fee accrues to the launch fee vault
//!
//! Creator gets zero allocation and has no first buy: the creator can only bid like anyone else.
use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hashv;
use anchor_lang::system_program;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t");

pub const SUPPLY: u64 = 1_000_000_000_000_000; // 1,000,000,000 tokens with 6 decimals
pub const SALE_BPS: u64 = 8_000;               // 80% of supply sold in the batch
pub const FEE_BPS: u64 = 100;                  // 1% swap fee
pub const MIN_BATCH_SECS: i64 = 60;
pub const MAX_BATCH_SECS: i64 = 15 * 60;
pub const REVEAL_SECS: i64 = 10 * 60;
pub const MIN_BID: u64 = 10_000_000;           // 0.01 SOL

#[program]
pub mod resurrect {
    use super::*;

    pub fn create_launch(ctx: Context<CreateLaunch>, name: String, symbol: String, batch_secs: i64, creator_fee_bps: u16) -> Result<()> {
        require!(name.len() <= 32 && symbol.len() >= 1 && symbol.len() <= 10, ErrorCode::BadParams);
        require!(batch_secs >= MIN_BATCH_SECS && batch_secs <= MAX_BATCH_SECS, ErrorCode::BadParams);
        require!(creator_fee_bps <= 2_000, ErrorCode::BadParams);
        let mint = &ctx.accounts.mint;
        // fixed supply: the whole supply must already exist and authorities must be gone
        require!(mint.supply == SUPPLY, ErrorCode::BadSupply);
        require!(mint.mint_authority.is_none(), ErrorCode::MintAuthorityPresent);
        require!(mint.freeze_authority.is_none(), ErrorCode::FreezeAuthorityPresent);
        require!(mint.decimals == 6, ErrorCode::BadParams);
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
        let l = &mut ctx.accounts.launch;
        l.creator = ctx.accounts.creator.key();
        l.mint = mint.key();
        l.vault = ctx.accounts.vault.key();
        l.name = name;
        l.symbol = symbol;
        l.start_ts = now;
        l.end_ts = now + batch_secs;
        l.reveal_end_ts = l.end_ts + REVEAL_SECS;
        l.creator_fee_bps = creator_fee_bps;
        l.bids = 0;
        l.revealed = 0;
        l.total_revealed_lamports = 0;
        l.clearing_price_q32 = 0; // lamports per token * 2^32, set at settle
        l.settled = false;
        l.pool_sol = 0;
        l.pool_tokens = 0;
        l.fee_lamports = 0;
        l.bump = ctx.bumps.launch;
        emit!(LaunchCreated { launch: l.key(), mint: l.mint, end_ts: l.end_ts });
        Ok(())
    }

    /// Escrow `deposit` lamports and commit to hash(bidder, amount, salt). Deposit must cover the bid.
    pub fn commit_bid(ctx: Context<CommitBid>, commitment: [u8; 32], deposit: u64) -> Result<()> {
        let l = &mut ctx.accounts.launch;
        let now = Clock::get()?.unix_timestamp;
        require!(now < l.end_ts, ErrorCode::BatchClosed);
        require!(deposit >= MIN_BID, ErrorCode::BidTooSmall);
        system_program::transfer(
            CpiContext::new(ctx.accounts.system_program.to_account_info(), system_program::Transfer {
                from: ctx.accounts.bidder.to_account_info(),
                to: ctx.accounts.escrow.to_account_info(),
            }),
            deposit,
        )?;
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
        require!(amount >= MIN_BID && amount <= b.deposit, ErrorCode::BadReveal);
        let h = hashv(&[b.bidder.as_ref(), &amount.to_le_bytes(), &salt]);
        require!(h.to_bytes() == b.commitment, ErrorCode::BadReveal);
        b.amount = amount;
        b.revealed = true;
        l.revealed += 1;
        l.total_revealed_lamports = l.total_revealed_lamports.checked_add(amount).ok_or(ErrorCode::Overflow)?;
        Ok(())
    }

    /// Anyone can settle once the reveal window closes. One clearing price; pool is seeded and sealed.
    pub fn settle(ctx: Context<Settle>) -> Result<()> {
        let l = &mut ctx.accounts.launch;
        let now = Clock::get()?.unix_timestamp;
        require!(now >= l.reveal_end_ts, ErrorCode::NotRevealWindow);
        require!(!l.settled, ErrorCode::AlreadySettled);
        let sale_tokens = SUPPLY / 10_000 * SALE_BPS;
        let pool_tokens = SUPPLY - sale_tokens;
        if l.total_revealed_lamports == 0 {
            // nobody showed up: nothing is sold, the vault simply holds the supply; pool stays empty
            l.settled = true;
            return Ok(());
        }
        // clearing price: lamports per base unit, fixed point 2^32
        let num = (l.total_revealed_lamports as u128) << 32; let price_q32 = (num + (sale_tokens as u128) - 1) / (sale_tokens as u128); // ceil: allocations never exceed sale supply
        require!(price_q32 > 0, ErrorCode::BadParams);
        l.clearing_price_q32 = price_q32;
        // all raised SOL moves to the pool authority; 20% of supply is the pool's token side
        let raised = l.total_revealed_lamports;
        let lk = l.key();
        let eb = ctx.bumps.escrow;
        pda_pay(&ctx.accounts.system_program, &ctx.accounts.escrow.to_account_info(), &ctx.accounts.pool_sol.to_account_info(), raised, &[b"escrow", lk.as_ref(), &[eb]])?;
        let l = &mut ctx.accounts.launch;
        l.pool_sol = raised;
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
        let refund = if b.revealed && l.clearing_price_q32 > 0 { b.deposit - b.amount } else { b.deposit };
        let lk = l.key(); let eb = ctx.bumps.escrow;
        pda_pay(&ctx.accounts.system_program, &ctx.accounts.escrow.to_account_info(), &ctx.accounts.bidder.to_account_info(), refund, &[b"escrow", lk.as_ref(), &[eb]])?;
        if b.revealed && l.clearing_price_q32 > 0 {
            let tokens = (((b.amount as u128) << 32) / l.clearing_price_q32) as u64;
            let seeds: &[&[u8]] = &[b"launch", l.mint.as_ref(), &[l.bump]];
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

    /// Buy tokens with SOL from the sealed pool. 1% fee on the SOL side accrues to the launch fee vault.
    pub fn swap_sol_for_tokens(ctx: Context<Swap>, lamports_in: u64, min_tokens_out: u64) -> Result<()> {
        let l = &mut ctx.accounts.launch;
        require!(l.settled && l.pool_tokens > 0, ErrorCode::NotSettled);
        require!(lamports_in > 0, ErrorCode::BadParams);
        let fee = lamports_in * FEE_BPS / 10_000;
        let net = lamports_in - fee;
        let out = cp_out(net as u128, l.pool_sol as u128, l.pool_tokens as u128);
        require!(out >= min_tokens_out && out > 0, ErrorCode::Slippage);
        system_program::transfer(
            CpiContext::new(ctx.accounts.system_program.to_account_info(), system_program::Transfer {
                from: ctx.accounts.trader.to_account_info(), to: ctx.accounts.pool_sol.to_account_info(),
            }), net)?;
        system_program::transfer(
            CpiContext::new(ctx.accounts.system_program.to_account_info(), system_program::Transfer {
                from: ctx.accounts.trader.to_account_info(), to: ctx.accounts.fee_vault.to_account_info(),
            }), fee)?;
        let mint_key = l.mint; let bump = l.bump;
        let seeds: &[&[u8]] = &[b"launch", mint_key.as_ref(), &[bump]];
        let launch_ai = ctx.accounts.launch.to_account_info();
        token::transfer(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.vault.to_account_info(), to: ctx.accounts.trader_token.to_account_info(),
                authority: launch_ai,
            }, &[seeds]), out)?;
        let l = &mut ctx.accounts.launch;
        l.pool_sol += net;
        l.pool_tokens -= out;
        l.fee_lamports += fee;
        emit!(Swapped { launch: l.key(), trader: ctx.accounts.trader.key(), sol_in: lamports_in, tokens_out: out, tokens_in: 0, sol_out: 0 });
        Ok(())
    }

    /// Sell tokens to the pool for SOL. 1% fee on the SOL side.
    pub fn swap_tokens_for_sol(ctx: Context<Swap>, tokens_in: u64, min_lamports_out: u64) -> Result<()> {
        let l = &mut ctx.accounts.launch;
        require!(l.settled && l.pool_tokens > 0, ErrorCode::NotSettled);
        require!(tokens_in > 0, ErrorCode::BadParams);
        let gross = cp_out(tokens_in as u128, l.pool_tokens as u128, l.pool_sol as u128);
        let fee = gross * FEE_BPS / 10_000;
        let out = gross - fee;
        require!(out >= min_lamports_out && out > 0, ErrorCode::Slippage);
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.trader_token.to_account_info(), to: ctx.accounts.vault.to_account_info(),
                authority: ctx.accounts.trader.to_account_info(),
            }), tokens_in)?;
        let lk = l.key(); let pb = ctx.bumps.pool_sol;
        pda_pay(&ctx.accounts.system_program, &ctx.accounts.pool_sol.to_account_info(), &ctx.accounts.trader.to_account_info(), out, &[b"pool", lk.as_ref(), &[pb]])?;
        pda_pay(&ctx.accounts.system_program, &ctx.accounts.pool_sol.to_account_info(), &ctx.accounts.fee_vault.to_account_info(), fee, &[b"pool", lk.as_ref(), &[pb]])?;
        let l = &mut ctx.accounts.launch;
        l.pool_sol -= gross;
        l.pool_tokens += tokens_in;
        l.fee_lamports += fee;
        emit!(Swapped { launch: l.key(), trader: ctx.accounts.trader.key(), sol_in: 0, tokens_out: 0, tokens_in, sol_out: out });
        Ok(())
    }
}

fn pda_pay<'info>(sys: &Program<'info, System>, from: &AccountInfo<'info>, to: &AccountInfo<'info>, lamports: u64, seeds: &[&[u8]]) -> Result<()> {
    if lamports == 0 { return Ok(()); }
    system_program::transfer(CpiContext::new_with_signer(sys.to_account_info(), system_program::Transfer { from: from.clone(), to: to.clone() }, &[seeds]), lamports)
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
    #[max_len(32)] pub name: String,
    #[max_len(10)] pub symbol: String,
    pub start_ts: i64,
    pub end_ts: i64,
    pub reveal_end_ts: i64,
    pub creator_fee_bps: u16,
    pub bids: u32,
    pub revealed: u32,
    pub total_revealed_lamports: u64,
    pub clearing_price_q32: u128,
    pub settled: bool,
    pub pool_sol: u64,
    pub pool_tokens: u64,
    pub fee_lamports: u64,
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
    pub mint: Account<'info, Mint>,
    #[account(init, payer = creator, space = 8 + Launch::INIT_SPACE, seeds = [b"launch", mint.key().as_ref()], bump)]
    pub launch: Account<'info, Launch>,
    #[account(init, payer = creator, token::mint = mint, token::authority = launch, seeds = [b"vault", mint.key().as_ref()], bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, constraint = creator_token.mint == mint.key() && creator_token.owner == creator.key())]
    pub creator_token: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct CommitBid<'info> {
    #[account(mut)]
    pub bidder: Signer<'info>,
    #[account(mut)]
    pub launch: Account<'info, Launch>,
    /// one bid per wallet per launch
    #[account(init, payer = bidder, space = 8 + Bid::INIT_SPACE, seeds = [b"bid", launch.key().as_ref(), bidder.key().as_ref()], bump)]
    pub bid: Account<'info, Bid>,
    /// CHECK: PDA that only holds lamports
    #[account(mut, seeds = [b"escrow", launch.key().as_ref()], bump)]
    pub escrow: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealBid<'info> {
    pub bidder: Signer<'info>,
    #[account(mut)]
    pub launch: Account<'info, Launch>,
    #[account(mut, seeds = [b"bid", launch.key().as_ref(), bidder.key().as_ref()], bump = bid.bump, has_one = bidder)]
    pub bid: Account<'info, Bid>,
}

#[derive(Accounts)]
pub struct Settle<'info> {
    #[account(mut)]
    pub launch: Account<'info, Launch>,
    /// CHECK: lamport escrow PDA
    #[account(mut, seeds = [b"escrow", launch.key().as_ref()], bump)]
    pub escrow: UncheckedAccount<'info>,
    /// CHECK: lamport pool PDA — no instruction can withdraw from it except swaps
    #[account(mut, seeds = [b"pool", launch.key().as_ref()], bump)]
    pub pool_sol: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(mut)]
    pub bidder: Signer<'info>,
    #[account(seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Account<'info, Launch>,
    #[account(mut, seeds = [b"bid", launch.key().as_ref(), bidder.key().as_ref()], bump = bid.bump, has_one = bidder)]
    pub bid: Account<'info, Bid>,
    /// CHECK: lamport escrow PDA
    #[account(mut, seeds = [b"escrow", launch.key().as_ref()], bump)]
    pub escrow: UncheckedAccount<'info>,
    #[account(mut, address = launch.vault)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, constraint = bidder_token.mint == launch.mint && bidder_token.owner == bidder.key())]
    pub bidder_token: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Swap<'info> {
    #[account(mut)]
    pub trader: Signer<'info>,
    #[account(mut, seeds = [b"launch", launch.mint.as_ref()], bump = launch.bump)]
    pub launch: Account<'info, Launch>,
    /// CHECK: lamport pool PDA
    #[account(mut, seeds = [b"pool", launch.key().as_ref()], bump)]
    pub pool_sol: UncheckedAccount<'info>,
    /// CHECK: lamport fee vault PDA
    #[account(mut, seeds = [b"fees", launch.key().as_ref()], bump)]
    pub fee_vault: UncheckedAccount<'info>,
    #[account(mut, address = launch.vault)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, constraint = trader_token.mint == launch.mint && trader_token.owner == trader.key())]
    pub trader_token: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[event] pub struct LaunchCreated { pub launch: Pubkey, pub mint: Pubkey, pub end_ts: i64 }
#[event] pub struct Settled { pub launch: Pubkey, pub clearing_price_q32: u128, pub raised: u64, pub pool_tokens: u64 }
#[event] pub struct Swapped { pub launch: Pubkey, pub trader: Pubkey, pub sol_in: u64, pub tokens_out: u64, pub tokens_in: u64, pub sol_out: u64 }

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
}
