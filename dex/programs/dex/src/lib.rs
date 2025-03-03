#![allow(unexpected_cfgs)]

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint,Token,TokenAccount, Transfer};
use anchor_spl::associated_token::AssociatedToken;


declare_id!("Ab53L5iKwJmLUGRyEX9z9tLygPHEyLDzEdjq9qk3CAB1"); // Replace with your program ID

#[program]
pub mod dex {
    use super::*;

    pub fn initialize_factory(ctx: Context<InitializeFactory>) -> Result<()> {
        let factory = &mut ctx.accounts.factory;
        factory.authority = ctx.accounts.authority.key();
        factory.pair_count = 0;
        factory.fee_to = Pubkey::default(); // No fee recipient initially
        factory.fee_on = false;
        
        Ok(())
    }

    pub fn create_pair(
        ctx: Context<CreatePair>, 
        bump_seed: u8,
        lp_bump: u8,
    ) -> Result<()> {
        msg!("Creating pair with bumps: {}, {}", bump_seed, lp_bump);
        msg!("Token A: {}", ctx.accounts.token_a_mint.key());
        msg!("Token B: {}", ctx.accounts.token_b_mint.key());
        // Verify tokens are different
        require!(
            ctx.accounts.token_a_mint.key() != ctx.accounts.token_b_mint.key(),
            PairError::IdenticalTokens
        );
        
        // No need to re-sort since we're enforcing the correct order
    let token_0 = ctx.accounts.token_a_mint.key();
    let token_1 = ctx.accounts.token_b_mint.key();
        
        // Get factory key before mutable borrow
        let factory_key = ctx.accounts.factory.key();
        
        // Initialize pair state
        let pair = &mut ctx.accounts.pair;
        pair.factory = factory_key;
        pair.token_0 = token_0;
        pair.token_1 = token_1;
        pair.reserve_0 = 0;
        pair.reserve_1 = 0;
        pair.k_last = 0;
        pair.block_timestamp_last = 0;
        pair.price_0_cumulative_last = 0;
        pair.price_1_cumulative_last = 0;
        pair.bump = bump_seed;
        pair.lp_bump = lp_bump;
        
        // Now do the mutable borrow of factory
        let factory = &mut ctx.accounts.factory;
        factory.pair_count += 1;
        let pair_count = factory.pair_count;
        
        // Setup LP token mint with initial minimal supply
        const MINIMUM_LIQUIDITY: u64 = 1000;

        msg!("About to mint LP tokens");

        
        // Use the pair PDA as the mint authority
    let seeds = &[
        b"pair".as_ref(),
        token_0.as_ref(),
        token_1.as_ref(),
        &[bump_seed],
    ];
        let signer = &[&seeds[..]];

        msg!("Pair PDA: {}", ctx.accounts.pair.key());
        msg!("LP Token Mint: {}", ctx.accounts.lp_token_mint.key());
        
        let cpi_accounts = token::MintTo {
            mint: ctx.accounts.lp_token_mint.to_account_info(),
            to: ctx.accounts.lp_token_vault.to_account_info(),
            authority: ctx.accounts.pair.to_account_info(),
        };
        
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer,
        );
        
        token::mint_to(cpi_ctx, MINIMUM_LIQUIDITY)?;
        
        emit!(PairCreated {
            factory: factory_key,
            token_0,
            token_1,
            pair: ctx.accounts.pair.key(),
            pair_count,
        });
        
