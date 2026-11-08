// Resurrect client v2 — framework-free encoder for the on-chain program (works in Node and browser)
// Any-asset pairing: every launch is paired with `pairMint` (wrapped SOL, USDC, xStocks, Ondo…). SPL Token or Token-2022.
// Usage (browser): window.RSRCT = makeClient(solanaWeb3, splToken, programId)
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.makeRsrct = factory();
})(typeof self !== 'undefined' ? self : this, function () {
  return function makeClient(web3, spl, programIdStr) {
    const { PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY, TransactionInstruction } = web3;
    const PROGRAM_ID = new PublicKey(programIdStr);
    const TOKEN = spl.TOKEN_PROGRAM_ID, TOKEN22 = spl.TOKEN_2022_PROGRAM_ID, NATIVE = spl.NATIVE_MINT;
    const enc = new TextEncoder();
    const sha256 = async (bytes) => new Uint8Array(await (globalThis.crypto.subtle).digest('SHA-256', bytes));
    const concat = (...arrs) => { const n = arrs.reduce((a, b) => a + b.length, 0); const o = new Uint8Array(n); let p = 0; for (const a of arrs) { o.set(a, p); p += a.length; } return o; };
    const u64 = (v) => { const b = new Uint8Array(8); let x = BigInt(v); for (let i = 0; i < 8; i++) { b[i] = Number(x & 0xffn); x >>= 8n; } return b; };
    const i64 = u64;
    const u16 = (v) => new Uint8Array([v & 0xff, (v >> 8) & 0xff]);
    const str = (s) => { const b = enc.encode(s); return concat(new Uint8Array([b.length, 0, 0, 0]), b); };
    async function disc(name) { return (await sha256(enc.encode('global:' + name))).slice(0, 8); }
    const pda = (seeds) => PublicKey.findProgramAddressSync(seeds, PROGRAM_ID)[0];
    // all PDAs hang off the launched coin's mint
    const seedsFor = (mint) => {
      const m = mint.toBuffer();
      return { launch: pda([enc.encode('launch'), m]), vault: pda([enc.encode('vault'), m]), escrow: pda([enc.encode('escrow'), m]), pool: pda([enc.encode('pool'), m]), fees: pda([enc.encode('fees'), m]), stakeVault: pda([enc.encode('stake'), m]) };
    };
    const bidPda = (launch, bidder) => pda([enc.encode('bid'), launch.toBuffer(), bidder.toBuffer()]);
    const stakePda = (launch, owner) => pda([enc.encode('stake'), launch.toBuffer(), owner.toBuffer()]);
    // Civic gateway: pass account for a wallet on a gatekeeper network
    const GATEWAY = new PublicKey('gatem74V238djXdzWnJf94Wo1DcnuGkfijbf3AuBhfs');
    const UNIQUENESS = new PublicKey('uniqobk8oGh4XBLMqM68K8M2zNu3CdYX7q5go7whQiv');
    const gatewayTokenPda = (owner, network = UNIQUENESS) => PublicKey.findProgramAddressSync([owner.toBuffer(), enc.encode('gateway'), new Uint8Array(8), network.toBuffer()], GATEWAY)[0];
    const meta = (pk, s = false, w = false) => ({ pubkey: pk, isSigner: s, isWritable: w });
    const ata = (mint, owner, prog = TOKEN) => spl.getAssociatedTokenAddressSync(mint, owner, false, prog);

    // ---- pair asset helpers ----
    /// Which token program owns this mint (SPL Token or Token-2022), plus decimals.
    async function pairInfo(connection, pairMint) {
      const ai = await connection.getAccountInfo(pairMint);
      if (!ai) throw new Error('pair mint not found on this cluster');
      const prog = ai.owner.equals(TOKEN22) ? TOKEN22 : TOKEN;
      const decimals = ai.data[44];
      return { prog, decimals, isNative: pairMint.equals(NATIVE) };
    }
    /// Instructions that make sure `owner` has a pair-asset ATA; when the pair is SOL, wrap `lamports` into it.
    function ixsPreparePair({ owner, pairMint, prog, lamports = 0n }) {
      const a = ata(pairMint, owner, prog);
      const ixs = [spl.createAssociatedTokenAccountIdempotentInstruction(owner, a, owner, pairMint, prog)];
      if (pairMint.equals(NATIVE) && lamports > 0n) {
        ixs.push(SystemProgram.transfer({ fromPubkey: owner, toPubkey: a, lamports: BigInt(lamports) }), spl.createSyncNativeInstruction(a, prog));
      }
      return { ata: a, ixs };
    }
    /// For SOL pairs: close the wSOL ATA afterwards so the wallet holds plain SOL again.
    function ixsUnwrap({ owner, pairMint, prog }) {
      if (!pairMint.equals(NATIVE)) return [];
      return [spl.createCloseAccountInstruction(ata(pairMint, owner, prog), owner, owner, [], prog)];
    }

    // ---- instructions ----
    async function ixCreateLaunch({ creator, mint, pairMint, pairProg, creatorToken, name, symbol, batchSecs, creatorFeeBps, requireHuman = false }) {
      const k = seedsFor(mint);
      const data = concat(await disc('create_launch'), str(name), str(symbol), i64(batchSecs), u16(creatorFeeBps), new Uint8Array([requireHuman ? 1 : 0]));
      return new TransactionInstruction({ programId: PROGRAM_ID, data, keys: [
        meta(creator, true, true), meta(mint), meta(pairMint), meta(k.launch, false, true), meta(k.vault, false, true),
        meta(k.escrow, false, true), meta(k.pool, false, true), meta(k.fees, false, true), meta(k.stakeVault, false, true), meta(creatorToken, false, true),
        meta(TOKEN), meta(pairProg), meta(SystemProgram.programId), meta(SYSVAR_RENT_PUBKEY)] });
    }
    async function commitment(bidder, amount, salt) { return sha256(concat(bidder.toBytes(), u64(amount), salt)); }
    async function ixCommitBid({ bidder, mint, pairMint, pairProg, amount, salt, deposit, gatewayToken = null }) {
      const k = seedsFor(mint); const c = await commitment(bidder, amount, salt);
      const data = concat(await disc('commit_bid'), c, u64(deposit));
      return new TransactionInstruction({ programId: PROGRAM_ID, data, keys: [
        meta(bidder, true, true), meta(k.launch, false, true), meta(bidPda(k.launch, bidder), false, true), meta(pairMint), meta(k.escrow, false, true),
        meta(ata(pairMint, bidder, pairProg), false, true), meta(pairProg), meta(SystemProgram.programId), meta(gatewayToken || PROGRAM_ID)] });
    }
    async function ixStake({ staker, mint, amount, unstake = false }) {
      const k = seedsFor(mint);
      const data = concat(await disc(unstake ? 'unstake' : 'stake'), u64(amount));
      return new TransactionInstruction({ programId: PROGRAM_ID, data, keys: [meta(staker, true, true), meta(k.launch, false, true), meta(stakePda(k.launch, staker), false, true), meta(k.stakeVault, false, true), meta(ata(mint, staker), false, true), meta(TOKEN), meta(SystemProgram.programId)] });
    }
    async function ixClaimRewards({ staker, mint, pairMint, pairProg }) {
      const k = seedsFor(mint);
      return new TransactionInstruction({ programId: PROGRAM_ID, data: await disc('claim_rewards'), keys: [meta(staker, true, true), meta(k.launch, false, true), meta(stakePda(k.launch, staker), false, true), meta(pairMint), meta(k.fees, false, true), meta(ata(pairMint, staker, pairProg), false, true), meta(pairProg)] });
    }
    async function ixRevealBid({ bidder, mint, amount, salt }) {
      const k = seedsFor(mint);
      const data = concat(await disc('reveal_bid'), u64(amount), salt);
      return new TransactionInstruction({ programId: PROGRAM_ID, data, keys: [meta(bidder, true, false), meta(k.launch, false, true), meta(bidPda(k.launch, bidder), false, true)] });
    }
    async function ixSettle({ mint, pairMint, pairProg }) {
      const k = seedsFor(mint);
      return new TransactionInstruction({ programId: PROGRAM_ID, data: await disc('settle'), keys: [meta(k.launch, false, true), meta(pairMint), meta(k.escrow, false, true), meta(k.pool, false, true), meta(pairProg)] });
    }
    async function ixClaim({ bidder, mint, pairMint, pairProg }) {
      const k = seedsFor(mint);
      return new TransactionInstruction({ programId: PROGRAM_ID, data: await disc('claim'), keys: [
        meta(bidder, true, true), meta(k.launch), meta(bidPda(k.launch, bidder), false, true), meta(pairMint), meta(k.escrow, false, true),
        meta(ata(pairMint, bidder, pairProg), false, true), meta(k.vault, false, true), meta(ata(mint, bidder), false, true), meta(TOKEN), meta(pairProg)] });
    }
    async function ixSwap({ trader, mint, pairMint, pairProg, pairForTokens, amountIn, minOut }) {
      const k = seedsFor(mint);
      const data = concat(await disc(pairForTokens ? 'swap_pair_for_tokens' : 'swap_tokens_for_pair'), u64(amountIn), u64(minOut));
      return new TransactionInstruction({ programId: PROGRAM_ID, data, keys: [
        meta(trader, true, true), meta(k.launch, false, true), meta(pairMint), meta(k.pool, false, true), meta(k.fees, false, true), meta(k.vault, false, true),
        meta(ata(mint, trader), false, true), meta(ata(pairMint, trader, pairProg), false, true), meta(TOKEN), meta(pairProg)] });
    }
    async function ixClaimCreatorFees({ creator, mint, pairMint, pairProg }) {
      const k = seedsFor(mint);
      return new TransactionInstruction({ programId: PROGRAM_ID, data: await disc('claim_creator_fees'), keys: [
        meta(creator, true, false), meta(k.launch, false, true), meta(pairMint), meta(k.fees, false, true), meta(ata(pairMint, creator, pairProg), false, true), meta(pairProg)] });
    }

    // ---- decoding ----
    const rd = (buf) => { let o = 8; const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength); return {
      pk: () => { const p = new PublicKey(buf.slice(o, o + 32)); o += 32; return p; },
      str: () => { const n = dv.getUint32(o, true); o += 4; const s = new TextDecoder().decode(buf.slice(o, o + n)); o += n; return s; },
      i64: () => { const v = dv.getBigInt64(o, true); o += 8; return v; }, u64: () => { const v = dv.getBigUint64(o, true); o += 8; return v; },
      u128: () => { const lo = dv.getBigUint64(o, true), hi = dv.getBigUint64(o + 8, true); o += 16; return (hi << 64n) | lo; },
      u16: () => { const v = dv.getUint16(o, true); o += 2; return v; }, u32: () => { const v = dv.getUint32(o, true); o += 4; return v; },
      u8: () => buf[o++], bool: () => !!buf[o++], bytes: (n) => { const b = buf.slice(o, o + n); o += n; return b; } }; };
    function decodeLaunch(buf) { const r = rd(buf); return {
      creator: r.pk(), mint: r.pk(), vault: r.pk(), pairMint: r.pk(), pairTokenProgram: r.pk(), escrow: r.pk(), poolPairAccount: r.pk(), feeVault: r.pk(),
      name: r.str(), symbol: r.str(), startTs: Number(r.i64()), endTs: Number(r.i64()), revealEndTs: Number(r.i64()),
      creatorFeeBps: r.u16(), pairDecimals: r.u8(), minBid: r.u64(), bids: r.u32(), revealed: r.u32(), totalRevealed: r.u64(), clearingPriceQ32: r.u128(),
      settled: r.bool(), poolPair: r.u64(), poolTokens: r.u64(), feesAccrued: r.u64(), creatorFeesOwed: r.u64(), creatorFeesClaimed: r.u64(),
      requireHuman: r.bool(), gatekeeperNetwork: r.pk(), stakeVault: r.pk(), totalStaked: r.u64(), rewardIndex: r.u128(), rewardsPending: r.u64(), rewardsDistributed: r.u64(), bump: r.u8() }; }
    function decodeStake(buf) { const r = rd(buf); return { launch: r.pk(), owner: r.pk(), amount: r.u64(), index: r.u128(), owed: r.u64(), claimed: r.u64(), bump: r.u8() }; }
    async function fetchStake(connection, mint, owner) { const k = seedsFor(mint); const a = await connection.getAccountInfo(stakePda(k.launch, owner)); return a ? decodeStake(new Uint8Array(a.data)) : null; }
    /// pending rewards for a stake given the current launch state (same math as the program)
    const pendingRewards = (l, s) => s ? s.owed + (l.rewardIndex > s.index ? ((l.rewardIndex - s.index) * s.amount) >> 64n : 0n) : 0n;
    async function hasHumanPass(connection, owner, network = UNIQUENESS) { const a = await connection.getAccountInfo(gatewayTokenPda(owner, network)); if (!a || !a.owner.equals(GATEWAY)) return false; const d = a.data; let o = 1; o += d[o] ? 33 : 1; const w = new PublicKey(d.slice(o, o + 32)); o += 32; o += d[o] ? 33 : 1; o += 64; const state = d[o]; o += 1; const exp = d[o] ? Number(new DataView(d.buffer, d.byteOffset + o + 1, 8).getBigInt64(0, true)) : null; return w.equals(owner) && state === 0 && (exp === null || exp > Date.now() / 1000); }
    function decodeBid(buf) { const r = rd(buf); return { launch: r.pk(), bidder: r.pk(), commitment: r.bytes(32), deposit: r.u64(), amount: r.u64(), revealed: r.bool(), claimed: r.bool(), bump: r.u8() }; }
    async function accountDisc(name) { return (await sha256(enc.encode('account:' + name))).slice(0, 8); }
    async function fetchLaunches(connection) {
      const d = await accountDisc('Launch');
      const accs = await connection.getProgramAccounts(PROGRAM_ID, { filters: [{ memcmp: { offset: 0, bytes: web3Bs58(d) } }] });
      return accs.map(a => ({ pubkey: a.pubkey, ...decodeLaunch(new Uint8Array(a.account.data)) }));
    }
    async function fetchLaunch(connection, mint) { const k = seedsFor(mint); const a = await connection.getAccountInfo(k.launch); return a ? { pubkey: k.launch, ...decodeLaunch(new Uint8Array(a.data)) } : null; }
    async function fetchBid(connection, mint, bidder) { const k = seedsFor(mint); const a = await connection.getAccountInfo(bidPda(k.launch, bidder)); return a ? decodeBid(new Uint8Array(a.data)) : null; }
    const ALPH = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
    function web3Bs58(bytes) { let x = 0n; for (const b of bytes) x = (x << 8n) | BigInt(b); let s = ''; while (x > 0n) { s = ALPH[Number(x % 58n)] + s; x /= 58n; } for (const b of bytes) { if (b === 0) s = '1' + s; else break; } return s; }

    // ---- helpers for the fixed-supply mint (1B, 6 decimals, authorities revoked) ----
    const SUPPLY = 1_000_000_000_000_000n;
    async function ixsCreateFixedMint({ connection, payer, mintKp }) {
      const lamports = await connection.getMinimumBalanceForRentExemption(spl.MINT_SIZE);
      const a = ata(mintKp.publicKey, payer);
      return { ata: a, ixs: [
        SystemProgram.createAccount({ fromPubkey: payer, newAccountPubkey: mintKp.publicKey, space: spl.MINT_SIZE, lamports, programId: TOKEN }),
        spl.createInitializeMint2Instruction(mintKp.publicKey, 6, payer, null),
        spl.createAssociatedTokenAccountInstruction(payer, a, payer, mintKp.publicKey),
        spl.createMintToInstruction(mintKp.publicKey, a, payer, SUPPLY),
        spl.createSetAuthorityInstruction(mintKp.publicKey, payer, spl.AuthorityType.MintTokens, null),
      ] };
    }
    const priceFromQ32 = (q) => Number(q) / 2 ** 32; // pair base units per token base unit
    return { PROGRAM_ID, TOKEN, TOKEN22, NATIVE, GATEWAY, UNIQUENESS, seedsFor, bidPda, stakePda, gatewayTokenPda, ata, commitment, pairInfo, ixsPreparePair, ixsUnwrap,
      ixCreateLaunch, ixCommitBid, ixRevealBid, ixSettle, ixClaim, ixSwap, ixClaimCreatorFees, ixStake, ixClaimRewards,
      decodeLaunch, decodeBid, decodeStake, fetchLaunches, fetchLaunch, fetchBid, fetchStake, pendingRewards, hasHumanPass, ixsCreateFixedMint, SUPPLY, priceFromQ32, randomSalt: () => globalThis.crypto.getRandomValues(new Uint8Array(32)) };
  };
});
