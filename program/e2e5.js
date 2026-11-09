// v4: fee split 35/25/20/≤10/10, treasury collection, RSRCT buyback & burn through our own pool, verified-trader weekly rewards.
// phase 1 (no arg): config + two SOL-paired launches (R = mock RSRCT, A = a coin) + bids. phase 2 "reveal". phase 3 "settle" (does everything else).
const web3 = require('@solana/web3.js'); const spl = require('@solana/spl-token'); const fs = require('fs');
const R = require('./rsrct.js')(web3, spl, '5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t', { epochSecs: 90 });
const conn = new web3.Connection('http://127.0.0.1:8899', 'confirmed');
const kp = (a) => web3.Keypair.fromSecretKey(Uint8Array.from(a));
const payer = kp(JSON.parse(fs.readFileSync('/home/claude/sol/resurrect-program/keys/payer.json')));
const human = kp(JSON.parse(fs.readFileSync('/tmp/human.json')));
const ST = '/tmp/e2e5_state.json'; const SOL = spl.NATIVE_MINT, TP = spl.TOKEN_PROGRAM_ID;
const send = (ixs, s) => web3.sendAndConfirmTransaction(conn, new web3.Transaction().add(...ixs), s, { commitment: 'confirmed' });
const sol = v => (Number(v) / 1e9).toFixed(6) + ' SOL';
const P = (mint) => ({ mint, pairMint: SOL, pairProg: TP });
const wrap = (o, lam) => R.ixsPreparePair({ owner: o, pairMint: SOL, prog: TP, lamports: lam }).ixs, unwrap = (o) => R.ixsUnwrap({ owner: o, pairMint: SOL, prog: TP });
const ataIx = (mint, o) => spl.createAssociatedTokenAccountIdempotentInstruction(o, R.ata(mint, o), o, mint);
const expectFail = async (f, re) => { try { await f(); return 'ACCEPTED (BUG)'; } catch (e) { const t = JSON.stringify(e.logs || e.message); return re.test(t) ? 'rejected ✓ (' + re.source + ')' : 'rejected (other): ' + (e.message || '').slice(0, 90); } };