        Ok(())
    }

    pub fn add_liquidity(
        ctx: Context<AddLiquidity>,
        amount_a_desired: u64,
        amount_b_desired: u64,
        amount_a_min: u64,
        amount_b_min: u64,
        deadline: i64,
    ) -> Result<()> {
        // Check deadline hasn't passed
        require!(
            Clock::get()?.unix_timestamp <= deadline,
            PairError::Expired
        );
        
        // Store token keys for later use
        let token_0_key = ctx.accounts.token_0_mint.key();
        let token_1_key = ctx.accounts.token_1_mint.key();
        
        // First get these values before the mutable borrow
        let reserve_0 = ctx.accounts.pair.reserve_0;
        let reserve_1 = ctx.accounts.pair.reserve_1;
        let bump = ctx.accounts.pair.bump;
        
        // Calculate optimal amounts
        let (amount_0, amount_1) = if reserve_0 == 0 && reserve_1 == 0 {
            // Initial liquidity - accept exact desired amounts
            (amount_a_desired, amount_b_desired)
        } else {
            // Existing liquidity - calculate optimal amounts
            let amount_b_optimal = quote(
                amount_a_desired,
                reserve_0,
                reserve_1,
            )?;
            
            if amount_b_optimal <= amount_b_desired {
                require!(
                    amount_b_optimal >= amount_b_min,
                    PairError::InsufficientBAmount
                );
                (amount_a_desired, amount_b_optimal)
            } else {
                let amount_a_optimal = quote(
                    amount_b_desired,
                    reserve_1,
                    reserve_0,
                )?;
                require!(
                    amount_a_optimal <= amount_a_desired,
                    PairError::InsufficientAAmount
                );
                require!(
                    amount_a_optimal >= amount_a_min,
                    PairError::InsufficientAAmount
                );
                (amount_a_optimal, amount_b_desired)
            }
        };
        
        // Transfer tokens to pair vaults
        // Token 0 transfer
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.user_token_0.to_account_info(),
                    to: ctx.accounts.token_0_vault.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount_0,
        )?;
        
        // Token 1 transfer
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.user_token_1.to_account_info(),
                    to: ctx.accounts.token_1_vault.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount_1,
        )?;
        
        // Calculate liquidity amount
        let liquidity = if reserve_0 == 0 && reserve_1 == 0 {
            // Initial liquidity - sqrt(amount_0 * amount_1) - MINIMUM_LIQUIDITY
            let liquidity_full = (amount_0 as u128)
                .checked_mul(amount_1 as u128)
                .ok_or(PairError::Overflow)?;
            let liquidity = integer_sqrt(liquidity_full)
                .checked_sub(1000) // MINIMUM_LIQUIDITY
                .ok_or(PairError::InsufficientLiquidityMinted)?;
            liquidity as u64
        } else {
            // Get LP token supply before mint
            let lp_supply = ctx.accounts.lp_token_mint.supply;
            
            // Calculate liquidity based on existing reserves
            let liquidity_0 = (amount_0 as u128)
                .checked_mul(lp_supply as u128)
                .ok_or(PairError::Overflow)?
                .checked_div(reserve_0 as u128)
                .ok_or(PairError::Overflow)?;
                
            let liquidity_1 = (amount_1 as u128)
                .checked_mul(lp_supply as u128)
                .ok_or(PairError::Overflow)?
                .checked_div(reserve_1 as u128)
                .ok_or(PairError::Overflow)?;
                
            // Use the smaller amount
            let liquidity = std::cmp::min(liquidity_0, liquidity_1) as u64;
            require!(
                liquidity > 0,
                PairError::InsufficientLiquidityMinted
            );
            liquidity
        };
        
        // Create seeds for PDA signer - using stored keys
        let token_0_key_ref = token_0_key.as_ref();
        let token_1_key_ref = token_1_key.as_ref();
        
        let seeds = &[
            b"pair".as_ref(),
            token_0_key_ref,
            token_1_key_ref,
            &[bump],
        ];
        let signer = &[&seeds[..]];
        
        // Mint LP tokens to user
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::MintTo {
                    mint: ctx.accounts.lp_token_mint.to_account_info(),
                    to: ctx.accounts.user_lp_token.to_account_info(),
                    authority: ctx.accounts.pair.to_account_info(),
                },
                signer,
            ),
            liquidity,
        )?;
        
        // Now get mutable access to pair for updates
        let pair = &mut ctx.accounts.pair;
        
        // Update reserves and timestamp
        update_reserves(
            pair,
            ctx.accounts.token_0_vault.amount,
            ctx.accounts.token_1_vault.amount,
        )?;
        
        // Update kLast for protocol fee calculation
        if ctx.accounts.factory.fee_on {
            pair.k_last = (pair.reserve_0 as u128)
                .checked_mul(pair.reserve_1 as u128)
                .ok_or(PairError::Overflow)? as u64;
        }
        
        emit!(LiquidityAdded {
            sender: ctx.accounts.user.key(),
            amount_0,
            amount_1,
            liquidity,
        });
        
        Ok(())
    }

    // Additional functions (swap, remove_liquidity, etc.) would be implemented here
}

