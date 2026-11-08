// v2 end-to-end: two launches — one paired with SOL (wrapped automatically), one with a Token-2022 "stock" mint
// phase 1 (no arg): create pairs, create launches, commit bids, write state. phase 2 (arg "reveal"), phase 3 (arg "settle").
const web3 = require('@solana/web3.js'); const spl = require('@solana/spl-token'); const fs = require('fs');
const makeClient = require('./rsrct.js');
const R = makeClient(web3, spl, '5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t');
const conn = new web3.Connection(process.env.RPC || 'http://127.0.0.1:8899', 'confirmed');
const payer = web3.Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync('/home/claude/sol/resurrect-program/keys/payer.json'))));
const ST = '/tmp/e2e3_state.json';
const send = async (ixs, signers) => web3.sendAndConfirmTransaction(conn, new web3.Transaction().add(...ixs), signers, { commitment: 'confirmed' });
const kp = (a) => web3.Keypair.fromSecretKey(Uint8Array.from(a));
const fmt = (v, d) => (Number(v) / 10 ** d).toLocaleString('en-US', { maximumFractionDigits: 6 });

async function phase1() {
  // --- a fake tokenized stock on Token-2022, 8 decimals, like xStocks ---
  const stock = web3.Keypair.generate();
  const lam = await conn.getMinimumBalanceForRentExemption(spl.MINT_SIZE);
  await send([
    web3.SystemProgram.createAccount({ fromPubkey: payer.publicKey, newAccountPubkey: stock.publicKey, space: spl.MINT_SIZE, lamports: lam, programId: spl.TOKEN_2022_PROGRAM_ID }),
    spl.createInitializeMint2Instruction(stock.publicKey, 8, payer.publicKey, null, spl.TOKEN_2022_PROGRAM_ID),
  ], [payer, stock]);
  console.log('mock TSLAx (Token-2022, 8 dec):', stock.publicKey.toBase58());

  const launches = [];
  for (const [tag, pairMint] of [['SOL', spl.NATIVE_MINT], ['TSLAx', stock.publicKey]]) {
    const info = await R.pairInfo(conn, pairMint);
    const mintKp = web3.Keypair.generate();
    const { ata, ixs } = await R.ixsCreateFixedMint({ connection: conn, payer: payer.publicKey, mintKp });
    const sym = tag === 'SOL' ? 'LAZ' : 'TSLZ';
    ixs.push(await R.ixCreateLaunch({ creator: payer.publicKey, mint: mintKp.publicKey, pairMint, pairProg: info.prog, creatorToken: ata, name: 'Lazarus on ' + tag, symbol: sym, batchSecs: 60, creatorFeeBps: 500 }));
    await send(ixs, [payer, mintKp]);
    const L = await R.fetchLaunch(conn, mintKp.publicKey);
    console.log(`launch ${sym} paired with ${tag} · pair prog ${info.prog.equals(spl.TOKEN_2022_PROGRAM_ID) ? 'Token-2022' : 'SPL'} · min bid ${fmt(L.minBid, L.pairDecimals)} ${tag} · vault holds ${fmt((await conn.getTokenAccountBalance(R.seedsFor(mintKp.publicKey).vault)).value.amount, 6)} tokens`);
    // creator must hold zero
    console.log('  creator balance after create:', (await conn.getTokenAccountBalance(ata)).value.uiAmountString);

    // three bidders
    const bidders = [], bids = [], salts = [];
    const amounts = tag === 'SOL' ? [1_000_000_000n, 2_500_000_000n, 500_000_000n] : [300_000_000n, 1_200_000_000n, 50_000_000n]; // SOL: 1/2.5/0.5 · stock: 3/12/0.5 shares
    for (let i = 0; i < 3; i++) {
      const b = web3.Keypair.generate(); await conn.confirmTransaction(await conn.requestAirdrop(b.publicKey, 6 * web3.LAMPORTS_PER_SOL));
      const salt = R.randomSalt(); const amount = amounts[i]; const deposit = amount + (tag === 'SOL' ? 500_000_000n : 100_000_000n);
      const prep = R.ixsPreparePair({ owner: b.publicKey, pairMint, prog: info.prog, lamports: deposit });
      const ixs = [...prep.ixs];
      if (tag !== 'SOL') ixs.push(spl.createMintToInstruction(pairMint, prep.ata, payer.publicKey, deposit, [], spl.TOKEN_2022_PROGRAM_ID)); // give the bidder some mock stock
      ixs.push(await R.ixCommitBid({ bidder: b.publicKey, mint: mintKp.publicKey, pairMint, pairProg: info.prog, amount, salt, deposit }));
      ixs.push(...R.ixsUnwrap({ owner: b.publicKey, pairMint, prog: info.prog }));
      await send(ixs, tag === 'SOL' ? [b] : [b, payer]);
      bidders.push(Array.from(b.secretKey)); bids.push(amount.toString()); salts.push(Array.from(salt));
    }
    const esc = await conn.getTokenAccountBalance(R.seedsFor(mintKp.publicKey).escrow);
    console.log(`  3 bids committed · escrow holds ${fmt(esc.value.amount, info.decimals)} ${tag}`);
    // second bid from same wallet must fail
    let dup = false; try { const b0 = kp(bidders[0]); const prep = R.ixsPreparePair({ owner: b0.publicKey, pairMint, prog: info.prog, lamports: 100_000_000n }); await send([...prep.ixs, await R.ixCommitBid({ bidder: b0.publicKey, mint: mintKp.publicKey, pairMint, pairProg: info.prog, amount: 100_000_000n, salt: R.randomSalt(), deposit: 100_000_000n })], [b0]); } catch (e) { dup = true; }
    console.log('  second bid from same wallet rejected:', dup);
    launches.push({ tag, sym, mint: mintKp.publicKey.toBase58(), pairMint: pairMint.toBase58(), prog: info.prog.toBase58(), dec: info.decimals, bidders, bids, salts, endTs: L.endTs, revealEnd: L.revealEndTs });
  }
  fs.writeFileSync(ST, JSON.stringify({ launches, stock: Array.from(stock.secretKey) }));
  console.log('state saved. reveal opens in ~60s, settle in ~11 min');
}

