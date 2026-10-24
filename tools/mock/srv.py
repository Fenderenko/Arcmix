"""Тестовий сервер вікна Arcmix з вигаданими даними — щоб перевіряти інтерфейс без Windows.
Запуск: python tools/mock/srv.py  →  http://127.0.0.1:8772/  (той самий web/index.html, що й у програмі).
POST /api/mock_set {…} змінює стан, /api/mock_dev {"f": "<файл з devs.json>"} — інша гарнітура."""
import http.server, json, sys, time, threading, struct, zlib, os
HERE = os.path.dirname(os.path.abspath(__file__))
WEB = os.path.join(HERE, '..', '..', 'web')
DEVS = json.load(open(os.path.join(HERE, 'devs.json'), encoding='utf-8'))
FONT = open(os.path.join(WEB, 'onest.ttf'),'rb').read()
def png(rgb):
    w=h=32; raw=b''
    for y in range(h):
        raw+=b'\x00'
        for x in range(w):
            a = 255 if (x-16)**2+(y-16)**2<200 else 0
            raw+=bytes(rgb)+bytes([a])
    def ch(t,d): c=struct.pack('>I',len(d))+t+d; return c+struct.pack('>I',zlib.crc32(t+d)&0xffffffff)
    return b'\x89PNG\r\n\x1a\n'+ch(b'IHDR',struct.pack('>IIBBBBB',w,h,8,6,0,0,0))+ch(b'IDAT',zlib.compress(raw))+ch(b'IEND',b'')
# apps removed from the groups ("+" brings them back)
HIDDEN = []
STATE = {"win":{"pal":["#99EBFF","#4CC2FF","#0091F8","#0078D4","#0067C0","#003E92","#001A68","#F7630C"],"light":False},"nv":{"state":"ready"},"apo":{"installed":True,"ready":True,"dev":True,"sysfxOff":False,"hs":True},"apoMod":{"inst":False,"live":False,"free":False,"sysfx":False},"out":{"id":"r1","name":"Headphones (Arctis 7+)","short":"Arctis 7+","conn":"24g","headset":True,"vol":64,"mute":False},"connected":True,"powered":True,"haveBattery":True,"battery":75,"charging":False,"game":100,"chat":100,"pos":0,
 "cfg":{"theme":"pink","mode":"dark","lowBattery":True,"morph":"ink","sidetone":-1,"autoOff":-1,"autostart":True,"micGain":-1,"muteLed":-1,"customA":"#FF6A3D","customB":"#3DC8FF","route":{"on":False,"game":"","chat":"","media":"","out":"","auto":True},"micFx":{"on":False,"in":"","out":"","gate":-50,"ceil":-6,"denoise":True,"plugins":[]},"wheelMode":"battery","micDev":"","softEq":{},"autoSwitch":True,"switchMic":True,"lowLevel":25,"fullNotify":True,"osd":True,"updateCheck":True,"micHotkey":True,"ptt":False,"pttKey":0,"sounds":True,"winCtl":True,"osdCorner":"br","duckLevel":60,"duckMedia":False,"onboarded":True,"eqUser":[],"hotkeys":{"game":{"mod":6,"vk":39},"chat":{"mod":6,"vk":37},"balance":{"mod":6,"vk":40},"mic":{"mod":3,"vk":77},"out":{"mod":3,"vk":39},"outPrev":{"mod":3,"vk":37},"in":{"mod":3,"vk":40},"inPrev":{"mod":3,"vk":38}},"profiles":["CS2 — кроки","Музика"]},
 "duck":{"watching":True,"active":False},"monitor":{"mode":"","progress":0,"has":False,"tap":False},"hotkeyErr":[],"winCall":{"active":False,"who":"","pref":1},
 "apps":[{"exe":"Discord.exe","name":"Discord","cat":"chat","live":True,"playing":True,"sound":True,"removable":False,"vol":100},
         {"exe":"cs2.exe","name":"Counter-Strike 2","cat":"game","live":True,"playing":True,"sound":True,"removable":False,"vol":100},{"exe":"steam.exe","name":"Steam","cat":"game","live":True,"playing":False,"removable":False,"vol":100},
         {"exe":"Spotify.exe","name":"Spotify","cat":"media","live":True,"playing":True,"sound":True,"removable":False,"vol":100},
         {"exe":"telegram.exe","name":"Telegram Desktop","cat":"chat","live":False,"playing":False,"removable":True,"running":True,"vol":-1},{"exe":"obs64.exe","name":"OBS Studio","cat":"media","live":False,"playing":False,"removable":True,"vol":-1}],
 "version":"1.0.0","battHours":14.5,"update":{"avail":"1.8","page":"","current":"1.7"},"mic":{"ok":True,"name":"Microphone (Arctis 7+ Chat)","level":80,"mute":False},"eq":[0,0,0,0,0,0,0,0,0,0],"eqPreset":"flat","eqCustom":[0,0,0,0,0,0,0,0,0,0],"device":{"name":"Arctis 7+","native":True,"eq":{"api":"eq","kind":"graphic","bands":10,"freqs":[32,64,125,250,500,1000,2000,4000,8000,16000],"min":-12,"max":12}}}
