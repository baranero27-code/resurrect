// Resurrect side rails — PULSE (left) and LIVE TAPE (right). Everything shown is read from the program; nothing is curated or paid for.
// Usage: <script src="js/rails.js"></script> after the bundle, then Rails.mount({monitor:1280, rpc, cluster, programId, app:'app.html'}).
(function(){
  const CSS=`
.railwrap{display:grid;grid-template-columns:minmax(0,1fr);gap:16px;width:100%;align-items:start}
.railwrap>.monitor{max-width:none!important;width:100%;margin:0!important;min-width:0}
.rail{display:none;position:sticky;top:16px;flex-direction:column;gap:10px;font-size:12px;max-height:calc(100vh - 32px);overflow:hidden;font-family:var(--mono,'Share Tech Mono',ui-monospace,monospace)}
@media(min-width:1400px){.railwrap{grid-template-columns:clamp(240px,17vw,380px) minmax(0,1fr) clamp(240px,17vw,380px)}.rail{display:flex}}
.rail .box{background:var(--card,#0A161D);border:1px solid var(--line,rgba(207,230,216,.12));padding:12px 14px}
.rail h5{font-weight:400;font-size:11px;letter-spacing:.14em;color:var(--green,#4AE38A);margin:0 0 8px;display:flex;justify-content:space-between;gap:8px}
.rail h5 i{font-style:normal;color:var(--dim,#5F7A6C);letter-spacing:0;text-align:right}
.rail .kv{display:flex;justify-content:space-between;gap:10px;padding:5px 0;border-bottom:1px dashed var(--line,rgba(207,230,216,.12))}.rail .kv:last-child{border-bottom:0}.rail .kv span{color:var(--dim,#5F7A6C)}.rail .kv b{font-weight:400;color:var(--text,#CFE6D8);text-align:right}
.rail .kv.go{cursor:pointer}.rail .kv.go:hover b{color:var(--green,#4AE38A)}
.tape{display:flex;flex-direction:column;gap:6px;max-height:calc(100vh - 120px);overflow:hidden}
.tape .ev{border-left:2px solid var(--line,rgba(207,230,216,.12));padding:6px 10px;background:rgba(0,0,0,.25);cursor:pointer;animation:evin .4s ease-out}
.tape .ev.buy{border-left-color:var(--green,#4AE38A)}.tape .ev.sell{border-left-color:var(--red,#FF4747)}.tape .ev.new{border-left-color:var(--amber,#F2B134)}.tape .ev.burn,.tape .ev.grad{border-left-color:var(--cyan,#5BD3E6)}
.tape .ev b{display:block;font-weight:400;color:var(--text,#CFE6D8)}.tape .ev span{color:var(--dim,#5F7A6C);font-size:11px}.tape .ev a{color:var(--green,#4AE38A);text-decoration:none}.tape .ev:hover{background:rgba(74,227,138,.06)}
.tape .ev .a{color:var(--amber,#F2B134)}
@keyframes evin{from{opacity:0;transform:translateX(12px)}to{opacity:1;transform:none}}
`;
  const HTML=`<aside class="rail l" aria-label="pulse">
  <div class="box"><h5>PULSE <i id="pl-when">reading…</i></h5><div id="pl-stats"><div class="kv"><span>loading</span><b>—</b></div></div></div>
  <div class="box"><h5>TRENDING <i>by pool depth</i></h5><div id="pl-trend"></div></div>
  <div class="box"><h5>THIS EPOCH <i id="pl-epoch"></i></h5><div class="kv"><span>trader rewards</span><b>20% of every fee · claim after close</b></div><div class="kv"><span>holder rewards</span><b>35% + unused creator share</b></div></div>
</aside>
<aside class="rail r" aria-label="live tape">
  <div class="box"><h5>LIVE TAPE <i id="tp-when">reading the program…</i></h5><div class="tape" id="tape"><div class="ev"><b>waiting for the first event</b><span>every swap, launch, graduation and burn appears here, read from the chain — nothing is curated, nothing is paid for</span></div></div></div>
</aside>`;
  const KNOWN={'So11111111111111111111111111111111111111112':'SOL','4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU':'USDC','EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v':'USDC','Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB':'USDT'};
  const short=k=>k.slice(0,4)+'…'+k.slice(-4), mmss=s=>{s=Math.max(0,Math.floor(s));return String(Math.floor(s/60)).padStart(2,'0')+':'+String(s%60).padStart(2,'0');};
  const fmtTok=b=>(Number(b)/1e6).toLocaleString('en-US',{maximumFractionDigits:2});
  const b64=s=>{const bin=atob(s);const u=new Uint8Array(bin.length);for(let i=0;i<bin.length;i++)u[i]=bin.charCodeAt(i);return u;};
  const disc=async n=>new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode('event:'+n))).slice(0,8);
  const eq8=(u,d)=>d.every((x,j)=>u[j]===x);
  const now=()=>Math.floor(Date.now()/1000);

  async function mount(o){
    if(!window.solanaWeb3||!window.makeRsrct) return;
    const minW=1400;
    const st=document.createElement('style'); st.textContent=CSS; document.head.appendChild(st);
    const mon=document.querySelector(o.monitorSel||'.monitor'); if(!mon) return;
    const tmp=document.createElement('div'); tmp.innerHTML=HTML; const [L,Rr]=tmp.querySelectorAll('aside');
    const wrap=document.createElement('div'); wrap.className='railwrap'; mon.parentNode.insertBefore(wrap,mon); wrap.appendChild(L); wrap.appendChild(mon); wrap.appendChild(Rr);
    const $=s=>document.querySelector(s);
    const {Connection,PublicKey}=solanaWeb3; const conn=new Connection(o.rpc,'confirmed'); const R=makeRsrct(solanaWeb3,splToken,o.programId,{});
    const cluster=o.cluster||'devnet'; const explorer=(k,type='address')=>`https://explorer.solana.com/${type}/${k}`+(cluster==='mainnet'?'':'?cluster=devnet');
    const pairSym=l=>KNOWN[l.pairMint.toBase58()]||('pair '+short(l.pairMint.toBase58()));
    const fmtPair=(l,v)=>(Number(v)/10**l.pairDecimals).toLocaleString('en-US',{maximumFractionDigits:4})+' '+pairSym(l);
    const phaseOf=l=>!l.settled?(now()<l.endTs?'batch':now()<l.revealEndTs?'reveal':'settle'):(l.graduated?'raydium':l.poolTokens>0n?'live':'empty');
    const open=l=>{ if(window.__openCoin) window.__openCoin(l.mint.toBase58()); else location.href=(o.app||'app.html')+'#coin='+l.mint.toBase58(); };
    let launches=[]; const TAPE={}; let D=null, busy=false;
    async function pulse(){ try{ const raw=await R.fetchLaunches(conn); launches=raw.filter(l=>l._size>=(o.launchSize||643)); }catch(e){ $('#pl-when').textContent='RPC busy'; return; }
      if(!launches.length){ $('#pl-stats').innerHTML='<div class="kv"><span>no launches yet</span><b>—</b></div>'; return; }
      const live=launches.filter(l=>phaseOf(l)==='live').length, batch=launches.filter(l=>phaseOf(l)==='batch').length, grads=launches.filter(l=>l.graduated).length;
      const by={}; const add=(k,l,v)=>{const P=pairSym(l); by[k]=by[k]||{}; by[k][P]=(by[k][P]||0n)+v;};
      for(const l of launches){ add('fees',l,l.feesAccrued); add('pool',l,l.poolPair-l.virtualPair); add('holders',l,l.rewardsDistributed); }
      const fm=k=>Object.entries(by[k]||{}).filter(([,v])=>v>0n).map(([P,v])=>fmtPair(launches.find(x=>pairSym(x)===P),v)).join(' · ')||'0';
      const burned=launches.reduce((a,l)=>a+l.burned,0n);
      $('#pl-stats').innerHTML=[['coins trading',live+(batch?' · '+batch+' in sealed batch':'')],['graduated to Raydium',grads+''],['real liquidity in pools',fm('pool')],['fees accrued · 1% of swaps',fm('fees')],['paid to holders',fm('holders')],['RSRCT burned',fmtTok(burned)+' tokens']].map(r=>`<div class="kv"><span>${r[0]}</span><b>${r[1]}</b></div>`).join('');
      $('#pl-when').textContent='from the program · '+new Date().toLocaleTimeString('en-GB');
      const tr=[...launches].filter(l=>l.settled&&!l.graduated).sort((a,b)=>Number((b.poolPair-b.virtualPair)-(a.poolPair-a.virtualPair))).slice(0,4);
      $('#pl-trend').innerHTML=tr.map((l,i)=>`<div class="kv go" data-i="${launches.indexOf(l)}"><span>${i+1} · $${l.symbol}${l.devBought>0n?' · dev '+(Number(l.devBought)/1e13).toFixed(1)+'%':''}</span><b>${fmtPair(l,l.poolPair-l.virtualPair)}</b></div>`).join('')||'<div class="kv"><span>no live pools yet</span><b>—</b></div>';
      $('#pl-trend').querySelectorAll('[data-i]').forEach(e=>e.addEventListener('click',()=>open(launches[+e.dataset.i])));
      const ep=launches.find(l=>l.settled); if(ep){ const left=R.epochEnd(ep,R.epochOf(ep))-now(); const hum=left>=86400?Math.floor(left/86400)+'d '+Math.floor(left%86400/3600)+'h':left>=3600?Math.floor(left/3600)+'h '+Math.floor(left%3600/60)+'m':mmss(left); $('#pl-epoch').textContent='epoch '+R.epochOf(ep)+' · closes in '+hum; } }
    async function tape(){ if(busy||!launches.length) return; busy=true; try{ if(!D) D={sw:await disc('Swapped'),nw:await disc('LaunchCreated'),bu:await disc('Burned'),gr:await disc('GraduatedToRaydium')};
        const sigs=await conn.getSignaturesForAddress(new PublicKey(o.programId),{limit:25}); const fresh=sigs.filter(s=>!TAPE[s.signature]&&!s.err).slice(0,12);
        const txs=await Promise.all(fresh.map(sg=>conn.getTransaction(sg.signature,{maxSupportedTransactionVersion:0,commitment:'confirmed'}).catch(()=>null)));
        txs.forEach((tx,i)=>{ const sg=fresh[i]; const evs=[]; if(tx&&tx.meta&&!tx.meta.err) for(const line of tx.meta.logMessages||[]){ if(!line.startsWith('Program data: ')) continue; const u=b64(line.slice(14)); if(u.length<40) continue; const dv=new DataView(u.buffer,u.byteOffset); const launch=new PublicKey(u.slice(8,40)); const l=launches.find(x=>x.pubkey.equals(launch)); const sym=l?'$'+l.symbol:'a coin';
            if(eq8(u,D.sw)&&u.length>=104){ const trader=new PublicKey(u.slice(40,72)); const pairIn=dv.getBigUint64(72,true),tokOut=dv.getBigUint64(80,true),tokIn=dv.getBigUint64(88,true),pairOut=dv.getBigUint64(96,true); const buy=pairIn>0n; const dev=l&&trader.equals(l.creator); evs.push({cls:buy?'buy':'sell',h:`${buy?'BUY':'SELL'} ${fmtTok(buy?tokOut:tokIn)} ${sym}`,s:`${l?'for '+fmtPair(l,buy?pairIn:pairOut):''} · ${dev?'<span class="a">creator</span>':short(trader.toBase58())}`,l}); }
            else if(eq8(u,D.nw)){ evs.push({cls:'new',h:`NEW LAUNCH ${sym}`,s:l?(l.mode===1?'open curve · protected start':'sealed auction · 15 min')+' · paired with '+pairSym(l):'',l}); }
            else if(eq8(u,D.bu)&&u.length>=56){ const spent=dv.getBigUint64(40,true),burned=dv.getBigUint64(48,true); evs.push({cls:'burn',h:`BURNED ${fmtTok(burned)} RSRCT`,s:`buyback from ${sym} fees${l?' · '+fmtPair(l,spent):''}`,l}); }
            else if(eq8(u,D.gr)){ evs.push({cls:'grad',h:`${sym} GRADUATED TO RAYDIUM`,s:'liquidity moved · LP burned',l}); } }
          TAPE[sg.signature]={t:sg.blockTime||now(),sig:sg.signature,evs}; });
        const items=Object.values(TAPE).filter(x=>x.evs.length).sort((a,b)=>b.t-a.t).slice(0,14);
        $('#tape').innerHTML=items.map(x=>x.evs.map((e,k)=>`<div class="ev ${e.cls}" data-sig="${x.sig}" data-k="${k}" title="${x.sig}"><b>${e.h}</b><span>${new Date(x.t*1000).toLocaleTimeString('en-GB')} · ${e.s} · <a href="${explorer(x.sig,'tx')}" target="_blank" rel="noopener">tx</a></span></div>`).join('')).join('')||'<div class="ev"><b>no events yet</b><span>the first swap or launch will show here</span></div>';
        $('#tape').querySelectorAll('.ev[data-sig]').forEach(e=>e.addEventListener('click',ev=>{ if(ev.target.tagName==='A') return; const l=TAPE[e.dataset.sig].evs[+e.dataset.k].l; if(l) open(l); }));
        $('#tp-when').textContent='from the chain · '+new Date().toLocaleTimeString('en-GB'); }
      catch(e){ $('#tp-when').textContent='RPC busy · retrying'; } finally{ busy=false; } }
    const tick=async()=>{ if(window.innerWidth<minW) return; await pulse(); await tape(); };
    setTimeout(tick,o.delay||2500); setInterval(tick,o.every||20000);
  }
  window.Rails={mount};
})();
