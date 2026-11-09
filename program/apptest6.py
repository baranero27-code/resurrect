import asyncio,json
from playwright.async_api import async_playwright
PAYER=json.load(open('/home/claude/sol/resurrect-program/keys/payer.json'))
STOCK='So11111111111111111111111111111111111111112'
MOCK="""
(()=>{const {Keypair,PublicKey}=solanaWeb3;const kp=Keypair.fromSecretKey(Uint8Array.from(%s));
window.solana={isPhantom:true,publicKey:kp.publicKey,connect:async()=>({publicKey:kp.publicKey}),disconnect:async()=>{},on:()=>{},
 signTransaction:async(tx)=>{tx.partialSign(kp);return tx;}};})();
"""%json.dumps(PAYER)
async def main():
    async with async_playwright() as p:
        b=await p.chromium.launch(executable_path='/usr/bin/google-chrome',headless=True,args=['--no-sandbox']); pg=await b.new_page(viewport={'width':1400,'height':1000})
        pg.on('pageerror',lambda e: print('[pageerror]',str(e)[:300]))
        await pg.goto('http://127.0.0.1:8766/app.html?epochSecs=90&rpc=http://127.0.0.1:8899',wait_until='domcontentloaded'); await pg.wait_for_timeout(1500)
        await pg.evaluate(MOCK); await pg.click('#connect'); await pg.wait_for_timeout(1500)
        print('wallet:',await pg.inner_text('#connect'),'|',await pg.inner_text('#bal'))
        await pg.wait_for_timeout(1500); print('list:',(await pg.inner_text('#list')).replace('\\n',' | ')[:300])
        # create launch (1 min batch)
        await pg.fill('#c-name','Night Shift'); await pg.fill('#c-sym','NIGHT'); await pg.select_option('#c-batch','60'); await pg.select_option('#c-human','0'); await pg.wait_for_timeout(1500); print('pairinfo:',await pg.inner_text('#c-pairinfo')); await pg.click('#c-go')
        for _ in range(30):
            await pg.wait_for_timeout(1000); lg=await pg.inner_text('#log')
            if 'created' in lg or 'failed' in lg: break
        print('log:',lg.split('\\n')[0][:200])
        await pg.wait_for_timeout(1000); print('detail phase:',await pg.inner_text('#d-phase'))
        # commit bid
        await pg.fill('#b-amt','0.25'); await pg.click('#b-go')  # payer holds mock stock? (minted below)
        for _ in range(20):
            await pg.wait_for_timeout(1000); lg=await pg.inner_text('#log')
            if 'committed' in lg or 'commit failed' in lg: break
        print('log:',lg.split('\\n')[0][:200]); await pg.wait_for_timeout(1500); print('actions:',(await pg.inner_text('#actions'))[:200].replace('\\n',' | '))
        # open LAZ (live) and quote a buy
        await pg.click('#back'); await pg.wait_for_timeout(500)
        cards=await pg.query_selector_all('.tk'); 
        for c in cards:
            if 'LAZ' in await c.inner_text(): await c.click(); break
        await pg.wait_for_timeout(1500); print('LAZ phase:',await pg.inner_text('#d-phase')); print('pool:',(await pg.inner_text('#d-pool')).replace('\\n',' | '))
        await pg.fill('#sw-pair','0.1'); await pg.wait_for_timeout(300); print('quote:',await pg.inner_text('#sw-q1')); print('pop row:',await pg.inner_text('#d-pop')); print('rewards panel:',('HOLDER REWARDS' in await pg.inner_text('#actions')))
        await pg.click('#sw-buy')
        for _ in range(20):
            await pg.wait_for_timeout(1000); lg=await pg.inner_text('#log')
            if 'bought' in lg or 'buy failed' in lg: break
        print('log:',lg.split('\\n')[0][:200])
        await pg.fill('#st-amt','1000'); await pg.click('#st-go')
        for _ in range(20):
            await pg.wait_for_timeout(1000); lg=await pg.inner_text('#log')
            if 'staked' in lg or 'Stake failed' in lg: break
        print('log:',lg.split('\\n')[0][:200]); await pg.wait_for_timeout(2500); print('panels:',[k for k in ['TRADER REWARDS','PROTOCOL','HOLDER REWARDS'] if k in await pg.inner_text('#actions')]); print('rows:',(await pg.inner_text('#d-rows')).replace('\n',' | ')[:700]); print('stake rows:',(await pg.inner_text('#actions')).split('HOLDER REWARDS')[-1][:260].replace('\\n',' | '))
        await pg.screenshot(path='/tmp/app2.jpg',quality=80,type='jpeg',full_page=False)
        await b.close()
asyncio.run(main())