STATE['outs']=[{"id":"r1","name":"Headphones (Arctis 7+)","state":"on","default":True},
 {"id":"r2","name":"Speakers (Focusrite USB Audio)","state":"on"},
 {"id":"r6","name":"CABLE Input (VB-Audio Virtual Cable)","state":"on"},
 {"id":"r3","name":"DELL U2720Q (NVIDIA High Definition Audio)","state":"disabled"},
 {"id":"r4","name":"Speakers (Realtek(R) Audio)","state":"unplugged"},
 {"id":"r5","name":"Headphones (Arctis Nova 7)","state":"absent"}]
STATE['cfg']['taskbar']=False
I18 = json.load(open(os.path.join(WEB, 'i18n.json'), encoding='utf-8'))
def page(query=''):
    l = STATE['cfg'].get('lang') or 'uk'
    t = {} if l=='uk' else {k:v[l] for k,v in I18['strings'].items() if v.get(l)}
    pl = json.dumps({"lang":l,"pick":STATE['cfg'].get('lang',''),"langs":I18['langs'],"t":t}, ensure_ascii=False)
    html = open(os.path.join(WEB, 'index.html'),encoding='utf-8').read()
    html = html.replace('/*I18N*/{"lang":"uk","t":{},"langs":[]}', pl, 1)
    # screenshots: ?shot=<tab>[&sub=<settings page>][&eqt=<equalizer tab>][&to=<element id>] opens that view after the page is drawn
    q = dict(x.split('=',1) for x in query.split('&') if '=' in x)
    if q.get('shot'):
        js = ("setTimeout(function(){var q=%s;document.querySelector('#nav [data-tab='+q.shot+']').click();"
              "if(q.sub)document.querySelector('#setTabs [data-p='+q.sub+']').click();"
              "if(q.eqt)document.querySelector('#eqTarget [data-t='+q.eqt+']').click();"
              "setTimeout(function(){if(q.to){var e=document.getElementById(q.to),v=e.closest('.view');v.scrollTop=e.offsetTop-62;}},300);},700);") % json.dumps(q)
        # headless browsers keep windows at least ~500 px wide: the page is held to the app's 400 px;
        # no live event stream, so a headless browser's virtual time can run out and take the picture
        html = html.replace('<head>', '<head><script>window.EventSource=function(u){var o={close:function(){}};var k=(u.match(/[?&]m=(\\w+)/)||[])[1];var d={mic:{db:-14,ok:true},fx:{in:-18,out:-20,open:true,ok:true,run:true}}[k];if(d)setTimeout(function(){o.onmessage&&o.onmessage({data:JSON.stringify(d)});},50);return o;};</script>', 1)
        html = html.replace('</body>', '<style>html,body{width:400px!important;max-width:400px!important;overflow:hidden}*,*::before,*::after{transition:none!important}</style><script>' + js + '</script></body>', 1)
    return html.encode()
