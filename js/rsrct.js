// Resurrect client — framework-free encoder for the on-chain program (works in Node and browser)
// Usage (browser): window.RSRCT = makeClient(solanaWeb3, splToken, programId)
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.makeRsrct = factory();
})(typeof self !== 'undefined' ? self : this, function () {
  return function makeClient(web3, spl, programIdStr) {
    const { PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY, TransactionInstruction, Transaction, Keypair } = web3;
    const PROGRAM_ID = new PublicKey(programIdStr);
    const enc = new TextEncoder();
    const sha256 = async (bytes) => new Uint8Array(await (globalThis.crypto.subtle).digest('SHA-256', bytes));
    const concat = (...arrs) => { const n = arrs.reduce((a, b) => a + b.length, 0); const o = new Uint8Array(n); let p = 0; for (const a of arrs) { o.set(a, p); p += a.length; } return o; };
    const u64 = (v) => { const b = new Uint8Array(8); let x = BigInt(v); for (let i = 0; i < 8; i++) { b[i] = Number(x & 0xffn); x >>= 8n; } return b; };
    const i64 = u64;
    const u16 = (v) => new Uint8Array([v & 0xff, (v >> 8) & 0xff]);
    const str = (s) => { const b = enc.encode(s); return concat(new Uint8Array([b.length, 0, 0, 0]), b); };
    async function disc(name) { return (await sha256(enc.encode('global:' + name))).slice(0, 8); }
    const pda = (seeds) => PublicKey.findProgramAddressSync(seeds, PROGRAM_ID)[0];
    const seedsFor = (mint) => {
      const launch = pda([enc.encode('launch'), mint.toBuffer()]);
      return { launch, vault: pda([enc.encode('vault'), mint.toBuffer()]), escrow: pda([enc.encode('escrow'), launch.toBuffer()]), pool: pda([enc.encode('pool'), launch.toBuffer()]), fees: pda([enc.encode('fees'), launch.toBuffer()]) };
    };
    const bidPda = (launch, bidder) => pda([enc.encode('bid'), launch.toBuffer(), bidder.toBuffer()]);
    const meta = (pk, s = false, w = false) => ({ pubkey: pk, isSigner: s, isWritable: w });

    // ---- instructions ----
    async function ixCreateLaunch({ creator, mint, creatorToken, name, symbol, batchSecs, creatorFeeBps }) {
      const k = seedsFor(mint);
      const data = concat(await disc('create_launch'), str(name), str(symbol), i64(batchSecs), u16(creatorFeeBps));
      return new TransactionInstruction({ programId: PROGRAM_ID, data, keys: [meta(creator, true, true), meta(mint), meta(k.launch, false, true), meta(k.vault, false, true), meta(creatorToken, false, true), meta(spl.TOKEN_PROGRAM_ID), meta(SystemProgram.programId), meta(SYSVAR_RENT_PUBKEY)] });
    }
    async function commitment(bidder, lamports, salt) { return sha256(concat(bidder.toBytes(), u64(lamports), salt)); }
    async function ixCommitBid({ bidder, mint, lamports, salt, deposit }) {
      const k = seedsFor(mint); const c = await commitment(bidder, lamports, salt);
      const data = concat(await disc('commit_bid'), c, u64(deposit));
      return new TransactionInstruction({ programId: PROGRAM_ID, data, keys: [meta(bidder, true, true), meta(k.launch, false, true), meta(bidPda(k.launch, bidder), false, true), meta(k.escrow, false, true), meta(SystemProgram.programId)] });
    }
    async function ixRevealBid({ bidder, mint, lamports, salt }) {
      const k = seedsFor(mint);
      const data = concat(await disc('reveal_bid'), u64(lamports), salt);
      return new TransactionInstruction({ programId: PROGRAM_ID, data, keys: [meta(bidder, true, false), meta(k.launch, false, true), meta(bidPda(k.launch, bidder), false, true)] });
    }
    async function ixSettle({ mint }) {
      const k = seedsFor(mint);
      return new TransactionInstruction({ programId: PROGRAM_ID, data: await disc('settle'), keys: [meta(k.launch, false, true), meta(k.escrow, false, true), meta(k.pool, false, true), meta(SystemProgram.programId)] });
    }
    async function ixClaim({ bidder, mint, bidderToken }) {
      const k = seedsFor(mint);
      return new TransactionInstruction({ programId: PROGRAM_ID, data: await disc('claim'), keys: [meta(bidder, true, true), meta(k.launch), meta(bidPda(k.launch, bidder), false, true), meta(k.escrow, false, true), meta(k.vault, false, true), meta(bidderToken, false, true), meta(spl.TOKEN_PROGRAM_ID), meta(SystemProgram.programId)] });
    }
    async function ixSwap({ trader, mint, traderToken, solForTokens, amountIn, minOut }) {
      const k = seedsFor(mint);
      const data = concat(await disc(solForTokens ? 'swap_sol_for_tokens' : 'swap_tokens_for_sol'), u64(amountIn), u64(minOut));
      return new TransactionInstruction({ programId: PROGRAM_ID, data, keys: [meta(trader, true, true), meta(k.launch, false, true), meta(k.pool, false, true), meta(k.fees, false, true), meta(k.vault, false, true), meta(traderToken, false, true), meta(spl.TOKEN_PROGRAM_ID), meta(SystemProgram.programId)] });
    }

    // ---- decoding ----
    const rd = (buf) => { let o = 8; const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength); return {
      pk: () => { const p = new PublicKey(buf.slice(o, o + 32)); o += 32; return p; },
      str: () => { const n = dv.getUint32(o, true); o += 4; const s = new TextDecoder().decode(buf.slice(o, o + n)); o += n; return s; },
      i64: () => { const v = dv.getBigInt64(o, true); o += 8; return v; }, u64: () => { const v = dv.getBigUint64(o, true); o += 8; return v; },
      u128: () => { const lo = dv.getBigUint64(o, true), hi = dv.getBigUint64(o + 8, true); o += 16; return (hi << 64n) | lo; },
      u16: () => { const v = dv.getUint16(o, true); o += 2; return v; }, u32: () => { const v = dv.getUint32(o, true); o += 4; return v; },
      u8: () => buf[o++], bool: () => !!buf[o++], bytes: (n) => { const b = buf.slice(o, o + n); o += n; return b; } }; };
    function decodeLaunch(buf) { const r = rd(buf); return { creator: r.pk(), mint: r.pk(), vault: r.pk(), name: r.str(), symbol: r.str(), startTs: Number(r.i64()), endTs: Number(r.i64()), revealEndTs: Number(r.i64()), creatorFeeBps: r.u16(), bids: r.u32(), revealed: r.u32(), totalRevealedLamports: r.u64(), clearingPriceQ32: r.u128(), settled: r.bool(), poolSol: r.u64(), poolTokens: r.u64(), feeLamports: r.u64(), bump: r.u8() }; }
    function decodeBid(buf) { const r = rd(buf); return { launch: r.pk(), bidder: r.pk(), commitment: r.bytes(32), deposit: r.u64(), amount: r.u64(), revealed: r.bool(), claimed: r.bool(), bump: r.u8() }; }
    async function accountDisc(name) { return (await sha256(enc.encode('account:' + name))).slice(0, 8); }
    async function fetchLaunches(connection) {
      const d = await accountDisc('Launch');
      const accs = await connection.getProgramAccounts(PROGRAM_ID, { filters: [{ memcmp: { offset: 0, bytes: web3Bs58(d) } }] });
      return accs.map(a => ({ pubkey: a.pubkey, ...decodeLaunch(new Uint8Array(a.account.data)) }));
    }
    async function fetchLaunch(connection, mint) { const k = seedsFor(mint); const a = await connection.getAccountInfo(k.launch); return a ? { pubkey: k.launch, ...decodeLaunch(new Uint8Array(a.data)) } : null; }
    async function fetchBid(connection, mint, bidder) { const k = seedsFor(mint); const a = await connection.getAccountInfo(bidPda(k.launch, bidder)); return a ? decodeBid(new Uint8Array(a.data)) : null; }
    // tiny base58 for memcmp
    const ALPH = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
    function web3Bs58(bytes) { let x = 0n; for (const b of bytes) x = (x << 8n) | BigInt(b); let s = ''; while (x > 0n) { s = ALPH[Number(x % 58n)] + s; x /= 58n; } for (const b of bytes) { if (b === 0) s = '1' + s; else break; } return s; }

    // ---- helpers for the fixed-supply mint (1B, 6 decimals, authorities revoked) ----
    const SUPPLY = 1_000_000_000_000_000n;
    async function ixsCreateFixedMint({ connection, payer, mintKp }) {
      const lamports = await connection.getMinimumBalanceForRentExemption(spl.MINT_SIZE);
      const ata = spl.getAssociatedTokenAddressSync(mintKp.publicKey, payer);
      return { ata, ixs: [
        SystemProgram.createAccount({ fromPubkey: payer, newAccountPubkey: mintKp.publicKey, space: spl.MINT_SIZE, lamports, programId: spl.TOKEN_PROGRAM_ID }),
        spl.createInitializeMint2Instruction(mintKp.publicKey, 6, payer, null),
        spl.createAssociatedTokenAccountInstruction(payer, ata, payer, mintKp.publicKey),
        spl.createMintToInstruction(mintKp.publicKey, ata, payer, SUPPLY),
        spl.createSetAuthorityInstruction(mintKp.publicKey, payer, spl.AuthorityType.MintTokens, null),
      ] };
    }
    const priceFromQ32 = (q) => Number(q) / 2 ** 32; // lamports per base unit
    return { PROGRAM_ID, seedsFor, bidPda, commitment, ixCreateLaunch, ixCommitBid, ixRevealBid, ixSettle, ixClaim, ixSwap, decodeLaunch, decodeBid, fetchLaunches, fetchLaunch, fetchBid, ixsCreateFixedMint, SUPPLY, priceFromQ32, randomSalt: () => globalThis.crypto.getRandomValues(new Uint8Array(32)) };
  };
});
