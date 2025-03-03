import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Dex } from "../target/types/dex";
import { PublicKey, Keypair, SystemProgram, Connection, clusterApiUrl } from "@solana/web3.js";
import { 
  TOKEN_PROGRAM_ID, 
  ASSOCIATED_TOKEN_PROGRAM_ID, 
  getAssociatedTokenAddress,
  createInitializeMintInstruction,
  createAssociatedTokenAccountInstruction,
  createMintToInstruction,
  MINT_SIZE
} from "@solana/spl-token";
import { assert } from "chai";

describe("Dex on Devnet", () => {
  // Configure the client to use the devnet cluster
  const connection = new Connection(clusterApiUrl("devnet"), "confirmed");
  const wallet = anchor.Wallet.local();
  const provider = new anchor.AnchorProvider(
    connection, 
    wallet, 
    { commitment: "confirmed" }
  );
  anchor.setProvider(provider);

  const program = anchor.workspace.Dex as Program<Dex>;

  // Store accounts for later use
  let factoryKeypair: Keypair;
  let token0Keypair: Keypair;
  let token1Keypair: Keypair;
  let pairPDA: PublicKey;
  let lpTokenMintPDA: PublicKey;
  let pairBump: number;
  let lpBump: number;

  // Log wallet balance before tests
  before(async () => {
    const balance = await connection.getBalance(wallet.publicKey);
    console.log(`Using wallet: ${wallet.publicKey.toString()}`);
    console.log(`Wallet balance: ${balance / anchor.web3.LAMPORTS_PER_SOL} SOL`);
    
    // Check if there's enough SOL for testing
    if (balance < anchor.web3.LAMPORTS_PER_SOL) {
      console.warn("Warning: Wallet balance may be too low for testing. Consider adding more SOL.");
    }
  });

  // Helper function to create token mints
  async function createMint(decimals: number = 6): Promise<Keypair> {
    const mintKeypair = Keypair.generate();
    const lamportsForMint = await provider.connection.getMinimumBalanceForRentExemption(
      MINT_SIZE
    );

    const createMintAccountIx = SystemProgram.createAccount({
      fromPubkey: wallet.publicKey,
      newAccountPubkey: mintKeypair.publicKey,
      lamports: lamportsForMint,
      space: MINT_SIZE,
      programId: TOKEN_PROGRAM_ID,
    });

    const initMintIx = createInitializeMintInstruction(
      mintKeypair.publicKey,
      decimals,
      wallet.publicKey,
      null
    );

    const tx = new anchor.web3.Transaction().add(createMintAccountIx, initMintIx);
    await provider.sendAndConfirm(tx, [mintKeypair]);
    
    return mintKeypair;
  }

  // Helper function to create token accounts
  async function createTokenAccount(
    mint: PublicKey, 
    owner: PublicKey
  ): Promise<PublicKey> {
    const tokenAccount = await getAssociatedTokenAddress(
      mint,
      owner
    );

    const tx = new anchor.web3.Transaction().add(
      createAssociatedTokenAccountInstruction(
        wallet.publicKey,
        tokenAccount,
        owner,
        mint
      )
    );

    await provider.sendAndConfirm(tx, []);
    return tokenAccount;
  }

  // Helper function to mint tokens to an account
  async function mintTokens(
    mint: PublicKey,
    destination: PublicKey,
    amount: number
  ): Promise<void> {
    const tx = new anchor.web3.Transaction().add(
      createMintToInstruction(
        mint,
        destination,
        wallet.publicKey,
        amount
      )
    );

    await provider.sendAndConfirm(tx, []);
  }

  // Test for initialize_factory function
  it("Initialize Factory", async () => {
    // Create a keypair for the factory
    factoryKeypair = Keypair.generate();

    // Call initialize_factory function
    const tx = await program.methods
      .initializeFactory()
      .accounts({
        factory: factoryKeypair.publicKey,
        authority: wallet.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([factoryKeypair])
      .rpc();

    console.log("Initialize Factory transaction signature", tx);

    // Fetch the factory account to verify initialization
    const factoryAccount = await program.account.factory.fetch(factoryKeypair.publicKey);
    
    // Verify factory state
    assert.isTrue(factoryAccount.authority.equals(wallet.publicKey), "Factory authority should be wallet");
    assert.equal(factoryAccount.pairCount.toNumber(), 0, "Initial pair count should be 0");
    assert.isTrue(factoryAccount.feeTo.equals(PublicKey.default), "Fee to should be default");
    assert.equal(factoryAccount.feeOn, false, "Fee on should be false");
  });

  // Test for create_pair function
  it("Create Pair", async () => {
    // Create two token mints for the pair
    token0Keypair = await createMint();
    token1Keypair = await createMint();
    
    console.log("Token0 mint created:", token0Keypair.publicKey.toString());
    console.log("Token1 mint created:", token1Keypair.publicKey.toString());
    
    // Sort token mints to match contract logic
  let tokenA, tokenB;
  if (token0Keypair.publicKey.toBuffer().compare(token1Keypair.publicKey.toBuffer()) < 0) {
    tokenA = token0Keypair.publicKey;
    tokenB = token1Keypair.publicKey;
  } else {
    tokenA = token1Keypair.publicKey;
    tokenB = token0Keypair.publicKey;
  }

console.log("TokenA:", tokenA.toString());
console.log("TokenB:", tokenB.toString());
    
    console.log("TokenA (sorted):", tokenA.toString());
    console.log("TokenB (sorted):", tokenB.toString());

    // Find pair PDA
    const [pairAddress, bump] = await PublicKey.findProgramAddress(
      [
        Buffer.from("pair"),
        tokenA.toBuffer(),
        tokenB.toBuffer(),
      ],
      program.programId
    );
    pairPDA = pairAddress;
    pairBump = bump;
    
    console.log("Pair PDA:", pairPDA.toString(), "with bump:", pairBump);

    // Find LP token mint PDA using the same tokens
const [lpTokenMint, lpBumpSeed] = await PublicKey.findProgramAddress(
  [
    Buffer.from("lp_token"),
    tokenA.toBuffer(),
    tokenB.toBuffer(),
  ],
  program.programId
);
    lpTokenMintPDA = lpTokenMint;
    lpBump = lpBumpSeed;
    
    console.log("LP Token Mint PDA:", lpTokenMintPDA.toString(), "with bump:", lpBump);

    try {
      // Create LP token vault
      const lpTokenVault = await getAssociatedTokenAddress(
        lpTokenMintPDA,
        pairPDA,
        true // allowOwnerOffCurve = true for PDA
      );
      
      console.log("LP Token Vault:", lpTokenVault.toString());

      // Create token vaults
      const tokenAVault = await getAssociatedTokenAddress(
        tokenA,
        pairPDA,
        true // allowOwnerOffCurve = true for PDA
      );
      
      console.log("Token A Vault:", tokenAVault.toString());

      const tokenBVault = await getAssociatedTokenAddress(
        tokenB,
        pairPDA,
        true // allowOwnerOffCurve = true for PDA
      );
      
      console.log("Token B Vault:", tokenBVault.toString());

      // Log all accounts that will be passed to create_pair
      console.log("Accounts for create_pair:");
      console.log("- Factory:", factoryKeypair.publicKey.toString());
      console.log("- Pair:", pairPDA.toString());
      console.log("- Token A Mint:", tokenA.toString());
      console.log("- Token B Mint:", tokenB.toString());
      console.log("- LP Token Mint:", lpTokenMintPDA.toString());
      console.log("- LP Token Vault:", lpTokenVault.toString());
      console.log("- Token A Vault:", tokenAVault.toString());
      console.log("- Token B Vault:", tokenBVault.toString());
      console.log("- Payer:", wallet.publicKey.toString());
      console.log("- Token Program:", TOKEN_PROGRAM_ID.toString());
      console.log("- Associated Token Program:", ASSOCIATED_TOKEN_PROGRAM_ID.toString());
      console.log("- System Program:", SystemProgram.programId.toString());
      console.log("- Rent Sysvar:", anchor.web3.SYSVAR_RENT_PUBKEY.toString());
      
      // Call create_pair function - with additional error handling
      const tx = await program.methods
        .createPair(
          pairBump,
          lpBump
        )
        .accounts({
          factory: factoryKeypair.publicKey,
          pair: pairPDA,
          tokenAMint: tokenA,
          tokenBMint: tokenB,
          lpTokenMint: lpTokenMintPDA,
          lpTokenVault: lpTokenVault,
          tokenAVault: tokenAVault,
          tokenBVault: tokenBVault,
          payer: wallet.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: anchor.web3.SYSVAR_RENT_PUBKEY,
        })
        .rpc();

      console.log("Create Pair transaction signature", tx);

      // Fetch accounts to verify creation
      const pairAccount = await program.account.pair.fetch(pairPDA);
      const factoryAccount = await program.account.factory.fetch(factoryKeypair.publicKey);
      
      // Verify pair state
      assert.isTrue(pairAccount.factory.equals(factoryKeypair.publicKey), "Pair factory should match");
      assert.equal(pairAccount.bump, pairBump, "Pair bump should match");
      assert.equal(pairAccount.lpBump, lpBump, "LP bump should match");
      assert.equal(pairAccount.reserve0.toNumber(), 0, "Initial reserve0 should be 0");
      assert.equal(pairAccount.reserve1.toNumber(), 0, "Initial reserve1 should be 0");
      
      // Verify factory state was updated
      assert.equal(factoryAccount.pairCount.toNumber(), 1, "Pair count should be incremented");
    } catch (error) {
      console.error("Detailed error:", error);
      // Rethrow to fail the test
      throw error;
    }
  });

  // // Test for add_liquidity function
  // it("Add Liquidity", async () => {
  //   // Create user token accounts
  //   const userToken0 = await createTokenAccount(token0Keypair.publicKey, wallet.publicKey);
  //   const userToken1 = await createTokenAccount(token1Keypair.publicKey, wallet.publicKey);
    
  //   // Mint tokens to user
  //   const amount0 = 1_000_000; // 1 token with 6 decimals
  //   const amount1 = 2_000_000; // 2 tokens with 6 decimals
  //   await mintTokens(token0Keypair.publicKey, userToken0, amount0);
  //   await mintTokens(token1Keypair.publicKey, userToken1, amount1);
    
  //   // Create user LP token account
  //   const userLpToken = await createTokenAccount(lpTokenMintPDA, wallet.publicKey);
    
  //   // Find token vaults - need to ensure these match the sorted order
  //   let [token0, token1] = [token0Keypair.publicKey, token1Keypair.publicKey].sort((a, b) => 
  //     a.toBuffer().compare(b.toBuffer())
  //   );
    
  //   const token0Vault = await Token.getAssociatedTokenAddress(
  //     ASSOCIATED_TOKEN_PROGRAM_ID,
  //     TOKEN_PROGRAM_ID,
  //     token0,
  //     pairPDA,
  //     true
  //   );
    
  //   const token1Vault = await Token.getAssociatedTokenAddress(
  //     ASSOCIATED_TOKEN_PROGRAM_ID,
  //     TOKEN_PROGRAM_ID,
  //     token1,
  //     pairPDA,
  //     true
  //   );
    
  //   // Set deadline 1 hour from now
  //   const currentTime = Math.floor(Date.now() / 1000);
  //   const deadline = currentTime + 3600;
    
  //   // Call add_liquidity function
  //   const tx = await program.methods
  //     .addLiquidity(
  //       new anchor.BN(amount0),
  //       new anchor.BN(amount1),
  //       new anchor.BN(amount0), // min amount same as desired for test
  //       new anchor.BN(amount1), // min amount same as desired for test
  //       new anchor.BN(deadline)
  //     )
  //     .accounts({
  //       factory: factoryKeypair.publicKey,
  //       pair: pairPDA,
  //       token0Mint: token0,
  //       token1Mint: token1,
  //       lpTokenMint: lpTokenMintPDA,
  //       token0Vault: token0Vault,
  //       token1Vault: token1Vault,
  //       user: wallet.publicKey,
  //       userToken0: userToken0,
  //       userToken1: userToken1,
  //       userLpToken: userLpToken,
  //       tokenProgram: TOKEN_PROGRAM_ID,
  //       associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
  //       systemProgram: SystemProgram.programId,
  //     })
  //     .rpc();
    
  //   console.log("Add Liquidity transaction signature", tx);
    
  //   // Fetch accounts to verify liquidity addition
  //   const pairAccount = await program.account.pair.fetch(pairPDA);
    
  //   // Get LP token balance
  //   const userLpBalance = await provider.connection.getTokenAccountBalance(userLpToken);
  //   console.log("User LP balance:", userLpBalance.value.uiAmount);
    
  //   // Verify reserves were updated
  //   assert.equal(pairAccount.reserve0.toNumber(), amount0, "Reserve0 should match deposit");
  //   assert.equal(pairAccount.reserve1.toNumber(), amount1, "Reserve1 should match deposit");
    
  //   // Verify LP tokens were minted (exact amount will depend on the calculation in the contract)
  //   assert.isAbove(Number(userLpBalance.value.amount), 0, "User should have received LP tokens");
  // });

  // Final balance check after all tests
  after(async () => {
    const finalBalance = await connection.getBalance(wallet.publicKey);
    console.log(`Final wallet balance: ${finalBalance / anchor.web3.LAMPORTS_PER_SOL} SOL`);
    console.log(`SOL spent: ${(await connection.getBalance(wallet.publicKey) - finalBalance) / anchor.web3.LAMPORTS_PER_SOL} SOL`);
  });
});