DEVS2={"render":[{"id":"r1","name":"Headphones (Arctis 7+)","virtual":False,"default":True},{"id":"r2","name":"Speakers (Realtek(R) Audio)","virtual":False},
 {"id":"c1","name":"CABLE Input (VB-Audio Virtual Cable)","virtual":True,"pair":"CABLE Output (VB-Audio Virtual Cable)"},
 {"id":"c2","name":"Hi-Fi Cable Input (VB-Audio Hi-Fi Cable)","virtual":True,"pair":"Hi-Fi Cable Output (VB-Audio Hi-Fi Cable)"},
 {"id":"c3","name":"Line 1 (Virtual Audio Cable)","virtual":True,"pair":"Line 1 (Virtual Audio Cable)"}],
 "capture":[{"id":"m1","name":"Microphone (Arctis 7+ Chat)","virtual":False,"default":True},{"id":"m2","name":"CABLE Output (VB-Audio Virtual Cable)","virtual":True}]}
import math, random
MON=[0]
VSTFOUND=[{"path":"C:\\Program Files\\Common Files\\VST3\\TDR Nova.vst3","name":"TDR Nova (VST3)"},{"path":"C:\\Program Files\\VSTPlugins\\rnnoise_stereo.dll","name":"rnnoise_stereo"},{"path":"C:\\Program Files\\VSTPlugins\\ReaPlugs\\reacomp-standalone.dll","name":"reacomp-standalone"},{"path":"C:\\Program Files\\Common Files\\VST2\\TDR Nova.dll","name":"TDR Nova"}]
def status():
    c=STATE['cfg']; r=c['route']; m=c['micFx']
    STATE['route']={"on":r['on'],"running":r['on'] and bool(r['game'] or r['chat'] or r['media']),"latency":31}
    STATE['micFx']={"on":m['on'],"running":m['on'] and bool(m['out']),"latency":24}
    STATE['appRoute']=True
    STATE['monitor']['tap']=STATE['micFx']['running']
    mo=STATE['monitor']
    if mo['mode'] in ('record','play'):
        mo['progress']=min(1,(time.time()-MON[0])/5)
        if mo['progress']>=1: mo['mode']='ready'; mo['has']=True; mo['progress']=0
    names={'rnnoise_stereo':'Noise Suppression (RNNoise)','reacomp-standalone':'ReaComp','TDR Nova':'TDR Nova'}
    out=[]
    for sl in STATE['cfg']['micFx'].get('plugins',[]):
        b=sl['path'].split('\\')[-1].rsplit('.',1)[0]
        e={'id':sl['id'],'name':names.get(b,b),'editor':True,'open':False}
        if 'reacomp' in b: e['err']='32-bit'; e['editor']=False; e['name']=b
        out.append(e)
    STATE['vst']=out
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self,*a): pass
    def do_GET(self):
        p=self.path.split('?')[0]
        if p=='/': self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.end_headers(); self.wfile.write(page(self.path.split('?',1)[1] if '?' in self.path else ''))
        elif p in ('/app.css','/app.js'):
            self.send_response(200); self.send_header('Content-Type','text/css' if p.endswith('css') else 'text/javascript'); self.end_headers()
            self.wfile.write(open(os.path.join(os.path.dirname(os.path.abspath(__file__)),'..','..','web',p[1:]),'rb').read())
        elif p=='/font.ttf': self.send_response(200); self.send_header('Content-Type','font/ttf'); self.end_headers(); self.wfile.write(FONT)
        elif p=='/api/vst/scan': self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers(); self.wfile.write(json.dumps(VSTFOUND).encode())
        elif p=='/api/audiodevs': self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers(); self.wfile.write(json.dumps(DEVS2).encode())
        elif p=='/api/meters':
            kind=dict(x.split('=',1) for x in self.path.split('?',1)[1].split('&') if '=' in x).get('m','mic') if '?' in self.path else 'mic'
            self.send_response(200); self.send_header('Content-Type','text/event-stream'); self.end_headers()
            try:
                while True:
                    t=time.time()
                    if kind=='duck':
                        talk=STATE.get('duck',{}).get('active',False) or int(t/3)%2==0; db=(-28+10*math.sin(t*9)) if talk else -66+2*math.sin(t)
                        v={'db':round(db),'thr':-50,'voice':talk and not STATE.get('echo'),'active':talk and not STATE.get('echo'),'echo':STATE.get('echo',''),'watching':True}
                    elif kind=='fx' and STATE['cfg']['micFx'].get('on'):
                        x=-62+4*math.sin(t*3) if int(t)%3==0 else -24+8*math.sin(t*7)
                        v={'in':x,'out':min(x,-6),'open':x>STATE['cfg']['micFx']['gate'],'ok':True,'run':True}
                    else:
                        x=-30+18*abs(math.sin(t*2.3))+(8 if int(t*2)%7==0 else 0); v={'db':min(0,x),'ok':True}
                    self.wfile.write(b'data: '+json.dumps(v).encode()+b'\n\n'); self.wfile.flush(); time.sleep(0.033)
            except Exception: pass
        elif p=='/api/miclevel':
            t=time.time(); v=-62+4*math.sin(t*3) if int(t)%3==0 else -24+8*math.sin(t*7)
            self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers(); self.wfile.write(json.dumps({'in':v,'out':min(v,-6),'open':v>STATE['cfg']['micFx']['gate'],'ok':True}).encode())
        elif p=='/api/micpeak':
            t=time.time(); v=-30+18*abs(math.sin(t*2.3))+ (8 if int(t*2)%7==0 else 0)
            self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers(); self.wfile.write(json.dumps({'db':min(0,v),'ok':True}).encode())
        elif p=='/api/monitor':
            status(); self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers(); self.wfile.write(json.dumps(STATE['monitor']).encode())
        elif p=='/api/duckmeter':
            t=time.time(); talk=STATE.get('duck',{}).get('active',False) or int(t/3)%2==0; db=(-28+10*math.sin(t*9)) if talk else -66+2*math.sin(t)
            self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers(); self.wfile.write(json.dumps({'db':round(db),'thr':-50,'voice':talk and not STATE.get('echo'),'active':talk and not STATE.get('echo'),'echo':STATE.get('echo',''),'watching':True,'win':STATE.get('winDuck',1)}).encode())
        elif p=='/api/state': self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers(); self.wfile.write(json.dumps((status(),STATE)[1]).encode())
        elif p=='/api/events':
            self.send_response(200); self.send_header('Content-Type','text/event-stream'); self.end_headers()
            try:
                while True:
                    status(); self.wfile.write(b'data: '+json.dumps(STATE).encode()+b'\n\n'); self.wfile.flush(); time.sleep(0.4)
            except Exception: pass
        elif p=='/icon':
            q=self.path
            # real icons, if extracted (rust: appinfo icon_dump test); not in git
            from urllib.parse import parse_qs, urlsplit
            f=os.path.join(os.path.dirname(os.path.abspath(__file__)),'icons',(parse_qs(urlsplit(q).query).get('exe',[''])[0])+'.png')
            if os.path.basename(f)!='.png' and os.path.isfile(f):
                self.send_response(200); self.send_header('Content-Type','image/png'); self.end_headers(); self.wfile.write(open(f,'rb').read()); return
            if 'Discord' in q: c=(88,101,242)
            elif 'Spotify' in q: c=(30,215,96)
            elif 'steam' in q: c=(60,80,110)
            elif 'cs2' in q: c=(222,150,40)
            else: self.send_response(404); self.end_headers(); return
            self.send_response(200); self.send_header('Content-Type','image/png'); self.end_headers(); self.wfile.write(png(c))
        elif p=='/api/procs': self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers(); self.wfile.write(json.dumps([{"exe":"Code.exe","name":"Visual Studio Code","listed":False},{"exe":"obs64.exe","name":"OBS Studio","listed":False}]+[{"exe":a['exe'],"name":a['name'],"listed":False} for a in HIDDEN]).encode())
        else: self.send_response(404); self.end_headers()
    def do_POST(self):
        n=int(self.headers.get('Content-Length',0)); body=json.loads(self.rfile.read(n) or b'{}'); p=self.path.split('?')[0]
        if p=='/api/update':
            time.sleep(0.8); self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers()
            self.wfile.write(json.dumps({'latest':STATE['version'],'newer':False}).encode()); return
        if p=='/api/autoeq' and 'q' in body:
            # a few models, as the AutoEQ index lists them
            ms=[{'n':'SteelSeries Arctis 7+','s':'Rtings on HMS II.3','p':'x'},{'n':'SteelSeries Arctis 7P','s':'Rtings on HMS II.3','p':'y'},{'n':'Sony LinkBuds S','s':'DHRME','p':'z'},{'n':'Sony WH-1000XM5','s':'oratory1990','p':'w'}]
            q=body['q'].lower().split(); l=[m for m in ms if all(w in m['n'].lower() for w in q)]
            time.sleep(0.3); self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers()
            self.wfile.write(json.dumps({'list':l}).encode()); return
        if p=='/api/autoeq':
            dev=STATE['out']['id']; ae=STATE['cfg'].setdefault('autoEq',{})
            if body.get('remove'): ae.pop(dev,None); STATE['apo'].pop('autoEq',None)
            elif 'on' in body and dev in ae: ae[dev]['on']=body['on']
            elif body.get('path'): ae[dev]={'on':True}; STATE['apo']['autoEq']=[body['name'],body['src']]; time.sleep(0.5)
            self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers(); self.wfile.write(b'{"ok":true}'); return
        if p=='/api/apo':
            if body.get('op')=='modInstall': STATE['apoMod']={'inst':True,'live':True,'free':False}
            if body.get('op')=='modRemove': STATE['apoMod']={'inst':False,'live':False,'free':True}
            if body.get('op')=='setup': STATE['apo']['ready']=True
            if body.get('op')=='devices': STATE['apo']['dev']=True
        if p=='/api/pos': STATE['pos']=body['v']; STATE['game']=100-max(0,body['v']); STATE['chat']=100-max(0,-body['v'])
        elif p=='/api/cfg':
            hk=body.pop('hotkeys',None); ft=body.pop('features',None); STATE['cfg'].update(body)
            if ft: STATE['cfg'].setdefault('features',{}).update(ft)
            if hk: STATE['cfg']['hotkeys'].update(hk)
        elif p=='/api/mic':
            if 'level' in body: STATE['mic']['level']=body['level']
            if 'mute' in body: STATE['mic']['mute']=body['mute']
        elif p=='/api/eq' and body.get('target'):
            STATE['cfg']['softEq'][body['target']]={'bands':body['bands'],'preset':body['preset'],'custom':body['bands'] if body['preset']=='custom' else STATE['cfg']['softEq'].get(body['target'],{}).get('custom')}
        elif p=='/api/eq':
            STATE['eq']=body['bands']; STATE['eqPreset']=body['preset']
            if STATE['device'].get('eq') and STATE['device']['eq'].get('presets'): STATE['device']['hwPreset']=STATE['device']['eq']['customSlot']
            if body['preset']=='custom': STATE['eqCustom']=body['bands']
        elif p=='/api/mock_dev':
            d=DEVS[body['f']]; d['name']=d['name'].replace('SteelSeries ',''); STATE['device']=d
            n=d['eq']['bands'] if d.get('eq') else 10
            STATE['eq']=[0]*n; STATE['eqCustom']=[0]*n
        elif p=='/api/mock_set': STATE.update(body)
        elif p=='/api/outvol':
            if 'vol' in body: STATE['out']['vol']=body['vol']
            if 'mute' in body: STATE['out']['mute']=body['mute']
        elif p=='/api/output':
            for d in DEVS2['render']:
                if d['id']==body['dev']:
                    STATE['out']={'id':d['id'],'name':d['name'],'short':d['name'].split('(')[-1].rstrip(')'),'conn':'wire','headset':False,'vol':STATE['out'].get('vol',64),'mute':False}
        elif p=='/api/outdev' and body.get('op')=='order':
            by={d['id']:d for d in STATE['outs']}; STATE['outs']=[by[x] for x in body['ids'] if x in by]
        elif p=='/api/winsound': pass
        elif p=='/api/outdev':
            l=STATE['outs']; i=[k for k,d in enumerate(l) if d['id']==body['id']][0]
            if body['op']=='up' and i>0: l[i-1],l[i]=l[i],l[i-1]
            elif body['op'] in ('on','off'): l[i]['state']='on' if body['op']=='on' else 'disabled'
            elif body['op']=='forget': l.pop(i)
        elif p=='/api/winducking': STATE['winCall']['pref']=3
        elif p=='/api/route': STATE['cfg']['route'].update(body)
        elif p=='/api/vst':
            pl=STATE['cfg']['micFx']['plugins']
            if body['op']=='add': pl.append({'id':str(len(pl)+1)+'x','path':body['path'],'on':True})
            elif body['op']=='toggle':
                for x in pl:
                    if x['id']==body['id']: x['on']=body['on']
            elif body['op']=='remove': STATE['cfg']['micFx']['plugins']=[x for x in pl if x['id']!=body['id']]
        elif p=='/api/mic' and 'dev' in body: STATE['cfg']['micDev']=body['dev']
        elif p=='/api/micfx': STATE['cfg']['micFx'].update(body)
        elif p=='/api/eqpreset': STATE['device']['hwPreset']=body['value']
        elif p=='/api/dev':
            for st in STATE['device'].get('settings',[]):
                if st['api']==body['api'] and st['field']==body['field']: st['value']=body['value']
        elif p=='/api/profile':
            pr=STATE['cfg']['profiles']
            if body['op']=='save' and body['name'] not in pr: pr.append(body['name'])
            if body['op']=='save':
                pa=STATE['cfg'].setdefault('profApps',{}); pa.pop(body['name'],None)
                if body.get('app'): pa[body['name']]=body['app']
            if body['op']=='delete': STATE['cfg']['profiles']=[x for x in pr if x!=body['name']]
        elif p=='/api/update':
            if body['op']=='install': STATE['update']['status']='downloading'
            if body['op']=='skip': STATE['update']={'current':'1.7','status':'latest'}
            if body['op']=='check': STATE['update']['status']='latest'
        elif p=='/api/report':
            self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers(); self.wfile.write(json.dumps({'path':'C:\\Users\\Олександр\\Desktop\\ArctisChatMix-report-2026-10-14-201500.zip'}).encode()); return
        elif p=='/api/app' and 'vol' in body:
            for a in STATE['apps']:
                if a['exe']==body['exe']: a['vol']=round(body['vol'])
        elif p=='/api/monitor':
            mo=STATE['monitor']; op=body['op']
            if op in ('record','play'): MON[0]=time.time(); mo['mode']=op; mo['progress']=0
            elif op=='live': mo['mode']='live'
            else:
                if mo['mode']=='record': mo['has']=True
                mo['mode']='ready' if mo['has'] else ''
        elif p=='/api/equser':
            l=STATE['cfg']['eqUser']; same=[i for i,x in enumerate(l) if x['name'].lower()==body['name'].lower()]
            if body['op']=='save':
                e={'name':body['name'],'bands':body['bands'],'freqs':body['freqs']}
                if same: l[same[0]]=e
                else: l.append(e)
            elif body['op']=='rename' and same:
                l[same[0]]['name']=body['to']
                if STATE['eqPreset']=='u:'+body['name']: STATE['eqPreset']='u:'+body['to']
            elif body['op']=='delete': STATE['cfg']['eqUser']=[x for x in l if x['name'].lower()!=body['name'].lower()]
        elif p=='/api/app/remove':
            # removed apps leave the groups and come back through "+"
            HIDDEN.extend(a for a in STATE['apps'] if a['exe']==body['exe'])
            STATE['apps']=[a for a in STATE['apps'] if a['exe']!=body['exe']]
        elif p=='/api/app':
            back=[a for a in HIDDEN if a['exe']==body['exe']]
            for a in back: HIDDEN.remove(a); STATE['apps'].append(a)
            for a in STATE['apps']:
                if a['exe']==body['exe']: a['cat']=body['cat']
        self.send_response(204); self.end_headers()
http.server.ThreadingHTTPServer(('127.0.0.1',8772),H).serve_forever()
