// Auction + Raydium: bid, reveal, settle, then graduate (phase arg: '', 'reveal', 'settle')
const web3=require('@solana/web3.js'),spl=require('@solana/spl-token'),fs=require('fs');
const R=require('./rsrct.js')(web3,spl,'5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t',{epochSecs:90,cluster:'devnet'});
const conn=new web3.Connection('http://127.0.0.1:8899','confirmed'); const ST='/tmp/ray2.json';
const payer=web3.Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync('/home/claude/sol/resurrect-program/keys/payer.json'))));
const send=async(ixs,s,cu)=>{const tx=new web3.Transaction(); if(cu) tx.add(web3.ComputeBudgetProgram.setComputeUnitLimit({units:cu})); tx.add(...ixs); return web3.sendAndConfirmTransaction(conn,tx,s,{commitment:'confirmed'});};
const SOL=spl.NATIVE_MINT,TP=spl.TOKEN_PROGRAM_ID; const P=m=>({mint:m,pairMint:SOL,pairProg:TP});
const wrap=(o,l)=>R.ixsPreparePair({owner:o,pairMint:SOL,prog:TP,lamports:l}).ixs, un=o=>R.ixsUnwrap({owner:o,pairMint:SOL,prog:TP}); const ataIx=(m,o)=>spl.createAssociatedTokenAccountIdempotentInstruction(o,R.ata(m,o),o,m);
const sol=v=>(Number(v)/1e9).toFixed(4)+' SOL'; const fail=async(f,re)=>{try{await f();return 'ACCEPTED (BUG)';}catch(e){const t=JSON.stringify(e.logs||e.message);return re.test(t)?'rejected ✓':'rejected(other) '+t.slice(0,200);}};
const ph=process.argv[2];
(async()=>{
  if(!ph){ const mk=web3.Keypair.generate(); const {ata,ixs}=await R.ixsCreateFixedMint({connection:conn,payer:payer.publicKey,mintKp:mk});
    ixs.push(await R.ixCreateLaunch({creator:payer.publicKey,...P(mk.publicKey),creatorToken:ata,name:'Auction Ray',symbol:'ARAY',batchSecs:60,creatorFeeBps:500,mode:0,venue:1})); await send(ixs,[payer,mk]);
    const b=web3.Keypair.generate(); await conn.confirmTransaction(await conn.requestAirdrop(b.publicKey,10e9)); const salt=R.randomSalt(); const amt=3_000_000_000n;
    await send([...wrap(b.publicKey,amt),await R.ixCommitBid({bidder:b.publicKey,...P(mk.publicKey),amount:amt,salt,deposit:amt}),...un(b.publicKey)],[b]);
    const l=await R.fetchLaunch(conn,mk.publicKey); fs.writeFileSync(ST,JSON.stringify({mint:mk.publicKey.toBase58(),b:Array.from(b.secretKey),salt:Array.from(salt),amt:amt.toString(),endTs:l.endTs,revealEnd:l.revealEndTs}));
    console.log('AUCTION+RAYDIUM launch created, 1 sealed bid of 3 SOL'); return; }
  const S=JSON.parse(fs.readFileSync(ST)); const mint=new web3.PublicKey(S.mint); const b=web3.Keypair.fromSecretKey(Uint8Array.from(S.b)); const now=Math.floor(Date.now()/1000);
  if(ph==='reveal'){ if(now<S.endTs){console.log('wait',S.endTs-now);process.exit(2);} await send([await R.ixRevealBid({bidder:b.publicKey,mint,amount:BigInt(S.amt),salt:Uint8Array.from(S.salt)})],[b]); console.log('revealed'); return; }
  if(now<S.revealEnd){console.log('wait',S.revealEnd-now);process.exit(2);}
  await send([await R.ixSettle(P(mint))],[payer]); let l=await R.fetchLaunch(conn,mint);
  console.log(`settled · pool ${sol(l.poolPair)} + ${Number(l.poolTokens)/1e6} tokens (price continuity: ${(Number(l.poolPair)/Number(l.poolTokens)).toExponential(3)} vs clearing ${(R.priceFromQ32(l.clearingPriceQ32)).toExponential(3)}) · canGraduate ${R.canGraduate(l)}`);
  console.log('swap before graduation:', await fail(async()=>send([...wrap(b.publicKey,100_000_000n),ataIx(mint,b.publicKey),await R.ixSwap({trader:b.publicKey,...P(mint),pairForTokens:true,amountIn:100_000_000n,minOut:1n,epoch:0}),...un(b.publicKey)],[b]),/GraduatePending/));
  const g=await R.ixsGraduateRaydium({payer:payer.publicKey,launch:l}); await send([...g.pre,g.ix],[payer],600000); l=await R.fetchLaunch(conn,mint);
  const lp=await conn.getTokenSupply(g.lpMint); const r=R.raydiumPdas(...(SOL.toBuffer().compare(mint.toBuffer())<0?[SOL,mint]:[mint,SOL]));
  console.log(`graduated ✓ pool ${l.raydiumPool.toBase58().slice(0,8)} · LP supply ${lp.value.uiAmountString} · vaults ${(await conn.getTokenAccountBalance(r.vault0)).value.uiAmountString} / ${(await conn.getTokenAccountBalance(r.vault1)).value.uiAmountString}`);
  // bidder can still claim their tokens from our vault (800M sold side stays with us)
  await send([...wrap(b.publicKey,0n),ataIx(mint,b.publicKey),await R.ixClaim({bidder:b.publicKey,...P(mint)}),...un(b.publicKey)],[b]);
  console.log('bidder claimed after graduation:', (await conn.getTokenAccountBalance(R.ata(mint,b.publicKey))).value.uiAmountString, 'tokens · vault left', (await conn.getTokenAccountBalance(R.seedsFor(mint).vault)).value.uiAmountString);
})().catch(e=>{console.error(e.logs?e.logs.join('\n'):e);console.error(e.message);process.exit(1)});
