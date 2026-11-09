const web3=require('@solana/web3.js'),spl=require('@solana/spl-token'),fs=require('fs');
const R=require('./rsrct.js')(web3,spl,'5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t',{epochSecs:90});
const conn=new web3.Connection('http://127.0.0.1:8899','confirmed');
const payer=web3.Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync('/home/claude/sol/resurrect-program/keys/payer.json'))));
const send=(ixs,s)=>web3.sendAndConfirmTransaction(conn,new web3.Transaction().add(...ixs),s,{commitment:'confirmed'});
const SOL=spl.NATIVE_MINT,TP=spl.TOKEN_PROGRAM_ID; const P=m=>({mint:m,pairMint:SOL,pairProg:TP}); const me=payer.publicKey;
const wrap=l=>R.ixsPreparePair({owner:me,pairMint:SOL,prog:TP,lamports:l}).ixs, un=()=>R.ixsUnwrap({owner:me,pairMint:SOL,prog:TP}); const ataIx=m=>spl.createAssociatedTokenAccountIdempotentInstruction(me,R.ata(m,me),me,m);
const sol=v=>(Number(v)/1e9).toFixed(6);
(async()=>{
  if(!(await R.fetchConfig(conn))) await send([await R.ixInitConfig({admin:me,treasury:me})],[payer]);
  const mkR=web3.Keypair.generate(); const c=await R.ixsCreateFixedMint({connection:conn,payer:me,mintKp:mkR}); c.ixs.push(await R.ixCreateLaunch({creator:me,...P(mkR.publicKey),creatorToken:c.ata,name:'R',symbol:'RSRCT',batchSecs:60,creatorFeeBps:0,mode:1,virtualPair:30_000_000_000n})); await send(c.ixs,[payer,mkR]);
  await send([await R.ixSetConfig({authority:me,treasury:me,rsrctMint:mkR.publicKey})],[payer]);
  // RSRCT real reserve: buy 1 SOL so real ≈ 0.99 → cap = 0.0099 SOL
  await send([...wrap(1_000_000_000n),ataIx(mkR.publicKey,me),await R.ixSwap({trader:me,...P(mkR.publicKey),pairForTokens:true,amountIn:1_000_000_000n,minOut:1n,epoch:0}),...un()],[payer]);
  const mkA=web3.Keypair.generate(); const c2=await R.ixsCreateFixedMint({connection:conn,payer:me,mintKp:mkA}); c2.ixs.push(await R.ixCreateLaunch({creator:me,...P(mkA.publicKey),creatorToken:c2.ata,name:'A',symbol:'AAA',batchSecs:60,creatorFeeBps:0,mode:1,virtualPair:30_000_000_000n})); await send(c2.ixs,[payer,mkA]);
  // big buy on A: 20 SOL → fee 0.2 → buyback accrued 0.05 SOL > cap 0.0099
  await send([...wrap(20_000_000_000n),ataIx(mkA.publicKey,me),await R.ixSwap({trader:me,...P(mkA.publicKey),pairForTokens:true,amountIn:20_000_000_000n,minOut:1n,epoch:0}),...un()],[payer]);
  let A=await R.fetchLaunch(conn,mkA.publicKey), Rl=await R.fetchLaunch(conn,mkR.publicKey); const real=Rl.poolPair-Rl.virtualPair;
  console.log(`accrued ${sol(A.buybackAccrued)} · RSRCT real reserve ${sol(real)} · cap(1%) ${sol(real/100n)}`);
  await send([await R.ixBuybackAndBurn({...P(mkA.publicKey),rsrctMint:mkR.publicKey})],[payer]); const A2=await R.fetchLaunch(conn,mkA.publicKey);
  console.log(`one call spent ${sol(A.buybackAccrued-A2.buybackAccrued)} (≈cap) · still accrued ${sol(A2.buybackAccrued)} · burned ${(Number((await R.fetchLaunch(conn,mkR.publicKey)).burned)/1e6).toFixed(2)} RSRCT`);
  for(let i=0;i<4;i++) await send([await R.ixBuybackAndBurn({...P(mkA.publicKey),rsrctMint:mkR.publicKey})],[payer]);
  console.log(`after 4 more calls accrued ${sol((await R.fetchLaunch(conn,mkA.publicKey)).buybackAccrued)}`);
})().catch(e=>{console.error(e.logs?e.logs.slice(-5).join('\n'):e);process.exit(1)});
