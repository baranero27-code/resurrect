const web3=require('@solana/web3.js'),spl=require('@solana/spl-token'),fs=require('fs');
const R=require('./rsrct.js')(web3,spl,'5iHSXX9YQNYyYACuf2csorP7CLovzQD6nGvDu5G9VM7t');
const conn=new web3.Connection('http://127.0.0.1:8899','confirmed');
const payer=web3.Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync('/home/claude/sol/resurrect-program/keys/payer.json'))));
const send=(ixs,s)=>web3.sendAndConfirmTransaction(conn,new web3.Transaction().add(...ixs),s,{commitment:'confirmed'});
(async()=>{
  // Token-2022 mint WITH transfer fee
  const m=web3.Keypair.generate(); const exts=[spl.ExtensionType.TransferFeeConfig]; const len=spl.getMintLen(exts); const lam=await conn.getMinimumBalanceForRentExemption(len);
  await send([web3.SystemProgram.createAccount({fromPubkey:payer.publicKey,newAccountPubkey:m.publicKey,space:len,lamports:lam,programId:spl.TOKEN_2022_PROGRAM_ID}),
    spl.createInitializeTransferFeeConfigInstruction(m.publicKey,payer.publicKey,payer.publicKey,100,1_000_000n,spl.TOKEN_2022_PROGRAM_ID),
    spl.createInitializeMint2Instruction(m.publicKey,6,payer.publicKey,null,spl.TOKEN_2022_PROGRAM_ID)],[payer,m]);
  const info=await R.pairInfo(conn,m.publicKey); const mk=web3.Keypair.generate(); const {ata,ixs}=await R.ixsCreateFixedMint({connection:conn,payer:payer.publicKey,mintKp:mk});
  ixs.push(await R.ixCreateLaunch({creator:payer.publicKey,mint:mk.publicKey,pairMint:m.publicKey,pairProg:info.prog,creatorToken:ata,name:'FeeTest',symbol:'FEE',batchSecs:60,creatorFeeBps:0}));
  try{ await send(ixs,[payer,mk]); console.log('transfer-fee pair ACCEPTED (BUG)'); }catch(e){ console.log('transfer-fee pair rejected:', /UnsupportedPairExtension|6019|not supported/.test(JSON.stringify(e.logs||e.message))?'UnsupportedPairExtension ✓':'other: '+(e.message||'').slice(0,100)); }
  // Token-2022 mint with metadata pointer only -> must be accepted
  const m2=web3.Keypair.generate(); const exts2=[spl.ExtensionType.MetadataPointer]; const len2=spl.getMintLen(exts2); const lam2=await conn.getMinimumBalanceForRentExemption(len2);
  await send([web3.SystemProgram.createAccount({fromPubkey:payer.publicKey,newAccountPubkey:m2.publicKey,space:len2,lamports:lam2,programId:spl.TOKEN_2022_PROGRAM_ID}),
    spl.createInitializeMetadataPointerInstruction(m2.publicKey,payer.publicKey,m2.publicKey,spl.TOKEN_2022_PROGRAM_ID),
    spl.createInitializeMint2Instruction(m2.publicKey,8,payer.publicKey,null,spl.TOKEN_2022_PROGRAM_ID)],[payer,m2]);
  const info2=await R.pairInfo(conn,m2.publicKey); const mk2=web3.Keypair.generate(); const c2=await R.ixsCreateFixedMint({connection:conn,payer:payer.publicKey,mintKp:mk2});
  c2.ixs.push(await R.ixCreateLaunch({creator:payer.publicKey,mint:mk2.publicKey,pairMint:m2.publicKey,pairProg:info2.prog,creatorToken:c2.ata,name:'MetaTest',symbol:'META2',batchSecs:60,creatorFeeBps:0}));
  await send(c2.ixs,[payer,mk2]); console.log('metadata-pointer pair accepted ✓');
  // commit with delta check on plain mint
  const b=web3.Keypair.generate(); await conn.confirmTransaction(await conn.requestAirdrop(b.publicKey,2e9));
  const bata=spl.getAssociatedTokenAddressSync(m2.publicKey,b.publicKey,false,spl.TOKEN_2022_PROGRAM_ID);
  await send([spl.createAssociatedTokenAccountIdempotentInstruction(b.publicKey,bata,b.publicKey,m2.publicKey,spl.TOKEN_2022_PROGRAM_ID),spl.createMintToInstruction(m2.publicKey,bata,payer.publicKey,500_000_000n,[],spl.TOKEN_2022_PROGRAM_ID),
    await R.ixCommitBid({bidder:b.publicKey,mint:mk2.publicKey,pairMint:m2.publicKey,pairProg:info2.prog,amount:100_000_000n,salt:R.randomSalt(),deposit:200_000_000n})],[b,payer]);
  console.log('commit with balance-delta check ✓ escrow', (await conn.getTokenAccountBalance(R.seedsFor(mk2.publicKey).escrow)).value.uiAmountString);
})().catch(e=>{console.error(e.message);process.exit(1)});