async function phase2() {
  const S = JSON.parse(fs.readFileSync(ST));
  for (const L of S.launches) {
    const now = Math.floor(Date.now() / 1000); if (now < L.endTs) { console.log('batch still open', L.endTs - now, 's'); process.exit(2); }
    const mint = new web3.PublicKey(L.mint);
    for (let i = 0; i < 3; i++) { const b = kp(L.bidders[i]); await send([await R.ixRevealBid({ bidder: b.publicKey, mint, amount: BigInt(L.bids[i]), salt: Uint8Array.from(L.salts[i]) })], [b]); }
    const l = await R.fetchLaunch(conn, mint);
    console.log(`${L.sym}: ${l.revealed}/${l.bids} revealed · total ${fmt(l.totalRevealed, L.dec)} ${L.tag}`);
  }
}

async function phase3() {
  const S = JSON.parse(fs.readFileSync(ST));
  for (const L of S.launches) {
    const now = Math.floor(Date.now() / 1000); if (now < L.revealEnd) { console.log('reveal window still open', L.revealEnd - now, 's'); process.exit(2); }
    const mint = new web3.PublicKey(L.mint), pairMint = new web3.PublicKey(L.pairMint), prog = new web3.PublicKey(L.prog); const k = R.seedsFor(mint);
    await send([await R.ixSettle({ mint, pairMint, pairProg: prog })], [payer]);
    let l = await R.fetchLaunch(conn, mint);
    const price = R.priceFromQ32(l.clearingPriceQ32) * 1e6 / 10 ** L.dec;
    console.log(`\n${L.sym} settled · clearing price ${price.toFixed(9)} ${L.tag}/token · pool ${fmt(l.poolPair, L.dec)} ${L.tag} + ${fmt(l.poolTokens, 6)} tokens`);
    console.log('  pool token account:', fmt((await conn.getTokenAccountBalance(k.pool)).value.amount, L.dec), L.tag, '· escrow left:', fmt((await conn.getTokenAccountBalance(k.escrow)).value.amount, L.dec));
    for (let i = 0; i < 3; i++) {
      const b = kp(L.bidders[i]);
      const prep = R.ixsPreparePair({ owner: b.publicKey, pairMint, prog });
      const before = await conn.getBalance(b.publicKey);
      await send([...prep.ixs, spl.createAssociatedTokenAccountIdempotentInstruction(b.publicKey, R.ata(mint, b.publicKey), b.publicKey, mint),
        await R.ixClaim({ bidder: b.publicKey, mint, pairMint, pairProg: prog }), ...R.ixsUnwrap({ owner: b.publicKey, pairMint, prog })], [b]);
      const tok = await conn.getTokenAccountBalance(R.ata(mint, b.publicKey));
      const refund = L.tag === 'SOL' ? ((await conn.getBalance(b.publicKey) - before) / 1e9).toFixed(3) + ' SOL' : fmt((await conn.getTokenAccountBalance(prep.ata)).value.amount, L.dec) + ' ' + L.tag;
      console.log(`  bidder ${i} bid ${fmt(L.bids[i], L.dec)} ${L.tag} → ${Number(tok.value.uiAmountString).toLocaleString('en-US')} tokens · refund ≈ ${refund}`);
    }
    // trader: buys with pair asset, sells half back
    const t = web3.Keypair.generate(); await conn.confirmTransaction(await conn.requestAirdrop(t.publicKey, 3 * web3.LAMPORTS_PER_SOL));
    const buyIn = L.tag === 'SOL' ? 300_000_000n : 100_000_000n;
    const prep = R.ixsPreparePair({ owner: t.publicKey, pairMint, prog, lamports: buyIn });
    const ixs = [...prep.ixs, spl.createAssociatedTokenAccountIdempotentInstruction(t.publicKey, R.ata(mint, t.publicKey), t.publicKey, mint)];
    if (L.tag !== 'SOL') ixs.push(spl.createMintToInstruction(pairMint, prep.ata, payer.publicKey, buyIn, [], spl.TOKEN_2022_PROGRAM_ID));
    ixs.push(await R.ixSwap({ trader: t.publicKey, mint, pairMint, pairProg: prog, pairForTokens: true, amountIn: buyIn, minOut: 1n }), ...R.ixsUnwrap({ owner: t.publicKey, pairMint, prog }));
    await send(ixs, L.tag === 'SOL' ? [t] : [t, payer]);
    const tb = await conn.getTokenAccountBalance(R.ata(mint, t.publicKey));
    console.log(`  trader bought ${Number(tb.value.uiAmountString).toLocaleString('en-US')} tokens for ${fmt(buyIn, L.dec)} ${L.tag}`);
    const half = BigInt(tb.value.amount) / 2n; const prep2 = R.ixsPreparePair({ owner: t.publicKey, pairMint, prog }); const sb = await conn.getBalance(t.publicKey);
    await send([...prep2.ixs, await R.ixSwap({ trader: t.publicKey, mint, pairMint, pairProg: prog, pairForTokens: false, amountIn: half, minOut: 1n }), ...R.ixsUnwrap({ owner: t.publicKey, pairMint, prog })], [t]);
    const got = L.tag === 'SOL' ? ((await conn.getBalance(t.publicKey) - sb) / 1e9).toFixed(4) + ' SOL' : fmt((await conn.getTokenAccountBalance(prep2.ata)).value.amount, L.dec) + ' ' + L.tag;
    console.log(`  trader sold half back for ${got}`);
    l = await R.fetchLaunch(conn, mint);
    console.log(`  fees accrued ${fmt(l.feesAccrued, L.dec)} ${L.tag} · creator owed ${fmt(l.creatorFeesOwed, L.dec)} (5%) · fee vault ${fmt((await conn.getTokenAccountBalance(k.fees)).value.amount, L.dec)}`);
    // creator claims their share
    const cprep = R.ixsPreparePair({ owner: payer.publicKey, pairMint, prog });
    await send([...cprep.ixs, await R.ixClaimCreatorFees({ creator: payer.publicKey, mint, pairMint, pairProg: prog })], [payer]);
    l = await R.fetchLaunch(conn, mint);
    console.log(`  creator claimed ${fmt(l.creatorFeesClaimed, L.dec)} ${L.tag} · owed now ${l.creatorFeesOwed} · vault keeps ${fmt((await conn.getTokenAccountBalance(k.fees)).value.amount, L.dec)} for holders`);
    let again = false; try { await send([await R.ixClaimCreatorFees({ creator: payer.publicKey, mint, pairMint, pairProg: prog })], [payer]); } catch (e) { again = true; }
    console.log('  double fee claim rejected:', again);
  }
  console.log('\nlaunches on chain:', (await R.fetchLaunches(conn)).map(l => l.symbol + '/' + (l.pairMint.equals(spl.NATIVE_MINT) ? 'SOL' : l.pairMint.toBase58().slice(0, 4))));
}
const ph = process.argv[2]; (ph === 'reveal' ? phase2() : ph === 'settle' ? phase3() : phase1()).catch(e => { console.error(e.logs ? e.logs.join('\n') : e); console.error(e.message); process.exit(1); });
