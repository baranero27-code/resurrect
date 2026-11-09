// Open + Raydium: buy past the graduation threshold, graduate, verify Raydium pool + burned LP
const web3=require('@solana/web3.js'),spl=require('@solana/spl-token'),fs=require('fs');
const R=require('./rsrct.js')(web3,spl,'5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t',{epochSecs:90,cluster:'devnet'});
const conn=new web3.Connection('http://127.0.0.1:8899','confirmed');
const payer=web3.Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync('/home/claude/sol/resurrect-program/keys/payer.json'))));
const send=async(ixs,s,cu)=>{const tx=new web3.Transaction(); if(cu) tx.add(web3.ComputeBudgetProgram.setComputeUnitLimit({units:cu})); tx.add(...ixs); return web3.sendAndConfirmTransaction(conn,tx,s,{commitment:'confirmed'});};
const SOL=spl.NATIVE_MINT,TP=spl.TOKEN_PROGRAM_ID; const P=m=>({mint:m,pairMint:SOL,pairProg:TP});
const wrap=(o,l)=>R.ixsPreparePair({owner:o,pairMint:SOL,prog:TP,lamports:l}).ixs, un=o=>R.ixsUnwrap({owner:o,pairMint:SOL,prog:TP}); const ataIx=(m,o)=>spl.createAssociatedTokenAccountIdempotentInstruction(o,R.ata(m,o),o,m);
const sol=v=>(Number(v)/1e9).toFixed(4)+' SOL'; const fail=async(f,re)=>{try{await f();return 'ACCEPTED (BUG)';}catch(e){const t=JSON.stringify(e.logs||e.message);return re.test(t)?'rejected ✓':'rejected(other) '+t.slice(0,300);}};
(async()=>{
  const mk=web3.Keypair.generate(); const {ata,ixs}=await R.ixsCreateFixedMint({connection:conn,payer:payer.publicKey,mintKp:mk});
  ixs.push(await R.ixCreateLaunch({creator:payer.publicKey,...P(mk.publicKey),creatorToken:ata,name:'Ray Coin',symbol:'RAY1',batchSecs:60,creatorFeeBps:500,mode:1,virtualPair:10_000_000_000n,venue:1}));
  await send(ixs,[payer,mk]); let l=await R.fetchLaunch(conn,mk.publicKey);
  console.log(`OPEN+RAYDIUM launch · venue ${l.venue} · virtual ${sol(l.virtualPair)} · canGraduate now: ${R.canGraduate(l)}`);
  console.log('graduate too early:', await fail(async()=>{const g=await R.ixsGraduateRaydium({payer:payer.publicKey,launch:l}); await send([...g.pre,g.ix],[payer],400000);},/NotReadyToGraduate/));
  // buyer pushes real reserve past 10 SOL
  const t=web3.Keypair.generate(); await conn.confirmTransaction(await conn.requestAirdrop(t.publicKey,30e9)); const ep=R.epochOf(l);
  const buy=10_300_000_000n; await send([...wrap(t.publicKey,buy),ataIx(mk.publicKey,t.publicKey),await R.ixSwap({trader:t.publicKey,...P(mk.publicKey),pairForTokens:true,amountIn:buy,minOut:1n,epoch:ep}),...un(t.publicKey)],[t]);
  l=await R.fetchLaunch(conn,mk.publicKey); console.log(`after 10.3 SOL buy · real reserve ${sol(l.poolPair-l.virtualPair)} · tokens left ${Number(l.poolTokens)/1e6} · canGraduate ${R.canGraduate(l)}`);
  const g=await R.ixsGraduateRaydium({payer:payer.publicKey,launch:l}); const pb=await conn.getBalance(payer.publicKey);
  const sig=await send([...g.pre,g.ix],[payer],600000); console.log('graduated ✓', sig.slice(0,20), '· payer paid', ((pb-await conn.getBalance(payer.publicKey))/1e9).toFixed(3),'SOL (Raydium fee 1 SOL on devnet + rents)');
  l=await R.fetchLaunch(conn,mk.publicKey); const r=R.raydiumPdas(...(SOL.toBuffer().compare(mk.publicKey.toBuffer())<0?[SOL,mk.publicKey]:[mk.publicKey,SOL]));
  const ps=await conn.getAccountInfo(g.pool); const lp=await conn.getTokenSupply(g.lpMint); const v0=await conn.getTokenAccountBalance(r.vault0), v1=await conn.getTokenAccountBalance(r.vault1);
  console.log(`launch.graduated ${l.graduated} · raydium pool ${l.raydiumPool.toBase58().slice(0,8)} exists ${!!ps} owner ${ps?.owner.toBase58().slice(0,8)} · LP supply after burn ${lp.value.uiAmountString} · vaults ${v0.value.uiAmountString} / ${v1.value.uiAmountString}`);
  console.log(`our pool account ${sol((await conn.getTokenAccountBalance(R.seedsFor(mk.publicKey).pool)).value.amount)} · our vault ${(await conn.getTokenAccountBalance(R.seedsFor(mk.publicKey).vault)).value.uiAmountString} tokens · payer kept extra tokens? ${(await conn.getTokenAccountBalance(R.ata(mk.publicKey,payer.publicKey))).value.uiAmountString}`);
  console.log('swap after graduation:', await fail(async()=>send([...wrap(t.publicKey,100_000_000n),await R.ixSwap({trader:t.publicKey,...P(mk.publicKey),pairForTokens:true,amountIn:100_000_000n,minOut:1n,epoch:ep}),...un(t.publicKey)],[t]),/Graduated/));
  console.log('graduate twice:', await fail(async()=>{const g2=await R.ixsGraduateRaydium({payer:payer.publicKey,launch:l}); await send([...g2.pre,g2.ix],[payer],600000);},/BadParams|6000/));
  // Resurrect-venue launch must refuse graduation
  const mk2=web3.Keypair.generate(); const c2=await R.ixsCreateFixedMint({connection:conn,payer:payer.publicKey,mintKp:mk2}); c2.ixs.push(await R.ixCreateLaunch({creator:payer.publicKey,...P(mk2.publicKey),creatorToken:c2.ata,name:'Stay',symbol:'STAY',batchSecs:60,creatorFeeBps:0,mode:1,virtualPair:10_000_000_000n,venue:0})); await send(c2.ixs,[payer,mk2]);
  const l2=await R.fetchLaunch(conn,mk2.publicKey); console.log('graduate a Resurrect-venue launch:', await fail(async()=>{const g3=await R.ixsGraduateRaydium({payer:payer.publicKey,launch:l2}); await send([...g3.pre,g3.ix],[payer],600000);},/BadParams|6000/));
})().catch(e=>{console.error(e.logs?e.logs.join('\n'):e);console.error(e.message);process.exit(1)});