// Helper functions
fn quote(amount_a: u64, reserve_a: u64, reserve_b: u64) -> Result<u64> {
    require!(amount_a > 0, PairError::InsufficientAmount);
    require!(reserve_a > 0 && reserve_b > 0, PairError::InsufficientLiquidity);
    
    let amount_b = (amount_a as u128)
        .checked_mul(reserve_b as u128)
        .ok_or(PairError::Overflow)?
        .checked_div(reserve_a as u128)
        .ok_or(PairError::Overflow)? as u64;
        
    Ok(amount_b)
}

fn update_reserves(pair: &mut Account<Pair>, balance_0: u64, balance_1: u64) -> Result<()> {
    require!(
        balance_0 <= u64::MAX && balance_1 <= u64::MAX,
        PairError::Overflow
    );
    
    // Get current timestamp
    let block_timestamp = Clock::get()?.unix_timestamp as u64;
    let time_elapsed = block_timestamp.saturating_sub(pair.block_timestamp_last);
    
    // Update price accumulators if time has passed and reserves exist
    if time_elapsed > 0 && pair.reserve_0 > 0 && pair.reserve_1 > 0 {
        // Calculate and update cumulative prices
        // Note: In a real implementation, we would need fixed-point arithmetic here
        pair.price_0_cumulative_last += (pair.reserve_1 as u128)
            .checked_mul(time_elapsed as u128)
            .ok_or(PairError::Overflow)?
            .checked_div(pair.reserve_0 as u128)
            .ok_or(PairError::Overflow)? as u64;
            
        pair.price_1_cumulative_last += (pair.reserve_0 as u128)
            .checked_mul(time_elapsed as u128)
            .ok_or(PairError::Overflow)?
            .checked_div(pair.reserve_1 as u128)
            .ok_or(PairError::Overflow)? as u64;
    }
    
    // Update reserves and timestamp
    pair.reserve_0 = balance_0;
    pair.reserve_1 = balance_1;
    pair.block_timestamp_last = block_timestamp;
    
    Ok(())
}

// Babylonian method for integer square root
fn integer_sqrt(value: u128) -> u128 {
    if value == 0 {
        return 0;
    }
    
    let mut x = value / 2 + 1;
    let mut y = (x + value / x) / 2;
    
    while y < x {
        x = y;
        y = (x + value / x) / 2;
    }
    
    x
}

// Account structures
#[account]
pub struct Factory {
    pub authority: Pubkey,      // Admin authority
    pub pair_count: u64,        // Number of pairs created
    pub fee_to: Pubkey,         // Protocol fee recipient
    pub fee_on: bool,           // Whether protocol fees are enabled
}

#[account]
pub struct Pair {
    pub factory: Pubkey,        // Factory that created this pair
    pub token_0: Pubkey,        // First token mint (sorted)
    pub token_1: Pubkey,        // Second token mint (sorted)
    pub reserve_0: u64,         // Reserve amount of token_0
    pub reserve_1: u64,         // Reserve amount of token_1
    pub block_timestamp_last: u64, // Last block timestamp for oracle
    pub price_0_cumulative_last: u64, // Cumulative price of token_0 in terms of token_1
    pub price_1_cumulative_last: u64, // Cumulative price of token_1 in terms of token_0
    pub k_last: u64,            // Last k value (reserve_0 * reserve_1) for fee calculation
    pub bump: u8,               // PDA bump seed for pair
    pub lp_bump: u8,            // PDA bump seed for LP token mint
}

// Context structures for instructions
#[derive(Accounts)]
pub struct InitializeFactory<'info> {
    #[account(init, payer = authority, space = 8 + 32 + 8 + 32 + 1)]
    pub factory: Account<'info, Factory>,
    
    #[account(mut)]
    pub authority: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(bump_seed: u8, lp_bump: u8)]
