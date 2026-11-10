// Dev buy (localtest build: fixed 20s, cap 45s, decay 70s)
const web3=require('@solana/web3.js'),spl=require('@solana/spl-token'),fs=require('fs');
const R=require('./rsrct.js')(web3,spl,'5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t',{epochSecs:90,openSecs:[20,45,70]});
const conn=new web3.Connection('http://127.0.0.1:8899','confirmed');
const payer=web3.Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync('/home/claude/sol/resurrect-program/keys/payer.json'))));
const send=(ixs,s)=>web3.sendAndConfirmTransaction(conn,new web3.Transaction().add(...ixs),s,{commitment:'confirmed'});
const SOL=spl.NATIVE_MINT,TP=spl.TOKEN_PROGRAM_ID; const P=m=>({mint:m,pairMint:SOL,pairProg:TP});
const wrap=(o,l)=>R.ixsPreparePair({owner:o,pairMint:SOL,prog:TP,lamports:l}).ixs, un=o=>R.ixsUnwrap({owner:o,pairMint:SOL,prog:TP}); const ataIx=(m,o)=>spl.createAssociatedTokenAccountIdempotentInstruction(o,R.ata(m,o),o,m);
const tok=v=>(Number(v)/1e6).toLocaleString('en-US',{maximumFractionDigits:0});
const fail=async(f,re)=>{try{await f();return 'ACCEPTED (BUG)';}catch(e){const t=JSON.stringify(e.logs||e.message);return re.test(t)?'rejected ✓':'rejected(other) '+t.slice(0,200);}};
const chainNow=async()=>Number((await conn.getBlockTime(await conn.getSlot()))||Math.floor(Date.now()/1000));
const buyIx=async(who,m,lam)=>{const l=await R.fetchLaunch(conn,m);const q=R.quoteBuy(l,lam,await chainNow());return [...wrap(who.publicKey,lam),ataIx(m,who.publicKey),await R.ixSwap({trader:who.publicKey,...P(m),pairForTokens:true,amountIn:lam,minOut:q.out*98n/100n,epoch:R.epochOf(l)}),...un(who.publicKey)];};
const buy=async(who,m,lam)=>send(await buyIx(who,m,lam),[who]);
(async()=>{
  // virtual 30 SOL → opening price 3e-8 SOL/token; 20% of supply = 200M tokens = 6 SOL net (6.316 gross at 5% fee)
  const mk=web3.Keypair.generate(); const c=await R.ixsCreateFixedMint({connection:conn,payer:payer.publicKey,mintKp:mk});
  c.ixs.push(await R.ixCreateLaunch({creator:payer.publicKey,...P(mk.publicKey),creatorToken:c.ata,name:'DevBuy',symbol:'DEVB',batchSecs:60,creatorFeeBps:0,mode:1,virtualPair:30_000_000_000n,venue:0}));
  // 1) dev buy in the SAME transaction as create: 10% of supply (3 SOL net → 3.158 gross)
  const m=mk.publicKey; const gross=3_157_900_000n;
  c.ixs.push(...wrap(payer.publicKey,gross),ataIx(m,payer.publicKey),await R.ixSwap({trader:payer.publicKey,...P(m),pairForTokens:true,amountIn:gross,minOut:0n,epoch:0}),...un(payer.publicKey));
  await send(c.ixs,[payer,mk]);
  let l=await R.fetchLaunch(conn,m); const held=BigInt((await conn.getTokenAccountBalance(R.ata(m,payer.publicKey))).value.amount);
  console.log(`create+dev buy in one tx ✓ · dev holds ${tok(held)} tokens (${(Number(held)/1e15*100).toFixed(2)}% of supply) · launch.devBought = ${tok(l.devBought)} · equal: ${held===l.devBought}`);
  // 2) second creator buy in window → rejected
  console.log('second dev buy in window:', await fail(()=>buy(payer,m,100_000_000n),/CreatorCannotBuyYet/));
  // 3) others still capped at 1.5%
  const a=web3.Keypair.generate(); await conn.confirmTransaction(await conn.requestAirdrop(a.publicKey,50e9));
  console.log('other wallet 2% buy:', await fail(()=>buy(a,m,631_600_000n),/OpenCapExceeded/)); await buy(a,m,300_000_000n); console.log('other wallet 1% buy: accepted ✓');
  // 4) dev can sell any time
  l=await R.fetchLaunch(conn,m); const q=R.quoteSell(l,held/2n,await chainNow()); await send([...wrap(payer.publicKey,0n),await R.ixSwap({trader:payer.publicKey,...P(m),pairForTokens:false,amountIn:held/2n,minOut:q.out*98n/100n,epoch:R.epochOf(l)}),...un(payer.publicKey)],[payer]);
  l=await R.fetchLaunch(conn,m); console.log(`dev sold half in window: accepted ✓ · devBought stays ${tok(l.devBought)} (record of the buy, not the balance)`);
  // 5) >20% dev buy on a fresh launch → rejected
  const mk2=web3.Keypair.generate(); const c2=await R.ixsCreateFixedMint({connection:conn,payer:payer.publicKey,mintKp:mk2});
  c2.ixs.push(await R.ixCreateLaunch({creator:payer.publicKey,...P(mk2.publicKey),creatorToken:c2.ata,name:'DevBig',symbol:'DEVX',batchSecs:60,creatorFeeBps:0,mode:1,virtualPair:30_000_000_000n,venue:0}));
  const big=6_400_000_000n; c2.ixs.push(...wrap(payer.publicKey,big),ataIx(mk2.publicKey,payer.publicKey),await R.ixSwap({trader:payer.publicKey,...P(mk2.publicKey),pairForTokens:true,amountIn:big,minOut:0n,epoch:0}),...un(payer.publicKey));
  console.log('dev buy of 20.3%:', await fail(()=>send(c2.ixs,[payer,mk2]),/DevBuyTooLarge/));
  // 6) dev buy after the fixed window (but inside cap window) → rejected
  const mk3=web3.Keypair.generate(); const c3=await R.ixsCreateFixedMint({connection:conn,payer:payer.publicKey,mintKp:mk3});
  c3.ixs.push(await R.ixCreateLaunch({creator:payer.publicKey,...P(mk3.publicKey),creatorToken:c3.ata,name:'DevLate',symbol:'DEVL',batchSecs:60,creatorFeeBps:0,mode:1,virtualPair:30_000_000_000n,venue:0})); await send(c3.ixs,[payer,mk3]);
  const l3=await R.fetchLaunch(conn,mk3.publicKey); const w=l3.startTs+20-await chainNow()+2; if(w>0) await new Promise(r=>setTimeout(r,w*1000));
  console.log('dev buy after fixed window:', await fail(()=>buy(payer,mk3.publicKey,100_000_000n),/CreatorCannotBuyYet/));
})().catch(e=>{console.error(e.logs?e.logs.slice(-8).join('\n'):e);console.error(e.message);process.exit(1)});
