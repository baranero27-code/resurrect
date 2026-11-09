// Open-mode protections (localtest build: fixed 20s, cap 45s, decay 70s)
const web3=require('@solana/web3.js'),spl=require('@solana/spl-token'),fs=require('fs');
const R=require('./rsrct.js')(web3,spl,'5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t',{epochSecs:90,openSecs:[20,45,70]});
const conn=new web3.Connection('http://127.0.0.1:8899','confirmed');
const payer=web3.Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync('/home/claude/sol/resurrect-program/keys/payer.json'))));
const send=(ixs,s)=>web3.sendAndConfirmTransaction(conn,new web3.Transaction().add(...ixs),s,{commitment:'confirmed'});
const SOL=spl.NATIVE_MINT,TP=spl.TOKEN_PROGRAM_ID; const P=m=>({mint:m,pairMint:SOL,pairProg:TP});
const wrap=(o,l)=>R.ixsPreparePair({owner:o,pairMint:SOL,prog:TP,lamports:l}).ixs, un=o=>R.ixsUnwrap({owner:o,pairMint:SOL,prog:TP}); const ataIx=(m,o)=>spl.createAssociatedTokenAccountIdempotentInstruction(o,R.ata(m,o),o,m);
const sol=v=>(Number(v)/1e9).toFixed(4); const tok=v=>(Number(v)/1e6).toLocaleString('en-US',{maximumFractionDigits:0});
const fail=async(f,re)=>{try{await f();return 'ACCEPTED (BUG)';}catch(e){const t=JSON.stringify(e.logs||e.message);return re.test(t)?'rejected ✓':'rejected(other) '+t.slice(0,160);}};
const chainNow=async()=>Number((await conn.getBlockTime(await conn.getSlot()))||Math.floor(Date.now()/1000));
const buy=async(who,m,lam)=>{const l=await R.fetchLaunch(conn,m);const q=R.quoteBuy(l,lam,await chainNow());await send([...wrap(who.publicKey,lam),ataIx(m,who.publicKey),await R.ixSwap({trader:who.publicKey,...P(m),pairForTokens:true,amountIn:lam,minOut:q.out*98n/100n,epoch:R.epochOf(l)}),...un(who.publicKey)],[who]);return q;};
(async()=>{
  const mk=web3.Keypair.generate(); const c=await R.ixsCreateFixedMint({connection:conn,payer:payer.publicKey,mintKp:mk});
  c.ixs.push(await R.ixCreateLaunch({creator:payer.publicKey,...P(mk.publicKey),creatorToken:c.ata,name:'Protected',symbol:'PROT',batchSecs:60,creatorFeeBps:500,mode:1,virtualPair:30_000_000_000n,venue:0})); await send(c.ixs,[payer,mk]);
  const m=mk.publicKey; let l=await R.fetchLaunch(conn,m); const p0=Number(l.virtualPair)/1e15*1e6/1e9; // SOL per whole token console.log(`OPEN launch · opening price ${p0.toExponential(3)} SOL/token · fee now ${R.openFeeBps(l,await chainNow())} bps`);
  const a=web3.Keypair.generate(), b=web3.Keypair.generate(); for(const k of [a,b]) await conn.confirmTransaction(await conn.requestAirdrop(k.publicKey,50e9));
  // 1) creator cannot buy in the window
  console.log('creator buy in window:', await fail(()=>buy(payer,m,100_000_000n),/CreatorCannotBuyYet/));
  // 2) fixed price: two buyers, different sizes, same price per token
  const before=BigInt((await conn.getTokenAccountBalance(R.ata(m,a.publicKey)).catch(()=>({value:{amount:'0'}}))).value.amount);
  await buy(a,m,300_000_000n); const ga=BigInt((await conn.getTokenAccountBalance(R.ata(m,a.publicKey))).value.amount);
  await buy(b,m,400_000_000n); const gb=BigInt((await conn.getTokenAccountBalance(R.ata(m,b.publicKey))).value.amount);
  const pa=0.3*0.95/(Number(ga)/1e6), pb=0.4*0.95/(Number(gb)/1e6); console.log(`fixed window: A paid ${pa.toExponential(4)} /token, B paid ${pb.toExponential(4)} /token (fee 5%) · equal: ${Math.abs(pa-pb)/pa<1e-6} · = opening price: ${Math.abs(pa-p0)/p0<1e-6}`);
  // 3) sell in window at opening price (minus fee), no arbitrage
  l=await R.fetchLaunch(conn,m); const qs=R.quoteSell(l,ga,await chainNow()); await send([...wrap(a.publicKey,0n),await R.ixSwap({trader:a.publicKey,...P(m),pairForTokens:false,amountIn:ga,minOut:qs.out*98n/100n,epoch:0}),...un(a.publicKey)],[a]);
  console.log(`A sold all in window: got back ≈ ${sol(qs.out)} SOL of 0.3 SOL (fee both ways, no price gain) ✓`);
  // 4) cap: 1.5% of supply = 15M tokens. At p0 (3e-8) that's 0.45 SOL net; B already holds ${tok(gb)}; a 0.5 SOL buy pushes over
  console.log(`cap check: B holds ${tok(gb)} tokens; cap 15,000,000 ·`, 'buy that exceeds cap (0.4+0.1 SOL > 0.45):', await fail(()=>buy(b,m,100_000_000n),/OpenCapExceeded/));
  // 5) after fixed window: CP pricing, still capped
  const w1=l.startTs+20-await chainNow()+2; if(w1>0) await new Promise(r=>setTimeout(r,w1*1000));
  l=await R.fetchLaunch(conn,m); const q1=R.quoteBuy(l,100_000_000n,await chainNow()); await buy(a,m,100_000_000n); const ga2=BigInt((await conn.getTokenAccountBalance(R.ata(m,a.publicKey))).value.amount);
  console.log(`after fixed window: 0.1 SOL buy → ${tok(ga2)} tokens (quote ${tok(q1.out)}) · spot now ${(Number(l.poolPair)/Number(l.poolTokens)*1e6/1e9).toExponential(3)} vs p0 ${p0.toExponential(3)} · fee ${R.openFeeBps(l,await chainNow())} bps`);
  // 6) after cap window: big buy allowed; creator can buy
  const w2=l.startTs+45-await chainNow()+2; if(w2>0) await new Promise(r=>setTimeout(r,w2*1000));
  await buy(b,m,5_000_000_000n); console.log('after cap window: 5 SOL buy accepted ✓ · creator buy:', await (async()=>{try{await buy(payer,m,100_000_000n);return 'accepted ✓';}catch(e){return 'REJECTED (BUG)';}})());
  // 7) fee decays to 1%
  const w3=l.startTs+70-await chainNow()+2; if(w3>0) await new Promise(r=>setTimeout(r,w3*1000));
  l=await R.fetchLaunch(conn,m); console.log(`after decay: fee ${R.openFeeBps(l,await chainNow())} bps · fees accrued ${sol(l.feesAccrued)} SOL · holders pending ${sol(l.rewardsPending)} (includes the surcharge) · creator owed ${sol(l.creatorFeesOwed)}`);
})().catch(e=>{console.error(e.logs?e.logs.slice(-6).join('\n'):e);console.error(e.message);process.exit(1)});