async function phase1() {
  const treasury = web3.Keypair.generate();
  await send([await R.ixInitConfig({ admin: payer.publicKey, treasury: treasury.publicKey })], [payer]);
  console.log('config initialised · treasury', treasury.publicKey.toBase58().slice(0, 8), '· non-admin init:', await expectFail(async () => { const x = web3.Keypair.generate(); await conn.confirmTransaction(await conn.requestAirdrop(x.publicKey, 1e9)); await send([await R.ixInitConfig({ admin: x.publicKey, treasury: x.publicKey })], [x]); }, /ConstraintAddress|already in use|2012|0x0/));
  await conn.confirmTransaction(await conn.requestAirdrop(human.publicKey, 20e9));
  const out = { treasury: Array.from(treasury.secretKey), launches: [] };
  for (const [sym, name] of [['RSRCT', 'Resurrect'], ['LAZ', 'Lazarus']]) {
    const mk = web3.Keypair.generate(); const { ata, ixs } = await R.ixsCreateFixedMint({ connection: conn, payer: payer.publicKey, mintKp: mk });
    ixs.push(await R.ixCreateLaunch({ creator: payer.publicKey, ...P(mk.publicKey), creatorToken: ata, name, symbol: sym, batchSecs: 60, creatorFeeBps: sym === 'LAZ' ? 500 : 0 }));
    await send(ixs, [payer, mk]);
    console.log('creator fee 15% rejected:', sym === 'LAZ' ? await expectFail(async () => { const m2 = web3.Keypair.generate(); const c = await R.ixsCreateFixedMint({ connection: conn, payer: payer.publicKey, mintKp: m2 }); c.ixs.push(await R.ixCreateLaunch({ creator: payer.publicKey, ...P(m2.publicKey), creatorToken: c.ata, name: 'x', symbol: 'X', batchSecs: 60, creatorFeeBps: 1500 })); await send(c.ixs, [payer, m2]); }, /BadParams|6000/) : '(skipped)');
    const bidders = []; const bids = []; const salts = [];
    for (const [b, amt] of [[human, 2_000_000_000n], [web3.Keypair.generate(), 3_000_000_000n]]) {
      if (b !== human) await conn.confirmTransaction(await conn.requestAirdrop(b.publicKey, 10e9));
      const salt = R.randomSalt(); const dep = amt + 500_000_000n;
      await send([...wrap(b.publicKey, dep), await R.ixCommitBid({ bidder: b.publicKey, ...P(mk.publicKey), amount: amt, salt, deposit: dep }), ...unwrap(b.publicKey)], [b]);
      bidders.push(Array.from(b.secretKey)); bids.push(amt.toString()); salts.push(Array.from(salt));
    }
    const L = await R.fetchLaunch(conn, mk.publicKey);
    console.log(`launch $${sym} · 2 sealed bids · ends ${L.endTs}`);
    out.launches.push({ sym, mint: mk.publicKey.toBase58(), bidders, bids, salts, endTs: L.endTs, revealEnd: L.revealEndTs });
  }
  fs.writeFileSync(ST, JSON.stringify(out)); console.log('state saved');
}
async function phase2() { const S = JSON.parse(fs.readFileSync(ST)); for (const L of S.launches) { const now = Math.floor(Date.now() / 1000); if (now < L.endTs) { console.log('batch open', L.endTs - now); process.exit(2); } const mint = new web3.PublicKey(L.mint); for (let i = 0; i < 2; i++) { const b = kp(L.bidders[i]); await send([await R.ixRevealBid({ bidder: b.publicKey, mint, amount: BigInt(L.bids[i]), salt: Uint8Array.from(L.salts[i]) })], [b]); } console.log(L.sym, 'revealed'); } }