pub struct CreatePair<'info> {
    #[account(mut)]
    pub factory: Account<'info, Factory>,
    
    #[account(
        init,
        payer = payer,
        space = 8 + 32 + 32 + 32 + 8 + 8 + 8 + 8 + 8 + 8 + 1 + 1,
        seeds = [
            b"pair".as_ref(),
            token_a_mint.key().as_ref(),
            token_b_mint.key().as_ref(),
        ],
        bump,
    )]
    pub pair: Account<'info, Pair>,
    
    pub token_a_mint: Account<'info, Mint>,
    pub token_b_mint: Account<'info, Mint>,
    
    #[account(
        init,
        payer = payer,
        seeds = [
            b"lp_token".as_ref(),
            token_a_mint.key().as_ref(),
            token_b_mint.key().as_ref(),
        ],
        bump,
        mint::decimals = 6,
        mint::authority = pair,
    )]
    pub lp_token_mint: Account<'info, Mint>,
    
    #[account(
        init,
        payer = payer,
        associated_token::mint = lp_token_mint,
        associated_token::authority = pair,
    )]
    pub lp_token_vault: Account<'info, TokenAccount>,
    
    #[account(
        init,
        payer = payer,
        associated_token::mint = token_a_mint,
        associated_token::authority = pair,
    )]
    pub token_a_vault: Account<'info, TokenAccount>,
    
    #[account(
        init,
        payer = payer,
        associated_token::mint = token_b_mint,
        associated_token::authority = pair,
    )]
    pub token_b_vault: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub payer: Signer<'info>,
    
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct AddLiquidity<'info> {
    pub factory: Account<'info, Factory>,
    
    #[account(
        mut,
        seeds = [
            b"pair".as_ref(),
            pair.token_0.as_ref(),
            pair.token_1.as_ref(),
        ],
        bump = pair.bump,
    )]
    pub pair: Account<'info, Pair>,
    
    #[account(mut, address = pair.token_0)]
    pub token_0_mint: Account<'info, Mint>,
    
    #[account(mut, address = pair.token_1)]
    pub token_1_mint: Account<'info, Mint>,
    
    #[account(
        mut,
        seeds = [
            b"lp_token".as_ref(),
            pair.key().as_ref(),
        ],
        bump = pair.lp_bump,
        mint::authority = pair,
    )]
    pub lp_token_mint: Account<'info, Mint>,
    
    #[account(
        mut,
        associated_token::mint = token_0_mint,
        associated_token::authority = pair,
    )]
    pub token_0_vault: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        associated_token::mint = token_1_mint,
        associated_token::authority = pair,
    )]
    pub token_1_vault: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub user: Signer<'info>,
    
    #[account(
        mut,
        constraint = user_token_0.mint == token_0_mint.key(),
        constraint = user_token_0.owner == user.key(),
    )]
    pub user_token_0: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        constraint = user_token_1.mint == token_1_mint.key(),
        constraint = user_token_1.owner == user.key(),
    )]
    pub user_token_1: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        constraint = user_lp_token.mint == lp_token_mint.key(),
        constraint = user_lp_token.owner == user.key(),
    )]
    pub user_lp_token: Account<'info, TokenAccount>,
    
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

// Events
#[event]
pub struct PairCreated {
    pub factory: Pubkey,
    pub token_0: Pubkey,
    pub token_1: Pubkey,
    pub pair: Pubkey,
    pub pair_count: u64,
}

#[event]
pub struct LiquidityAdded {
    pub sender: Pubkey,
    pub amount_0: u64,
    pub amount_1: u64,
    pub liquidity: u64,
}

// Error codes
#[error_code]
pub enum PairError {
    #[msg("Tokens must be different")]
    IdenticalTokens,

    #[msg("Tokens must be provided in sorted order")]
    TokensNotSorted,
    
    #[msg("Insufficient amount")]
    InsufficientAmount,
    
    #[msg("Insufficient liquidity")]
    InsufficientLiquidity,
    
    #[msg("Insufficient token A amount")]
    InsufficientAAmount,
    
    #[msg("Insufficient token B amount")]
    InsufficientBAmount,
    
    #[msg("Insufficient liquidity minted")]
    InsufficientLiquidityMinted,
    
    #[msg("Insufficient liquidity burned")]
    InsufficientLiquidityBurned,
    
    #[msg("Insufficient output amount")]
    InsufficientOutputAmount,
    
    #[msg("Insufficient input amount")]
    InsufficientInputAmount,
    
    #[msg("K value error")]
    KError,
    
    #[msg("Transaction expired")]
    Expired,
    
    #[msg("Overflow")]
    Overflow,
}