async function phase3() {
  const S = JSON.parse(fs.readFileSync(ST)); const treasury = kp(S.treasury);
  const [Rm, Am] = S.launches.map(l => new web3.PublicKey(l.mint));
  for (const L of S.launches) { const now = Math.floor(Date.now() / 1000); if (now < L.revealEnd) { console.log('reveal open', L.revealEnd - now); process.exit(2); } await send([await R.ixSettle(P(new web3.PublicKey(L.mint)))], [payer]); }
  // claims so people hold tokens
  for (const L of S.launches) for (const sk of L.bidders) { const b = kp(sk); const m = new web3.PublicKey(L.mint); await send([...wrap(b.publicKey, 0n), ataIx(m, b.publicKey), await R.ixClaim({ bidder: b.publicKey, ...P(m) }), ...unwrap(b.publicKey)], [b]); }
  let Rl = await R.fetchLaunch(conn, Rm), Al = await R.fetchLaunch(conn, Am);
  console.log(`settled · RSRCT pool ${sol(Rl.poolPair)} · LAZ pool ${sol(Al.poolPair)} · LAZ creator fee ${Al.creatorFeeBps / 100}%`);
  // point config at RSRCT
  await send([await R.ixSetConfig({ authority: payer.publicKey, treasury: treasury.publicKey, rsrctMint: Rm })], [payer]);
  console.log('set_config by stranger:', await expectFail(async () => { const x = web3.Keypair.generate(); await conn.confirmTransaction(await conn.requestAirdrop(x.publicKey, 1e9)); await send([await R.ixSetConfig({ authority: x.publicKey, treasury: x.publicKey, rsrctMint: Rm })], [x]); }, /has_one|ConstraintHasOne|2001/));
  // --- trading on LAZ: human (verified) buys 1 SOL, nobody (unverified) buys 1 SOL. epoch 0.
  const nobody = kp(S.launches[1].bidders[1]); const ep = R.epochOf(Al);
  const buy = async (who, lam, gw) => send([...wrap(who.publicKey, lam), ataIx(Am, who.publicKey), await R.ixSwap({ trader: who.publicKey, ...P(Am), pairForTokens: true, amountIn: lam, minOut: 1n, epoch: ep, gatewayToken: gw }), ...unwrap(who.publicKey)], [who]);
  await buy(human, 1_000_000_000n, R.gatewayTokenPda(human.publicKey));
  await buy(nobody, 1_000_000_000n, null);
  console.log('swap with wrong epoch:', await expectFail(async () => send([...wrap(nobody.publicKey, 100_000_000n), await R.ixSwap({ trader: nobody.publicKey, ...P(Am), pairForTokens: true, amountIn: 100_000_000n, minOut: 1n, epoch: ep + 1 }), ...unwrap(nobody.publicKey)], [nobody]), /WrongEpoch|epoch/));
  Al = await R.fetchLaunch(conn, Am); const fee = Al.feesAccrued; // 0.02 SOL
  const E = await R.fetchEpoch(conn, Am, ep); const Th = await R.fetchTrader(conn, Am, human.publicKey, ep), Tn = await R.fetchTrader(conn, Am, nobody.publicKey, ep);
  console.log(`LAZ fees ${sol(fee)} → creator ${sol(Al.creatorFeesOwed)} (5%) · buyback ${sol(Al.buybackAccrued)} (25%) · traders ${sol(E.pool)} (20%) · treasury ${sol(Al.treasuryOwed)} (10%) · holders pending ${sol(Al.rewardsPending)} (40% = 35% + creator's unused 5%)`);
  console.log(`  epoch ${ep} verified volume ${sol(E.volume)} · human volume ${sol(Th.volume)} (raw ${sol(Th.rawVolume)}) · nobody volume ${sol(Tn.volume)} (raw ${sol(Tn.rawVolume)})`);
  const sum = Al.creatorFeesOwed + Al.buybackAccrued + E.pool + Al.treasuryOwed + Al.rewardsPending; console.log('  split sums to fee exactly:', sum === fee);
  // treasury
  await send([spl.createAssociatedTokenAccountIdempotentInstruction(payer.publicKey, R.ata(SOL, treasury.publicKey), treasury.publicKey, SOL), await R.ixCollectTreasury({ ...P(Am), treasury: treasury.publicKey })], [payer]);
  console.log('treasury collected:', (await conn.getTokenAccountBalance(R.ata(SOL, treasury.publicKey))).value.uiAmountString, 'wSOL · owed now', (await R.fetchLaunch(conn, Am)).treasuryOwed);
  // buyback & burn RSRCT through the RSRCT pool
  const supplyBefore = (await conn.getTokenSupply(Rm)).value.amount; const rPoolBefore = Rl.poolPair;
  await send([await R.ixBuybackAndBurn({ ...P(Am), rsrctMint: Rm })], [payer]);
  Rl = await R.fetchLaunch(conn, Rm); Al = await R.fetchLaunch(conn, Am); const supplyAfter = (await conn.getTokenSupply(Rm)).value.amount;
  console.log(`buyback: spent ${sol(Al.buybackTotal)} · RSRCT burned ${(Number(Rl.burned) / 1e6).toFixed(3)} · supply ${supplyBefore} → ${supplyAfter} (Δ ${(Number(BigInt(supplyBefore) - BigInt(supplyAfter)) / 1e6).toFixed(3)}) · RSRCT pool ${sol(rPoolBefore)} → ${sol(Rl.poolPair)} · vault==supply? ${(await conn.getTokenAccountBalance(R.seedsFor(Rm).vault)).value.amount}`);
  console.log('second buyback with nothing accrued:', await expectFail(async () => send([await R.ixBuybackAndBurn({ ...P(Am), rsrctMint: Rm })], [payer]), /NothingToClaim|6014/));
  // RSRCT epoch with unverified volume only -> sweep later
  const epR = R.epochOf(Rl); await send([...wrap(nobody.publicKey, 500_000_000n), ataIx(Rm, nobody.publicKey), await R.ixSwap({ trader: nobody.publicKey, ...P(Rm), pairForTokens: true, amountIn: 500_000_000n, minOut: 1n, epoch: epR }), ...unwrap(nobody.publicKey)], [nobody]);
  // trader claim before epoch end must fail
  console.log('trader claim before epoch end:', await expectFail(async () => send([...wrap(human.publicKey, 0n), await R.ixClaimTraderReward({ trader: human.publicKey, ...P(Am), epoch: ep })], [human]), /EpochOpen|not ended/));
  const wait = R.epochEnd(Al, ep) - Math.floor(Date.now() / 1000) + 3; console.log(`waiting ${wait}s for epoch ${ep} to end…`); await new Promise(r => setTimeout(r, Math.max(0, wait) * 1000));
  const hb = await conn.getBalance(human.publicKey);
  await send([...wrap(human.publicKey, 0n), await R.ixClaimTraderReward({ trader: human.publicKey, ...P(Am), epoch: ep }), ...unwrap(human.publicKey)], [human]);
  console.log(`human trader reward ≈ ${((await conn.getBalance(human.publicKey) - hb) / 1e9).toFixed(6)} SOL (expect ≈ whole trader pool ${sol(E.pool)} since only verified volume counts)`);
  console.log('nobody (unverified) trader claim:', await expectFail(async () => send([...wrap(nobody.publicKey, 0n), await R.ixClaimTraderReward({ trader: nobody.publicKey, ...P(Am), epoch: ep })], [nobody]), /NothingToClaim|6014/));
  console.log('human double claim:', await expectFail(async () => send([...wrap(human.publicKey, 0n), await R.ixClaimTraderReward({ trader: human.publicKey, ...P(Am), epoch: ep })], [human]), /NothingToClaim|6014/));
  // sweep RSRCT epoch (pool>0, verified volume 0) -> holders
  const rb = (await R.fetchLaunch(conn, Rm)).rewardsPending; await send([await R.ixSweepEpoch({ mint: Rm, epoch: epR })], [payer]);
  const ra = (await R.fetchLaunch(conn, Rm)).rewardsPending; console.log(`sweep RSRCT epoch ${epR}: holders pending ${sol(rb)} → ${sol(ra)}`);
  console.log('sweep again:', await expectFail(async () => send([await R.ixSweepEpoch({ mint: Rm, epoch: epR })], [payer]), /NothingToClaim|6014/));
  // holders: human stakes LAZ, pending 40% released, claim
  const bal = BigInt((await conn.getTokenAccountBalance(R.ata(Am, human.publicKey))).value.amount);
  await send([await R.ixStake({ staker: human.publicKey, mint: Am, amount: bal })], [human]);
  const st = await R.fetchStake(conn, Am, human.publicKey); Al = await R.fetchLaunch(conn, Am);
  console.log(`human staked all LAZ · pending rewards ${sol(R.pendingRewards(Al, st))} (expect ≈ holders share above)`);
  const hb2 = await conn.getBalance(human.publicKey); await send([...wrap(human.publicKey, 0n), await R.ixClaimRewards({ staker: human.publicKey, ...P(Am) }), ...unwrap(human.publicKey)], [human]);
  console.log(`holder rewards claimed ≈ ${((await conn.getBalance(human.publicKey) - hb2) / 1e9).toFixed(6)} SOL · fee vault left ${sol((await conn.getTokenAccountBalance(R.seedsFor(Am).fees)).value.amount)} (= creator owed ${sol(Al.creatorFeesOwed)} + dust)`);
}
const ph = process.argv[2]; (ph === 'reveal' ? phase2() : ph === 'settle' ? phase3() : phase1()).catch(e => { console.error(e.logs ? e.logs.join('\n') : e); console.error(e.message); process.exit(1); });
