(function () {
  // ---------- переклади: словник поточної мови вставляє сервер ----------
  var I18 = window.I18N; // the server puts the dictionary of the current language into the page
  function tx(s) {
    if (s === null || s === undefined) return s;
    s = String(s);
    var d = I18.t || {};
    if (d[s]) return d[s];
    if (I18.lang === 'uk') return s;
    var m = s.match(/^(\d+) хв$/);
    if (m) return m[1] + ' ' + (d['хв'] || 'min');
    var i = s.indexOf(': ');
    if (i > 0) {
      var a = s.slice(0, i), b = s.slice(i + 2), bb = b.charAt(0).toUpperCase() + b.slice(1);
      if (d[a] && d[bb]) { var r = d[bb]; if (I18.lang !== 'de') r = r.charAt(0).toLowerCase() + r.slice(1); return d[a] + ': ' + r; }
    }
    return s;
  }
  // статичний текст сторінки: перекладаємо лише «слова», зберігаючи стрілки, числа й знаки навколо
  (function (root) {
    var re = /^([^A-Za-zÀ-ɏЀ-ӿ]*)([\s\S]*?)([^A-Za-zÀ-ɏЀ-ӿ.)»]*)$/;
    var w = document.createTreeWalker(root, NodeFilter.SHOW_TEXT), n;
    while ((n = w.nextNode())) {
      if (n.parentNode && /^(SCRIPT|STYLE)$/.test(n.parentNode.nodeName)) continue;
      var m = n.nodeValue.match(re);
      if (m && m[2]) { var r = tx(m[2]); if (r !== m[2]) n.nodeValue = m[1] + r + m[3]; }
    }
    root.querySelectorAll('[aria-label],[title]').forEach(function (el) {
      ['aria-label', 'title'].forEach(function (a) { var v = el.getAttribute(a); if (v) el.setAttribute(a, tx(v)); });
    });
    document.documentElement.lang = I18.lang || 'uk';
  })(document.body);

  // ---------- теми: два кольори (гра, чат) + тон поверхні ----------
  var THEMES = [
    // «Чорна діра»: neutral near-black, text-coloured controls; the game and chat colours are only marks
    { k: 'hole', n: 'Чорна діра', a: '#E9EAEE', b: '#8FB2E3', s: '#0F1011', s1: '#17181A', s2: '#222326', sd: '#1C1D1F', tr: '#2C2E31', sw: 'a' },
    // «Як у Windows»: кольори, режим і прозорість — з налаштувань Windows (див. themeOf)
    { k: 'win', n: 'Як у Windows', a: '#4CC2FF', b: '#E8A87C', s: '#202020', s1: '#2B2B2B' },
    // OLED: the Windows accent on pure black (see oledTheme); a and b here only until the Windows colours are known
    { k: 'black', n: 'Чорна (OLED)', a: '#4CC2FF', b: '#E8A87C', s: '#000000', s1: '#111113', s2: '#1C1C1F', badge: 'OLED' },
    // NFS Heat at night: hot pink and electric cyan neon on a deep violet night, a violet haze in the corners
    { k: 'pink', n: 'Туманність', a: '#FF2E97', b: '#19D3FF', s: '#0A0614', s1: '#130B22', s2: '#1D1233', sd: '#100920', tr: '#2A1D44', tint: true, prim: 'linear-gradient(90deg,#FF2E97,#A04BFF)',
      fx: 'radial-gradient(120% 45% at 100% 0%,rgba(160,75,255,.16),transparent 60%),radial-gradient(90% 40% at 0% 100%,rgba(25,211,255,.08),transparent 60%)' },
    { k: 'blue', n: 'Нептун', a: '#6CB8FF', b: '#4FE3D9', s: '#0E141C' },
    { k: 'gray', n: 'Місяць', a: '#D3D6DD', b: '#8FB2E3', s: '#151618' },
    // теми програм: кольори зняті з самих програм; fill — колір натиснутих кнопок, ink — текст на них
    { k: 'spotify', n: 'Spotify', a: '#1ED760', b: '#B3B3B3', bl: '#191414', s: '#000000', s1: '#121212', s2: '#282828', sd: '#1F1F1F', fill: '#1ED760', ink: '#000000', sw: 'a' },
    { k: 'discord', n: 'Discord', a: '#5865F2', b: '#23A55A', s: '#121214', s1: '#1A1A1E', s2: '#28282D', sd: '#2E2E33', fill: '#5865F2', ink: '#FFFFFF' },
    { k: 'steam', n: 'Steam', a: '#66C0F4', b: '#A4D007', s: '#171A21', s1: '#1B2838', s2: '#2A475E', sd: '#16202D', fill: '#1A9FFF', ink: '#FFFFFF' },
    { k: 'twitch', n: 'Twitch', a: '#BF94FF', b: '#FF75E6', s: '#0E0E10', s1: '#18181B', s2: '#26262C', sd: '#26262C', fill: '#9147FF', ink: '#FFFFFF' },
    // YouTube in its dark mode: the red of the progress bar, blue switches, white chosen chips
    { k: 'youtube', n: 'YouTube', a: '#FF0033', b: '#F1F1F1', bl: '#0F0F0F', s: '#0F0F0F', s1: '#141414', s2: '#272727', sd: '#272727', fill: '#F1F1F1', ink: '#0F0F0F', fillL: '#0F0F0F', inkL: '#FFFFFF' },
    { k: 'telegram', n: 'Telegram', a: '#2AABEE', b: '#7BC862', s: '#0E1621', s1: '#17212B', s2: '#242F3D', sd: '#202B36', fill: '#2B5278', ink: '#FFFFFF', sw: 'a' },
    { k: 'custom', n: 'Власна' }
  ];
  function h2r(h) { h = (h || '#888888').replace('#', ''); return [0, 2, 4].map(function (i) { return parseInt(h.substr(i, 2), 16) / 255; }); }
  function r2h(c) { return '#' + c.map(function (v) { return ('0' + Math.round(Math.max(0, Math.min(1, v)) * 255).toString(16)).slice(-2); }).join(''); }
  function r2hsl(c) {
    var r = c[0], g = c[1], b = c[2], mx = Math.max(r, g, b), mn = Math.min(r, g, b), l = (mx + mn) / 2, h = 0, s = 0, d = mx - mn;
    if (d) { s = l > .5 ? d / (2 - mx - mn) : d / (mx + mn); h = mx === r ? (g - b) / d + (g < b ? 6 : 0) : mx === g ? (b - r) / d + 2 : (r - g) / d + 4; h /= 6; }
    return [h, s, l];
  }
  function hsl2r(h, s, l) {
    function f(p, q, t) { t = (t + 1) % 1; return t < 1 / 6 ? p + (q - p) * 6 * t : t < .5 ? q : t < 2 / 3 ? p + (q - p) * (2 / 3 - t) * 6 : p; }
    if (!s) return [l, l, l];
    var q = l < .5 ? l * (1 + s) : l + s - l * s, p = 2 * l - q;
    return [f(p, q, h + 1 / 3), f(p, q, h), f(p, q, h - 1 / 3)];
  }
  function lum(c) { c = c.map(function (v) { return v <= .03928 ? v / 12.92 : Math.pow((v + .055) / 1.055, 2.4); }); return .2126 * c[0] + .7152 * c[1] + .0722 * c[2]; }
  function cr(a, b) { var A = lum(a), B = lum(b); return (Math.max(A, B) + .05) / (Math.min(A, B) + .05); }
  function L(c, l) { var h = r2hsl(c); return hsl2r(h[0], h[1], Math.max(0, Math.min(1, l))); }
  function tone(c, s, l) { var h = r2hsl(c); return hsl2r(h[0], Math.min(h[1], s), l); }
  function fit(c, bg, ratio) {
    var h = r2hsl(c), l = h[2], up = lum(bg) < .18;
    for (var i = 0; i < 100 && cr(hsl2r(h[0], h[1], l), bg) < ratio; i++) { l += up ? .01 : -.01; if (l <= 0 || l >= 1) break; }
    return hsl2r(h[0], h[1], Math.max(0, Math.min(1, l)));
  }
  function container(c, dark) {
    var h = r2hsl(c), sat = (h[1] < .2 || h[2] < .15) ? h[1] * .3 : Math.max(h[1], .45);
    return hsl2r(h[0], sat * (dark ? .75 : .85), dark ? .27 : .86);
  }
  function makeTokens(t, mode) {
    var A = h2r(t.a), B = h2r(mode === 'light' && t.bl ? t.bl : t.b), dark = mode !== 'light', hA = r2hsl(A), o = {};
    // surfaces not given by a theme are near neutral: the game and chat colours are the marks, not the
    // background, so neither of them takes over the window
    // a chosen background (the custom theme): its hue and saturation, at the lightness the mode needs
    var hBg = t.bg ? r2hsl(h2r(t.bg)) : null;
    if (dark) {
      // a chosen background: darker or lighter as chosen, within the dark range (white text stays readable)
      o.s = t.s ? h2r(t.s) : hBg ? hsl2r(hBg[0], hBg[1], .04 + .14 * hBg[2]) : hsl2r(hA[0], Math.min(hA[1], .35) * .12, .075);
      o.s1 = t.s1 ? h2r(t.s1) : L(o.s, r2hsl(o.s)[2] + .055);
      o.s2 = t.s2 ? h2r(t.s2) : L(o.s, r2hsl(o.s)[2] + .11);
      var l1 = r2hsl(o.s1)[2];
      o.sd = t.sd ? h2r(t.sd) : L(o.s1, Math.max(0, l1 - .055));
      o.tr = t.tr ? h2r(t.tr) : L(o.s1, l1 + .11);
      o.on = tone(o.s, .12, .97);
    } else if (t.lt) { // світлі поверхні, задані темою (як у Windows 11)
      o.s = h2r(t.lt.s); o.s1 = h2r(t.lt.s1); o.s2 = h2r(t.lt.s2); o.sd = h2r(t.lt.sd); o.tr = h2r(t.lt.tr);
      o.on = h2r('#1B1B1B');
    } else {
      // світла: сіре тло вікна, білі картки — блоки чітко відділені
      var lh = hBg ? hBg[0] : hA[0], ls = hBg ? Math.min(hBg[1], .45) : Math.min(hA[1], .4) * .25;
      o.s = hsl2r(lh, ls * .4, .915);
      o.s1 = hsl2r(lh, ls * .32, .995);
      o.s2 = hsl2r(lh, ls * .32, .97);
      o.sd = hsl2r(lh, ls * .4, .9);
      o.tr = hsl2r(lh, ls * .32, .84);
      if (hBg) {
        // the custom theme's own background: its hue clearly, darker or lighter as chosen, within the light
        // range (dark text stays readable) — the light mode never turns the window dark
        var bl = .8 + .17 * hBg[2], bs = Math.min(hBg[1], .6);
        o.s = hsl2r(lh, bs * .8, bl);
        o.s1 = hsl2r(lh, bs * .45, Math.min(.985, bl + .07));
        o.s2 = hsl2r(lh, bs * .5, Math.min(.96, bl + .04));
        o.sd = hsl2r(lh, bs * .8, bl - .03);
        o.tr = hsl2r(lh, bs * .7, bl - .08);
      }
      o.on = tone(o.s, .2, .09);
    }
    o.ov = fit(tone(o.s, .1, dark ? .8 : .3), o.s1, 7);
    o.ol = fit(tone(o.s, .1, dark ? .58 : .48), o.s1, 3.2);
    o.g = fit(A, o.s1, 4.5); o.gc = container(A, dark); o.og = fit(L(A, dark ? .93 : .14), o.gc, 7.5);
    var fl = dark ? t.fill : (t.fillL || t.fill);
    if (fl) { // натиснуті кнопки — суцільним кольором, як у самій програмі
      o.gc = h2r(fl);
      var ink = dark ? t.ink : (t.inkL || t.ink);
      o.og = ink ? h2r(ink) : (lum(o.gc) > .4 ? [0, 0, 0] : [1, 1, 1]);
    }
    o.c = fit(B, o.s1, 4.5); o.cc = container(B, dark); o.oc = fit(L(B, dark ? .93 : .14), o.cc, 7.5);
    // roles: sliders and the active tab take the game colour, switches the chat one (or the game one, where the
    // app itself has it so); chosen chips and the main button — the app's own fill, else the game colour
    var mixc = function (x, y, t) { return [0, 1, 2].map(function (i) { return x[i] * t + y[i] * (1 - t); }); };
    var inkOf = function (c) { return lum(c) > .4 ? [0, 0, 0] : [1, 1, 1]; };
    o.sw = t.sw === 'a' ? o.g : o.c; o.swk = lum(o.sw) > .4 ? o.s : [1, 1, 1];
    if (!fl) { o.gc = o.g; o.og = inkOf(o.g); }
    // the chosen tab: the fill where the app uses one, else a tint of the game colour
    o.tab = fl ? o.gc : mixc(o.g, o.s2, dark ? .24 : .18); o.tabc = fl ? o.og : o.on;
    var mark = function (col) { return cr(col, o.tab) < 2.4 ? o.tabc : col; };
    o.tdg = mark(o.g); o.tdc = mark(o.c); o.tdp = mark(h2r('#34C46F'));
    var out = {};
    for (var k in o) out[k] = r2h(o[k]);
    out.prim = t.prim || out.gc;
    out.fx = t.fx || 'none';
    // lines tinted by the game colour (the neon themes), else neutral
    out.line = t.tint ? 'color-mix(in srgb,' + out.g + ' 16%,transparent)' : 'color-mix(in srgb,' + out.on + ' 8%,transparent)';
    out.line2 = t.tint ? 'color-mix(in srgb,' + out.g + ' 30%,transparent)' : 'color-mix(in srgb,' + out.on + ' 13%,transparent)';
    out.card = dark ? 'none' : '0 1px 2px rgba(0,0,0,.07), 0 0 0 1px rgba(0,0,0,.06)';
    out.sb = out.s; out.wbg = out.s; out.solid = out.s1; // тло липких панелей, тло вікна й суцільне тло карток
    return out;
  }
  // «Як у Windows»: акцент — з палітри Windows (світлий відтінок у темному режимі, темний —
  // у світлому), колір чату — протилежний акценту відтінок. Поверхні — як у Windows 11.
  function winTheme() {
    var w = (S && S.win) || {}, p = w.pal || [], light = !!w.light;
    var base = r2hsl(h2r(p[3] || '#0078D4'));
    var acc = light ? (p[4] || '#0067C0') : (p[1] || '#4CC2FF');
    return {
      k: 'win', n: 'Як у Windows', light: light, a: acc, b: r2h(hsl2r((base[0] + .5) % 1, .5, light ? .42 : .72)),
      s: '#202020', s1: '#2B2B2B', s2: '#2C2C2C', sd: '#373737', tr: '#4A4A4A',
      lt: { s: '#F3F3F3', s1: '#FBFBFB', s2: '#F9F9F9', sd: '#EAEAEA', tr: '#CFCFCF' },
      fill: acc, ink: light ? '#FFFFFF' : '#000000', sw: 'a'
    };
  }
  // OLED: the light shade of the Windows accent and the opposite hue on pure black surfaces
  function oledTheme() {
    var w = (S && S.win) || {}, p = w.pal || [], base = r2hsl(h2r(p[3] || '#0078D4')), acc = p[1] || '#4CC2FF';
    return { k: 'black', n: 'Чорна (OLED)', a: acc, b: r2h(hsl2r((base[0] + .5) % 1, .5, .72)), s: '#000000', s1: '#111113', s2: '#1C1C1F', sd: '#18181B', fill: acc, ink: '#000000', badge: 'OLED', sw: 'a' };
  }
  function themeOf(cfg) {
    var t = THEMES.filter(function (x) { return x.k === cfg.theme; })[0] || THEMES[0];
    if (t.k === 'win') t = winTheme();
    if (t.k === 'black') t = oledTheme();
    if (t.k === 'custom') {
      t = { k: 'custom', n: 'Власна', a: cfg.customA || '#FF6A3D', b: cfg.customB || '#3DC8FF', bg: cfg.customS || '' };
      if (cfg.customOled) { t.s = '#000000'; t.s1 = '#111113'; t.s2 = '#1C1C1F'; t.sd = '#18181B'; } // dark mode only
    }
    return t;
  }
  var TOK = new URLSearchParams(location.search).get('k') || '';
  var $ = function (id) { return document.getElementById(id); };
  var S = null, TK = null, tab = 'mix';

  function api(path, body) {
    return fetch(path + (path.indexOf('?') < 0 ? '?' : '&') + 'k=' + TOK, body === undefined ? {} : { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  }
  function patch(o) { api('/api/cfg', o); }
  function esc(s) { return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; }); }
  // ---------- icons (Lucide, ISC) for card labels and setting rows ----------
  var IC = {
    user: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
    sliders: '<path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M2 14h4M10 8h4M18 16h4"/>',
    headphones: '<path d="M3 14v-2a9 9 0 0 1 18 0v2"/><rect x="2.5" y="14" width="5" height="7" rx="2"/><rect x="16.5" y="14" width="5" height="7" rx="2"/>',
    route: '<circle cx="6" cy="19" r="3"/><path d="M9 19h8.5a3.5 3.5 0 0 0 0-7h-11a3.5 3.5 0 0 1 0-7H15"/><circle cx="18" cy="5" r="3"/>',
    duck: '<path d="M11 5 6 9H2v6h4l5 4z"/><path d="M15.5 8.5a5 5 0 0 1 0 7"/>',
    ear: '<path d="M6 8.5a6.5 6.5 0 1 1 13 0c0 6-6 6-6 10a3.5 3.5 0 1 1-7 0"/><path d="M15 8.5a2.5 2.5 0 0 0-5 0v1a2 2 0 1 1 0 4"/>',
    wand: '<path d="M15 4V2M15 16v-2M8 9h2M20 9h2M17.8 11.8 19 13M17.8 6.2 19 5M3 21l9-9M12.2 6.2 11 5"/>',
    micoff: '<path d="M2 2l20 20M18.9 13.4A7 7 0 0 0 19 12v-2M5 10v2a7 7 0 0 0 12 5M15 9.3V5a3 3 0 0 0-5.7-1.3M9 9v3a3 3 0 0 0 5.1 2.1M12 19v3"/>',
    mic: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21"/>',
    window: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M2 9h20"/>',
    speaker: '<path d="M11 5 6 9H2v6h4l5 4z"/><path d="M15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14"/>',
    battery: '<rect x="2" y="7" width="16" height="10" rx="2"/><path d="M22 11v2M6 11v2"/>',
    batfull: '<rect x="2" y="7" width="16" height="10" rx="2"/><path d="M22 11v2M6 10v4M10 10v4M14 10v4"/>',
    toggle: '<rect x="1" y="5" width="22" height="14" rx="7"/><circle cx="16" cy="12" r="3"/>',
    keyboard: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M6 8h.01M10 8h.01M14 8h.01M18 8h.01M8 12h.01M12 12h.01M16 12h.01M7 16h10"/>',
    help: '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="4"/><path d="m4.9 4.9 4.3 4.3M14.8 14.8l4.3 4.3M14.8 9.2l4.3-4.3M4.9 19.1l4.3-4.3"/>',
    palette: '<circle cx="13.5" cy="6.5" r="1"/><circle cx="17.5" cy="10.5" r="1"/><circle cx="8.5" cy="7.5" r="1"/><circle cx="6.5" cy="12.5" r="1"/><path d="M12 2a10 10 0 0 0 0 20c.9 0 1.6-.7 1.6-1.7 0-.4-.2-.8-.4-1.1-.3-.3-.4-.7-.4-1.1 0-.9.7-1.7 1.7-1.7h2a5.6 5.6 0 0 0 5.5-5.5C22 6 17.5 2 12 2z"/>',
    power: '<path d="M12 2v10M18.4 6.6a9 9 0 1 1-12.8 0"/>',
    globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>',
    monitor: '<rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/>',
    pin: '<path d="M12 17v5M9 10.8V4h6v6.8l2 2.2v2H7v-2z"/>',
    msg: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>',
    corner: '<path d="M3 9V3h6M21 15v6h-6"/>',
    bell: '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10.3 21a1.9 1.9 0 0 0 3.4 0"/>',
    refresh: '<path d="M21 12a9 9 0 0 1-15 6.7L3 16M3 12a9 9 0 0 1 15-6.7L21 8M21 3v5h-5M3 21v-5h5"/>',
    list: '<path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/>',
    sparkles: '<path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"/>',
    moon: '<path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"/>',
    mouse: '<rect x="6" y="3" width="12" height="18" rx="6"/><path d="M12 7v4"/>',
    bt: '<path d="m7 7 10 10-5 5V2l5 5L7 17"/>',
    search: '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>',
    layers: '<path d="m12 3 9 4.5-9 4.5-9-4.5z"/><path d="m3 12 9 4.5 9-4.5"/><path d="m3 16.5 9 4.5 9-4.5"/>',
    trash: '<path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M10 11v6"/><path d="M14 11v6"/>'
  };
  function icon(k, size) { return '<svg class="tic" width="' + (size || 14) + '" height="' + (size || 14) + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + IC[k] + '</svg>'; }
  // a title whose text changes keeps its icon
  function setTtl(id, text) { var e = $(id); if (e.dataset.t === text) return; e.dataset.t = text; e.innerHTML = icon(e.dataset.ic, 13) + esc(text); }
  document.querySelectorAll('.ttl[data-ic]').forEach(function (t) { t.insertAdjacentHTML('afterbegin', icon(t.dataset.ic, 13)); });
  // every setting row: an icon tile that says what it is about
  var ROWIC = { swAuto: 'power', langSel: 'globe', swWinCtl: 'monitor', swTaskbar: 'pin', swOsd: 'msg', osdCorner: 'corner', swSounds: 'bell', swMicHk: 'keyboard',
    swPtt: 'mic', swLow: 'battery', swFull: 'batfull', swSwitch: 'refresh', swCables: 'route', swOnLost: 'refresh', swSwitchMic: 'mic', outOrder: 'list', swMfx: 'wand', swDenoise: 'sparkles', swComp: 'sliders',
    swMonLive: 'headphones', swRt: 'route', swOled: 'moon', swDuckMedia: 'duck', wheelSeg: 'mouse' };
  Object.keys(ROWIC).forEach(function (id) {
    var el = $(id), row = el && el.closest('.row');
    if (row && !row.querySelector('.ri')) row.insertAdjacentHTML('afterbegin', '<span class="ri">' + icon(ROWIC[id], 15) + '</span>');
  });
  // ---------- «Функції»: вимкнена функція не видна й не працює ----------
  var FEATS = [
    ['eq', 'Еквалайзер', 'гарнітури й вбудовані в неї пресети'],
    ['profiles', 'Профілі', 'набори налаштувань звуку, перемикання з трею'],
    ['volmem', 'Пам’ять гучності', 'повертає гучність програм і мікрофона, якщо її змінить Windows чи інша програма'],
    ['vst', 'VST-плагіни для мікрофона', 'через віртуальний кабель'],
    ['denoise', 'Шумодав', 'прибирає клавіатуру, вентилятор, вуличний шум · через віртуальний кабель'],
    ['duck', 'Автоприглушення гри', 'коли говорять у чаті'],
    ['monitor', 'Послухати себе', 'запис і прослуховування свого мікрофона'],
    ['route', 'Маршрутизація звуку', 'окремий еквалайзер гри, чату й медіа через віртуальні кабелі']
  ];
  var FEATIC = { eq: 'sliders', profiles: 'user', volmem: 'refresh', vst: 'wand', denoise: 'sparkles', duck: 'duck', monitor: 'ear', route: 'route' };
  var FEAT_BASIC = { eq: 1, profiles: 1, volmem: 1, denoise: 1, duck: 1 };
  function feat(k) { var f = S && S.cfg && S.cfg.features; return !f || f[k] !== false; }
  function featInit() {
    var h = '';
    FEATS.forEach(function (f) {
      h += '<div class="row"><span class="ri">' + icon(FEATIC[f[0]] || 'toggle', 15) + '</span><div class="grow"><div>' + esc(tx(f[1])) + '</div><div class="sub">' + esc(tx(f[2])) + '</div></div><button class="sw" data-f="' + f[0] + '" role="switch" aria-label="' + esc(tx(f[1])) + '"><i></i></button></div>';
    });
    $('featList').innerHTML = h;
    $('featList').querySelectorAll('[data-f]').forEach(function (b) {
      b.onclick = function () { var m = {}; m[b.dataset.f] = !feat(b.dataset.f); patch({ features: m }); };
    });
    $('featSet').querySelectorAll('button').forEach(function (b) {
      b.onclick = function () { var m = {}; FEATS.forEach(function (f) { m[f[0]] = b.dataset.v === 'all' || !!FEAT_BASIC[f[0]]; }); patch({ features: m }); };
    });
  }
  function renderFeatures(c) {
    var all = true, basic = true;
    FEATS.forEach(function (f) {
      var on = feat(f[0]);
      $('featList').querySelector('[data-f="' + f[0] + '"]').setAttribute('aria-checked', on ? 'true' : 'false');
      if (!on) all = false;
      if (on !== !!FEAT_BASIC[f[0]]) basic = false;
    });
    pressed('featSet', all ? 'all' : (basic ? 'basic' : ''));
    var off = function (id, hide) { $(id).classList.toggle('foff', !!hide); };
    var mfx = feat('denoise') || feat('vst');
    off('eqCard', !feat('eq')); off('hwPresetCard', !feat('eq'));
    off('profCard', !feat('profiles'));
    off('duckCard', !feat('duck')); off('qDuck', !feat('duck'));
    off('qNs', !feat('denoise')); off('denoiseRow', !feat('denoise')); off('dnBox', !feat('denoise'));
    off('qRt', !feat('route'));
    off('quickRow', !feat('duck') && !feat('denoise') && !feat('route'));
    // an odd tile at the end takes the whole row
    var qv = [].filter.call($('quickRow').children, function (t) { return !t.classList.contains('foff'); });
    qv.forEach(function (t, i) { t.classList.toggle('wide', qv.length % 2 === 1 && i === qv.length - 1); });
    off('profBtn', !feat('profiles'));
    off('vstBox', !feat('vst'));
    off('mfxCard', !mfx);
    off('monCard', !feat('monitor'));
    off('rtCard', !feat('route')); off('rtEntry', !feat('route'));
    var snd = feat('eq') || feat('profiles') || feat('duck') || feat('route');
    document.querySelector('#nav [data-tab=sound]').classList.toggle('foff', !snd);
    if (!snd && tab === 'sound') show('mix');
    $('featWarn').classList.toggle('hide', mfx || !(c.micFx && c.micFx.on));
  }
  featInit();
  function setSw(id, on) { $(id).setAttribute('aria-checked', on ? 'true' : 'false'); }
  function pressed(boxId, val) { $(boxId).querySelectorAll('button').forEach(function (b) { b.setAttribute('aria-pressed', String(b.dataset.v) === String(val)); }); }
  var darkMQ = window.matchMedia ? matchMedia('(prefers-color-scheme: light)') : null;
  function modeOf(cfg) {
    if (cfg.theme === 'win' && S && S.win) return S.win.light ? 'light' : 'dark'; // режим Windows
    if (cfg.theme === 'black') return 'dark'; // OLED is dark by definition
    return cfg.mode === 'auto' ? (darkMQ && darkMQ.matches ? 'light' : 'dark') : (cfg.mode === 'light' ? 'light' : 'dark');
  }
  var uiSent = '';
  function applyTheme(cfg) {
    var mode = modeOf(cfg);
    TK = makeTokens(themeOf(cfg), mode);
    var st = document.documentElement.style;
    for (var k in TK) st.setProperty('--' + k, TK[k]);
    st.colorScheme = mode;
    // card, text, muted text, frame, hover, accent
    var ui = [TK.s1, TK.on, TK.ov, TK.tr, TK.s2, TK.g];
    if (ui.join() !== uiSent) { uiSent = ui.join(); patch({ uiColors: ui }); }
  }
  if (darkMQ && darkMQ.addEventListener) darkMQ.addEventListener('change', function () { if (S) applyTheme(S.cfg); });

  // рамку фокуса показуємо лише тим, хто керує з клавіатури (WebView2 інколи малює її й після кліку)
  document.addEventListener('keydown', function (e) { if (/^(Tab|Arrow|Home|End|Page)/.test(e.key)) document.documentElement.classList.add('kb'); }, true);
  document.addEventListener('pointerdown', function () { document.documentElement.classList.remove('kb'); }, true);

  // ---------- повзунок: однаковий плавний хід скрізь ----------
  // клік, колесо, клавіші й зміни ззовні — анімовано; під час перетягування — точно за пальцем
  function Slider(el, o) {
    el.classList.add('sl'); if (o.cls) el.classList.add(o.cls);
    el.tabIndex = 0; el.setAttribute('role', 'slider');
    el.setAttribute('aria-valuemin', o.min); el.setAttribute('aria-valuemax', o.max);
    el.innerHTML = '<div class="sl-tr"></div><div class="sl-fi"></div><div class="sl-th"></div>';
    var fi = el.children[1], th = el.children[2], drag = false, v = o.value === undefined ? o.min : o.value, self = { el: el, track: el.children[0] };
    function draw(anim) {
      var p = (v - o.min) / (o.max - o.min) * 100;
      el.classList.toggle('anim', !!anim);
      fi.style.width = p + '%'; th.style.left = p + '%';
      el.setAttribute('aria-valuenow', v);
    }
    function clampv(x) { x = Math.round((x - o.min) / o.step) * o.step + o.min; return Math.max(o.min, Math.min(o.max, +x.toFixed(4))); }
    function setv(x, anim) { x = clampv(x); var ch = x !== v; v = x; draw(anim); if (ch && o.onInput) o.onInput(v); }
    function rel(e) { var r = el.getBoundingClientRect(); return o.min + (e.clientX - r.left) / r.width * (o.max - o.min); }
    function done() { if (o.onChange) o.onChange(v); }
    el.addEventListener('pointerdown', function (e) { if (e.button) return; el.focus({ preventScroll: true }); el.setPointerCapture(e.pointerId); drag = true; setv(rel(e), true); });
    el.addEventListener('pointermove', function (e) { if (drag) setv(rel(e), false); });
    el.addEventListener('pointerup', function () { if (drag) { drag = false; done(); } });
    el.addEventListener('lostpointercapture', function () { if (drag) { drag = false; done(); } });
    el.addEventListener('wheel', function (e) { if (document.activeElement !== el) return; e.preventDefault(); setv(v + (e.deltaY < 0 ? 1 : -1) * (o.wheel || o.step), true); done(); }, { passive: false });
    el.addEventListener('keydown', function (e) {
      var d = { ArrowLeft: -1, ArrowDown: -1, ArrowRight: 1, ArrowUp: 1 }[e.key];
      if (d) { e.preventDefault(); setv(v + d * (o.key || o.step), true); done(); }
    });
    self.set = function (x) { if (drag) return; x = clampv(x); if (x !== v) { v = x; draw(true); } };
    self.get = function () { return v; };
    self.busy = function () { return drag; };
    draw(false);
    return self;
  }
  function throttle(f, ms) { var t = 0, last = 0, arg; return function (a, final) { arg = a; var now = Date.now(); clearTimeout(t); if (final || now - last > ms) { last = now; f(arg); } else t = setTimeout(function () { last = Date.now(); f(arg); }, ms); }; }

  // ---------- навігація ----------
  var VIEWS = ['mix', 'sound', 'micTab', 'settings'];
  function show(v, anchor) {
    var prev = navState();
    inShow = true;
    // the tab's button again, or a jump to a block of its first page: back to that first page
    if (PG[v] && (v === tab || anchor)) setPage(v, '');
    if (palOpen()) palClose();
    tab = v;
    VIEWS.forEach(function (id) { $(id).classList.toggle('off', id !== v); });
    $('nav').querySelectorAll('button').forEach(function (b) { if (b.dataset.tab === v) b.setAttribute('aria-current', 'page'); else b.removeAttribute('aria-current'); });
    closePops();
    if (v !== 'mix') loadDevs();
    if (anchor && $(anchor)) setTimeout(function () { $(anchor).scrollIntoView({ behavior: 'smooth', block: 'start' }); }, 60);
    pollers();
    inShow = false;
    navPush(prev);
  }
  $('nav').querySelectorAll('button').forEach(function (b) { b.onclick = function () { show(b.dataset.tab); }; });
  document.addEventListener('keydown', function (e) {
    if (e.key !== 'Escape' || capKind) return;
    if (palOpen()) palBackStep(); else if (anyPop()) closePops(); else if (PG[tab]) setPage(tab, ''); else if (tab !== 'mix') show('mix'); else api('/api/hide', {});
  });
  // ---------- pages inside a tab (Sound, Settings): a "‹" header and only that page's blocks (data-pg);
  //            "" is the tab's own first page ----------
  var PG = {}, PGT = { order: 'Порядок обробки', cables: 'Кабелі', dev: 'Пристрої', hs: 'Гарнітура', keys: 'Гарячі клавіші', look: 'Вигляд', gen: 'Загальне', about: 'Про програму', help: 'Налаштування й допомога' };
  function setPage(view, pg) {
    var v = $(view), h = v.querySelector('.phead'), prev = navState();
    PG[view] = pg || '';
    v.querySelectorAll('[data-pg]').forEach(function (el) { el.classList.toggle('pgoff', el.dataset.pg !== PG[view]); });
    // a wrapper of blocks (the settings groups) with none of this page's: out too, or its gap stays
    [].forEach.call(v.children, function (w) { if (w.dataset.pg === undefined && !w.classList.contains('phead') && w.querySelector('[data-pg]')) w.classList.toggle('pgoff', !w.querySelector('[data-pg="' + PG[view] + '"]')); });
    h.classList.toggle('hide', !pg);
    if (pg) h.querySelector('.pt').textContent = pg === 'hs' && S && S.device && S.device.name ? S.device.name : tx(PGT[pg]);
    v.scrollTop = 0;
    if (!inShow && view === tab) navPush(prev);
  }
  // ---------- back / forward (mouse buttons 4 and 5): the tabs and pages visited, as in a browser ----------
  var HIST = [], FWD = [], inShow = false, navMute = false;
  function navState() { return { tab: tab, pg: PG[tab] || '' }; }
  function navPush(prev) {
    var cur = navState();
    if (navMute || (prev.tab === cur.tab && prev.pg === cur.pg)) return;
    HIST.push(prev); if (HIST.length > 40) HIST.shift();
    FWD = [];
  }
  function navGo(st) {
    navMute = true;
    if (st.tab !== tab) show(st.tab);
    if ((PG[st.tab] || '') !== st.pg) setPage(st.tab, st.pg);
    navMute = false;
  }
  function navBack() {
    if (palOpen()) { palBackStep(); return; }
    if (anyPop()) { closePops(); return; }
    var p = HIST.pop();
    if (p) { FWD.push(navState()); navGo(p); }
    else if (PG[tab]) setPage(tab, '');
    else if (tab !== 'mix') show('mix');
  }
  function navFwd() { var f = FWD.pop(); if (f) { HIST.push(navState()); navGo(f); } }
  // the default of these buttons in a web view would be page history: here they move through the window
  ['mousedown', 'mouseup'].forEach(function (t) {
    window.addEventListener(t, function (e) {
      if (e.button !== 3 && e.button !== 4) return;
      e.preventDefault();
      if (t === 'mouseup') { if (e.button === 3) navBack(); else navFwd(); }
    });
  });
  document.querySelectorAll('.phead .pback').forEach(function (b) { b.onclick = function () { setPage(b.closest('.view').id, ''); }; });
  document.querySelectorAll('[data-page]').forEach(function (b) { b.onclick = function () { setPage(b.closest('.view').id, b.dataset.page); }; });
  // the page rows: an icon tile each, as the setting rows have (About — the app mark)
  var PGIC = { order: 'list', cables: 'route', dev: 'speaker', hs: 'headphones', keys: 'keyboard', look: 'palette', gen: 'toggle', help: 'help' };
  document.querySelectorAll('[data-page]').forEach(function (b) {
    var ic = b.dataset.page === 'about' ? '<svg class="tic" width="15" height="15" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 21 20Q12 14 3 20Z" fill="currentColor"/></svg>' : icon(PGIC[b.dataset.page], 15);
    b.insertAdjacentHTML('afterbegin', '<span class="ri">' + ic + '</span>');
  });
  setPage('sound', ''); setPage('settings', ''); setPage('micTab', '');
  $('pinBtn').onclick = function () { if (S) patch({ keepOpen: !S.cfg.keepOpen }); };
  document.querySelectorAll('[data-url]').forEach(function (b) { b.onclick = function () { api('/api/open', { url: b.dataset.url }); }; });

  // ---------- баланс гра / чат ----------
  var pos = 0, dragging = false, lastSend = 0, xf = $('xf'), cap = $('xfCap');
  (function () {
    var h = '';
    for (var i = 0; i <= 8; i++) h += '<div class="xf-tk' + (i === 4 ? ' c' : '') + '" style="left:calc(4px + (100% - 8px) * ' + (i / 8) + ')"></div>';
    $('xfTicks').innerHTML = h;
  })();
  function drawPos(p, animate) {
    xf.classList.toggle('anim', !!animate);
    cap.style.left = 'calc(4px + (100% - 8px) * ' + ((p + 100) / 200) + ')';
    var g = 100 - Math.max(0, p), c = 100 - Math.max(0, -p), bal = Math.abs(p) < 4;
    cap.classList.toggle('bal', bal);
    xf.setAttribute('aria-valuenow', p);
    $('gPct').textContent = g; $('cPct').textContent = c;
    $('gPct').style.opacity = .45 + .55 * g / 100; $('cPct').style.opacity = .45 + .55 * c / 100;
    // лінія гри йде від лівого краю, чату — від правого; тихіша сторона відступає до свого краю
    $('xfLL').style.width = 'calc((100% - 8px) * ' + (g / 200) + ')'; $('xfLR').style.width = 'calc((100% - 8px) * ' + (c / 200) + ')';
    $('xfGL').style.width = (g / 2) + '%'; $('xfGR').style.width = (c / 2) + '%';
    // balanced: the hotkeys; otherwise a button back to the middle
    $('balChip').classList.toggle('hide', bal);
    $('balKeys').classList.toggle('hide', !bal);
  }
  function setPos(p, send) {
    p = Math.max(-100, Math.min(100, Math.round(p)));
    pos = p; drawPos(p, !dragging);
    var now = Date.now();
    if (send && (now - lastSend > 40 || !dragging)) { lastSend = now; api('/api/pos', { v: p }); }
  }
  // позиція під курсором: край лінії, що рухається, іде за курсором (ліва половина — кінець лінії
  // гри, тобто більше чату; права — початок лінії чату, тобто більше гри)
  function relPos(e) {
    var r = xf.getBoundingClientRect(), p = 100 - ((e.clientX - r.left - 4) / (r.width - 8)) * 200;
    p = Math.max(-100, Math.min(100, p));
    return Math.abs(p) < 5 ? 0 : Math.round(p);
  }
  xf.addEventListener('pointerdown', function (e) { xf.setPointerCapture(e.pointerId); dragging = true; setPos(relPos(e), true); });
  xf.addEventListener('pointermove', function (e) { if (dragging) setPos(relPos(e), true); });
  xf.addEventListener('pointerup', function () { if (dragging) { dragging = false; api('/api/pos', { v: pos }); } });
  xf.addEventListener('dblclick', function () { setPos(0, true); });
  xf.addEventListener('wheel', function (e) { e.preventDefault(); setPos(pos + (e.deltaY < 0 ? 5 : -5), true); }, { passive: false });
  xf.addEventListener('keydown', function (e) {
    if (e.key === 'ArrowLeft') setPos(pos + 10, true); else if (e.key === 'ArrowRight') setPos(pos - 10, true); else if (e.key === 'ArrowDown' || e.key === 'Home') setPos(0, true);
  });
  $('balChip').onclick = function () { setPos(0, true); };
  // the balance hotkeys, as set in the settings: mods once, then both arrows
  function renderBalKeys(c) {
    var hk = c.hotkeys || {}, g = hk.game, ch = hk.chat, h = '';
    if (g && g.vk && ch && ch.vk) {
      var a = comboKeys(g), b = comboKeys(ch);
      var same = a.length === b.length && a.slice(0, -1).join('+') === b.slice(0, -1).join('+');
      // the arrows in the order they stand on the keyboard: ← before →
      var ends = [a[a.length - 1], b[b.length - 1]].sort(function (x, y) { return (x === '→') - (y === '→'); });
      h = same ? a.slice(0, -1).concat([ends.join(' ')]).map(function (k) { return '<kbd>' + esc(k) + '</kbd>'; }).join('') : '';
    }
    $('balKeys').innerHTML = h;
  }

  // ---------- мікрофон (головна) ----------
  var micSl = Slider($('micVol'), { min: 0, max: 100, step: 1, wheel: 2, key: 5, onInput: throttle(function (v) { api('/api/mic', { level: v }); }, 60), onChange: function (v) { api('/api/mic', { level: v }); } });
  function drawMic(m) {
    var mute = !!m.mute;
    micSl.el.classList.toggle('mute', mute);
    micSl.set(m.level || 0);
    $('micPct').textContent = mute ? tx('Вимк.') : (m.level || 0);
    $('micSlash').style.display = mute ? '' : 'none';
    $('micMute').style.color = mute ? 'var(--red)' : '';
  }
  $('micVol').addEventListener('pointermove', function () { if (micSl.busy()) $('micPct').textContent = micSl.get(); });
  $('micMute').onclick = function () { if (S) api('/api/mic', { mute: !S.mic.mute }); };
  // ---------- the Windows output (main screen): device and its Windows volume ----------
  var outSl = Slider($('outVol'), { min: 0, max: 100, step: 1, wheel: 2, key: 5, onInput: throttle(function (v) { api('/api/outvol', { vol: v }); }, 60), onChange: function (v) { api('/api/outvol', { vol: v }); } });
  function drawOut(o) {
    var mute = !!o.mute;
    outSl.el.classList.toggle('mute', mute);
    if (!outSl.busy()) outSl.set(o.vol || 0);
    $('outPct').textContent = mute ? tx('Вимк.') : (o.vol || 0);
    $('outSlash').style.display = mute ? '' : 'none';
    $('outMute').style.color = mute ? 'var(--red)' : '';
  }
  $('outVol').addEventListener('pointermove', function () { if (outSl.busy()) $('outPct').textContent = outSl.get(); });
  $('outMute').onclick = function () { if (S) api('/api/outvol', { mute: !(S.out || {}).mute }); };
  // ---------- пристрій виводу Windows (картка гарнітури на головному екрані) ----------
  function renderOut(s) {
    var o = s.out || {};
    $('devName').textContent = o.short || o.name || tx('Пристрій виводу Windows');
    $('outDev').title = o.name || tx('Пристрій виводу Windows');
    var conn = { '24g': tx('2,4 ГГц'), bt: 'Bluetooth', wire: tx('провід'), virtual: tx('віртуальний'), off: tx('вимкнена') }[o.conn] || '';
    $('connLbl').textContent = conn;
    $('connLbl').classList.toggle('hide', !conn);
    // the battery of the device Windows plays to: the headset's own, or what Windows knows of Bluetooth headphones
    $('bchip').classList.toggle('hide', !(o.headset ? s.connected && s.powered && s.haveBattery : o.batt >= 0));
    // the picture of the card: headphones for a headset or Bluetooth / 2.4 GHz, a speaker otherwise
    var hp = o.headset || o.conn === 'bt' || o.conn === '24g' || o.conn === 'off';
    if ($('heroIcon').dataset.k !== (hp ? 'h' : 's')) { $('heroIcon').dataset.k = hp ? 'h' : 's'; $('heroIcon').innerHTML = icon(hp ? 'headphones' : 'speaker', 16); }
  }
  // ---------- пристрої виводу (попап у шапці): вибір, порядок перетягуванням, вмикання в Windows ----------
  var OUTS = [], outSig = '', outDrag = null;
  var OUTST = { unplugged: 'Не під’єднано', absent: 'Не під’єднано', hsoff: 'Гарнітуру вимкнено' };
  var OUTGR = [['on', 'Увімкнені'], ['gone', 'Від’єднані'], ['disabled', 'Вимкнені в Windows']];
  var chevSvg = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 9l6 6 6-6"/></svg>';
  // collapsed groups of the priority view (disconnected, disabled), remembered
  var outOpen = {}; // groups of devices that are not on: collapsed unless opened (remembered)
  try { outOpen = JSON.parse(localStorage.getItem('outOpen') || '{}') || {}; } catch (x) { }
  var gripSvg = '<svg width="12" height="16" viewBox="0 0 12 16" fill="currentColor" aria-hidden="true"><circle cx="3.5" cy="3" r="1.4"/><circle cx="8.5" cy="3" r="1.4"/><circle cx="3.5" cy="8" r="1.4"/><circle cx="8.5" cy="8" r="1.4"/><circle cx="3.5" cy="13" r="1.4"/><circle cx="8.5" cy="13" r="1.4"/></svg>';
  var xSvg = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18"/></svg>';
  function outGroup(d) { return d.state === 'on' ? 'on' : (d.state === 'disabled' ? 'disabled' : 'gone'); }
  // i: the place in its group; the enabled ones carry their priority number, 1 — the highest
  function outRow(d, i) {
    var st = d.default ? tx('Зараз грає') : (OUTST[d.state] ? tx(OUTST[d.state]) : ''), grip = tx('Перетягніть, щоб змінити пріоритет');
    return '<div class="od' + (d.default ? ' cur' : '') + '" data-id="' + esc(d.id) + '">' +
      '<button class="dh" data-act="grip" title="' + grip + '" aria-label="' + grip + ': ' + esc(d.name) + '">' + gripSvg + '</button>' +
      (d.state === 'on' ? '<span class="pn">' + (i + 1) + '</span>' : '') +
      '<button class="dn" data-act="pick"' + (d.state === 'on' ? '' : ' data-off') + ' title="' + esc(d.name) + '"><span class="t">' + esc(d.name) + '</span>' + (st ? '<span class="s">' + esc(st) + '</span>' : '') + '</button>' +
      // від'єднаний пристрій вимкнути не можна — на місці перемикача «прибрати зі списку»
      (d.state === 'absent' ? '<button class="ib dx" data-act="forget" title="' + tx('Прибрати') + '" aria-label="' + tx('Прибрати') + '">' + xSvg + '</button>' :
        '<button class="sw" role="switch" data-act="' + (d.state === 'disabled' ? 'on' : 'off') + '" aria-checked="' + (d.state !== 'disabled') + '" title="' + tx('Увімкнено в Windows') + '" aria-label="' + tx('Увімкнено в Windows') + ': ' + esc(d.name) + '"><i></i></button>') +
      '</div>';
  }
  function outPopRender(force) {
    var p = $('palL');
    if (!palOpen() || palMode() !== 'prio' || outDrag) return; // not shown, or a row is being dragged
    var sig = JSON.stringify([OUTS, !!(S && S.cfg.autoSwitch), I18.lang]);
    if (!force && sig === outSig) return;
    outSig = sig;
    var a = document.activeElement, row = a && p.contains(a) && a.closest('.od'), fk = row ? row.dataset.id + '|' + (a.dataset.act || '') : (a && a.id === 'outAuto' ? 'auto' : '');
    // a priority list: the switch sends the sound to the highest connected device
    var h = '<div class="hd" style="color:var(--on);font-weight:600">' + esc(tx('Пріоритет пристроїв виводу')) + '</div>' +
      '<div class="hd" style="padding-top:0">' + esc(tx('Звук іде на найвищий у списку під’єднаний пристрій. Перетягніть ⠿, щоб змінити пріоритет.')) + '</div>' +
      '<div class="oauto"><span>' + tx('Перемикати звук автоматично') + '</span><button class="sw" role="switch" id="outAuto" aria-checked="' + !!(S && S.cfg.autoSwitch) + '" aria-label="' + tx('Перемикати звук автоматично') + '"><i></i></button></div>';
    if (!OUTS.length) h += '<div class="hd">' + tx('Шукаю пристрої…') + '</div>';
    OUTGR.forEach(function (g) {
      var l = OUTS.filter(function (d) { return outGroup(d) === g[0]; });
      if (!l.length) return;
      if (g[0] === 'on') { h += '<div class="hd og"><span>' + tx(g[1]) + '</span></div><div class="ol" data-g="on">' + l.map(outRow).join('') + '</div>'; return; }
      var open = !!outOpen[g[0]];
      h += '<button class="hd og ogt" data-g="' + g[0] + '" aria-expanded="' + open + '"><span>' + tx(g[1]) + '</span><span class="n">' + l.length + '</span>' + chevSvg + '</button>' +
        (open ? '<div class="ol" data-g="' + g[0] + '">' + l.map(outRow).join('') + '</div>' : '');
    });
    var top = p.scrollTop;
    p.innerHTML = h;
    p.scrollTop = top;
    $('outAuto').onclick = function () { var on = !(S && S.cfg.autoSwitch); this.setAttribute('aria-checked', on); patch({ autoSwitch: on }); };
    p.querySelectorAll('.ogt').forEach(function (b) {
      b.onclick = function () {
        var g = b.dataset.g;
        outOpen[g] = !outOpen[g];
        try { localStorage.setItem('outOpen', JSON.stringify(outOpen)); } catch (x) { }
        outPopRender(true);
        p.querySelector('.ogt[data-g="' + g + '"]').focus({ preventScroll: true });
      };
    });
    p.querySelectorAll('.od [data-act]').forEach(function (b) {
      var id = b.closest('.od').dataset.id, act = b.dataset.act;
      if (act === 'grip') { outGrip(b); return; }
      b.onclick = function () {
        // the view stays open: it is for arranging; a click on a name makes that device the output
        if (act === 'pick') { if (!b.hasAttribute('data-off')) api('/api/output', { dev: id }); return; }
        if (act === 'forget') { OUTS = OUTS.filter(function (d) { return d.id !== id; }); outPopRender(true); }
        else b.setAttribute('aria-checked', act === 'on');
        api('/api/outdev', { op: act, id: id });
      };
    });
    if (fk === 'auto') $('outAuto').focus({ preventScroll: true });
    else if (fk) {
      var f = fk.split('|');
      p.querySelectorAll('.od').forEach(function (r) {
        var btn = r.dataset.id === f[0] && (r.querySelector('[data-act="' + f[1] + '"]') || r.querySelector('.sw,.dx'));
        if (btn) btn.focus({ preventScroll: true });
      });
    }
  }
  // новий порядок групи g (ids) — на свої місця в загальному списку пріоритету
  function outCommit(g, ids) {
    var by = {}, k = 0;
    OUTS.forEach(function (d) { by[d.id] = d; });
    OUTS = OUTS.map(function (d) { return outGroup(d) === g ? by[ids[k++]] : d; });
    outPopRender(true);
    api('/api/outdev', { op: 'order', ids: OUTS.map(function (d) { return d.id; }) });
  }
  function groupIds(list) { return [].map.call(list.children, function (r) { return r.dataset.id; }); }
  // перетягування за ручку: рядок іде за курсором, сусіди плавно розступаються
  function outGrip(h) {
    var row = h.closest('.od'), list = row.parentNode, p = $('palL');
    h.addEventListener('keydown', function (e) { // з клавіатури: ↑ / ↓
      var d = { ArrowUp: -1, ArrowDown: 1 }[e.key];
      if (!d) return;
      e.preventDefault();
      var ids = groupIds(list), i = ids.indexOf(row.dataset.id), j = i + d;
      if (j < 0 || j >= ids.length) return;
      ids[i] = ids[j]; ids[j] = row.dataset.id;
      outCommit(list.dataset.g, ids);
    });
    h.addEventListener('pointerdown', function (e) {
      if (e.button) return;
      e.preventDefault();
      h.setPointerCapture(e.pointerId);
      outDrag = { before: groupIds(list).join('|'), grab: e.clientY - row.getBoundingClientRect().top, t: 0 };
      row.classList.add('lift');
    });
    h.addEventListener('pointermove', function (e) {
      if (!outDrag) return;
      var pr = p.getBoundingClientRect(), y = e.clientY - pr.top - p.clientTop + p.scrollTop; // у координатах попапу
      // the place: before the first neighbour whose middle is below the middle of the dragged row
      var yc = y - outDrag.grab + row.offsetHeight / 2;
      var rows = [].slice.call(list.children), target = null;
      for (var k = 0; k < rows.length; k++) {
        if (rows[k] !== row && yc < rows[k].offsetTop + rows[k].offsetHeight / 2) { target = rows[k]; break; }
      }
      if ((target === null && list.lastElementChild !== row) || (target && row.nextElementSibling !== target)) {
        var pos = rows.map(function (r) { return r.getBoundingClientRect().top; });
        // the neighbours move, not the dragged row: taking it out of the page would end its pointer capture
        // and with it the drag, after one step
        var down = target === null || rows.indexOf(target) > rows.indexOf(row);
        while (row.nextElementSibling !== target) {
          if (down) list.insertBefore(row.nextElementSibling, row);
          else list.insertBefore(row.previousElementSibling, row.nextElementSibling);
        }
        rows.forEach(function (r, i) {
          if (r === row) return;
          var d = pos[i] - r.getBoundingClientRect().top;
          if (!d) return;
          r.style.transition = 'none'; r.style.transform = 'translateY(' + d + 'px)';
          requestAnimationFrame(function () { r.style.transition = 'transform .16s var(--ease)'; r.style.transform = ''; });
        });
      }
      var lo = list.offsetTop - row.offsetTop, hi = list.offsetTop + list.offsetHeight - row.offsetHeight - row.offsetTop;
      outDrag.t = Math.max(lo, Math.min(hi, y - outDrag.grab - row.offsetTop));
      row.style.transition = 'none'; row.style.transform = 'translateY(' + outDrag.t + 'px)';
      if (e.clientY < pr.top + 28) p.scrollTop -= 8; else if (e.clientY > pr.bottom - 28) p.scrollTop += 8;
    });
    function end() {
      if (!outDrag) return;
      var changed = groupIds(list).join('|') !== outDrag.before;
      outDrag = null;
      row.classList.remove('lift');
      row.style.transition = 'transform .16s var(--ease)'; row.style.transform = '';
      if (changed) setTimeout(function () { outCommit(list.dataset.g, groupIds(list)); }, 170);
    }
    h.addEventListener('pointerup', end);
    h.addEventListener('lostpointercapture', end);
  }
  $('outDev').onclick = function (e) { e.stopPropagation(); palShow('out'); };
  // the priority of the output devices: the same window in its arranging view
  $('outOrder').onclick = function (e) { e.stopPropagation(); palShow('prio'); };
  function defName() { // the microphone behind "As in Windows": never a virtual cable (see autoMic in Go)
    var l = (DEVS && DEVS.capture) || [], d = l.filter(function (x) { return x.auto; })[0] || l.filter(function (x) { return x.default; })[0];
    return d ? d.name : '';
  }
  $('micDev').onclick = function (e) { e.stopPropagation(); palShow('in'); };

  // ---------- the device palette: outputs and microphones, search, keys ----------
  var palItems = [], palSel = 0;
  function palOpen() { return !$('pal').classList.contains('off'); }
  function palClose() { $('pal').classList.add('off'); }
  function palMode() { return $('pal').dataset.mode || 'out'; }
  // mode: 'out' — output devices, 'in' — microphones, 'prio' — the priority of the outputs
  // the priority list opened from the device list goes back there; opened from elsewhere (Settings,
  // Tab before the list was shown) "back" closes it — "Done" always closes it
  var palFromList = false;
  function palBackStep() { if (palMode() === 'prio' && palFromList) palShow('out'); else palClose(); }
  function palShow(mode) {
    if (mode === 'prio') palFromList = palOpen() && palMode() !== 'prio';
    closePops();
    $('palQ').value = '';
    $('pal').dataset.mode = mode;
    $('pal').classList.remove('off');
    var prio = mode === 'prio';
    $('pal').querySelector('.pal-s').classList.toggle('hide', prio);
    $('palKeys').classList.toggle('hide', prio);
    $('palL').classList.toggle('prio', prio);
    $('palOrder').classList.toggle('hide', mode === 'in');
    $('palOrder').textContent = tx(prio ? 'Готово' : 'Пріоритет');
    $('palOrder').classList.toggle('acc', prio);
    $('palL').scrollTop = 0;
    if (prio) { outPopRender(true); return; }
    palRender(true);
    $('palQ').focus();
    loadDevs(function () { if (palOpen() && palMode() === mode) palRender(); });
  }
  function devKind(d, input) {
    if (input) return 'mic';
    return /bluetooth|airpods|buds|wh-|wf-/i.test(d.name) ? 'bt' : (/headphone|headset|навуш|гарніт|arctis|nova/i.test(d.name) ? 'headphones' : 'speaker');
  }
  function palRender(reset) {
    var q = $('palQ').value.trim().toLowerCase(), mode = palMode();
    var mf = (S && S.cfg.micFx) || {}, curMic = S ? (S.cfg.micDev || '') : '', caps = (DEVS && DEVS.capture) || [];
    var useMic = curMic || (caps.filter(function (x) { return x.auto; })[0] || {}).id;
    // a headset its receiver calls off stays pickable: the receiver may be wrong, the choice is kept
    var outs = OUTS.filter(function (d) { return d.state === 'on' || d.state === 'hsoff'; }).map(function (d) {
      return { k: 'out', id: d.id, name: d.name, sub: d.default ? tx('Зараз грає') : (d.state === 'hsoff' ? tx('Гарнітуру вимкнено') : (d.virtual ? tx('віртуальний') : '')), cur: !!d.default, ic: devKind(d, false) };
    });
    var ins = caps.filter(function (d) { return !d.virtual || d.id === curMic; }).map(function (d) {
      var cur = d.id === useMic;
      return { k: 'in', id: d.id, name: d.name, sub: cur ? (mf.on && mf.out ? tx('з обробкою') : tx('Обрано')) : (d.virtual ? tx('віртуальний') : ''), cur: cur, ic: 'mic' };
    });
    if (S && !S.cfg.winCtl) ins.unshift({ k: 'in', id: '', name: tx('Як у Windows'), sub: defName(), cur: curMic === '', ic: 'mic' });
    var groups = mode === 'in' ? [['in', tx('Мікрофон'), ins]] : [['out', tx('Вивід'), outs]];
    var h = '';
    palItems = [];
    groups.forEach(function (g) {
      var l = g[2].filter(function (it) { return !q || it.name.toLowerCase().indexOf(q) >= 0; });
      if (!l.length) return;
      h += '<div class="pal-g">' + esc(g[1]) + '</div>';
      l.forEach(function (it) {
        var i = palItems.length;
        palItems.push(it);
        h += '<button class="pal-i" role="option" data-i="' + i + '"><span class="dtile">' + icon(it.ic === 'bt' ? 'bt' : it.ic, 16) + '</span>' +
          '<span class="t"><span class="n">' + esc(it.name) + '</span>' + (it.sub ? '<span class="s">' + esc(it.sub) + '</span>' : '') + '</span>' +
          (it.cur ? '<svg class="ck" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>' : '') + '</button>';
      });
    });
    if (!palItems.length) h = '<div class="pal-e">' + esc(DEVS || OUTS.length ? tx('Нічого не знайдено') : tx('Шукаю пристрої…')) + '</div>';
    // processing off: apps hear the chosen microphone only through it
    if (mode === 'in' && !q && !(mf.on && mf.out) && (feat('denoise') || feat('vst'))) {
      h += '<button class="pal-a" id="palFx">' + icon('sparkles', 15) + '<span>' + esc(tx(virt().length ? 'Увімкнути обробку мікрофона' : 'Як налаштувати')) + '</span></button>';
    }
    $('palL').innerHTML = h;
    if (reset) { palSel = Math.max(0, palItems.findIndex(function (it) { return it.cur; })); }
    palSel = Math.min(palSel, Math.max(0, palItems.length - 1));
    palMark();
    $('palL').querySelectorAll('.pal-i').forEach(function (b) {
      b.onclick = function () { palPick(+b.dataset.i); };
      b.onpointermove = function () { if (palSel !== +b.dataset.i) { palSel = +b.dataset.i; palMark(true); } };
    });
    var fx = $('palFx');
    if (fx) fx.onclick = function () { palClose(); if (virt().length) toggleMfx(); else show('micTab', 'mfxCard'); };
  }
  function palMark(noScroll) {
    $('palL').querySelectorAll('.pal-i').forEach(function (b) {
      var on = +b.dataset.i === palSel;
      b.classList.toggle('sel', on); b.setAttribute('aria-selected', on);
      if (on && !noScroll) b.scrollIntoView({ block: 'nearest' });
    });
  }
  function palPick(i) {
    var it = palItems[i];
    if (!it) return;
    if (it.k === 'out') api('/api/output', { dev: it.id }); else api('/api/mic', { dev: it.id });
    palClose();
  }
  $('palQ').oninput = function () { palSel = 0; palRender(); };
  $('palQ').onkeydown = function (e) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (palItems.length) { palSel = (palSel + (e.key === 'ArrowDown' ? 1 : -1) + palItems.length) % palItems.length; palMark(); }
    } else if (e.key === 'Enter') { e.preventDefault(); palPick(palSel); }
    else if (e.key === 'Tab' && palMode() === 'out') { e.preventDefault(); palShow('prio'); }
  };
  $('pal').onclick = function (e) { if (e.target === $('pal')) palClose(); };
  $('palOrder').onclick = function (e) { e.stopPropagation(); if (palMode() === 'prio') palClose(); else palShow('prio'); };
  $('palWin').onclick = function () { api('/api/winsound', {}); palClose(); };
  $('palBack').onclick = function () { palBackStep(); };

  // рівень мікрофона: червоний, коли б'ється об межу
  var pkHold = -120, pkAt = 0, clipAt = 0;
  function lvlPct(db) { return Math.max(0, Math.min(100, (db + 60) / 60 * 100)); }
  function micMeterDraw(m) {
    var raw = m.ok ? m.db : -120, db = smooth('mic', raw), now = Date.now();
    if (raw > pkHold || now - pkAt > 1200) { pkHold = raw; pkAt = now; }
    if (raw >= -1) clipAt = now;
    $('micLvl').style.width = lvlPct(db) + '%'; $('micLvl').style.background = vuZone(db)[2];
    $('micPk').style.left = 'calc(' + lvlPct(pkHold) + '% - 2px)';
    $('micLvlBox').classList.toggle('clip', now - clipAt < 1000);
  }

  // ---------- швидкі перемикачі ----------
  document.querySelectorAll('[data-goto]').forEach(function (b) {
    b.onclick = function (e) { e.stopPropagation(); var id = b.dataset.goto; if (id === 'rtCard') { show('sound'); setPage('sound', 'cables'); } else show(id === 'mfxCard' ? 'micTab' : 'sound', id); };
  });
  // вимикаємо лише шумодав: обробка (і кабель, який обрано мікрофоном у Discord) лишається
  function quickNs() {
    var mf = (S.cfg.micFx || {});
    if (mf.on && mf.denoise) { api('/api/micfx', { denoise: false }); return; }
    if (!mf.on && !mf.out && !virt().length) { show('micTab', 'mfxCard'); return; }
    if (!mf.on) toggleMfx({ denoise: true }); else api('/api/micfx', { denoise: true });
  }
  function quickDuck() {
    var l = S.cfg.duckLevel || 100, prev = 60;
    try { prev = +localStorage.getItem('duckPrev') || 60; } catch (x) { }
    if (l < 100) { try { localStorage.setItem('duckPrev', l); } catch (x) { } patch({ duckLevel: 100 }); } else patch({ duckLevel: prev });
  }
  function quickRt() { $('swRt').onclick(); }
  [['qNs', quickNs], ['qDuck', quickDuck], ['qRt', quickRt]].forEach(function (d) {
    var el = $(d[0]);
    el.onclick = function () { if (S) d[1](); };
    el.onkeydown = function (e) { if ((e.key === 'Enter' || e.key === ' ') && e.target === el) { e.preventDefault(); if (S) d[1](); } };
  });

  // ---------- групи програм ----------
  window.retryIcon = function (img) {
    var n = +(img.dataset.n || 0);
    if (n >= 4) { img.remove(); return; }
    img.dataset.n = n + 1;
    setTimeout(function () { img.src = img.src.split('&r=')[0] + '&r=' + (n + 1); }, 1200 * (n + 1));
  };
  function iconHTML(exe, name) {
    return '<span class="aico">' + esc((name || exe || '?').charAt(0).toUpperCase()) + '<img alt="" draggable="false" style="visibility:hidden" src="/icon?k=' + TOK + '&exe=' + encodeURIComponent(exe) + '" onload="this.style.visibility=\'visible\';this.parentNode.classList.add(\'img\')" onerror="retryIcon(this)"></span>';
  }
  var GROUPS = [
    // Lucide icons, as on the balance card and the tiles
    { k: 'game', n: 'Гра', i: '<line x1="6" x2="10" y1="11" y2="11"/><line x1="8" x2="8" y1="9" y2="13"/><line x1="15" x2="15.01" y1="12" y2="12"/><line x1="18" x2="18.01" y1="10" y2="10"/><path d="M17.32 5H6.68a4 4 0 0 0-3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414.586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3-3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z"/>' },
    { k: 'chat', n: 'Чат', i: '<path d="M3 11h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-5Zm0 0a9 9 0 1 1 18 0m0 0v5a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3Z"/><path d="M21 16v2a4 4 0 0 1-4 4h-5"/>' },
    { k: 'media', n: 'Медіа', i: '<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>' }
  ];
  var appSig = '', tabSig = '', appSl = {}, APPS = [], appCat = 'game';
  try { appCat = localStorage.getItem('appCat') || 'game'; } catch (x) { }
  function setAppCat(k) { appCat = k; try { localStorage.setItem('appCat', k); } catch (x) { } appSig = ''; tabSig = ''; renderApps(APPS); }
  function renderApps(list) {
    APPS = list;
    if (dd) return; // rows and tabs stay as they are until the drop; the next state redraws them
    if (!GROUPS.some(function (g) { return g.k === appCat; })) appCat = 'game';
    // the tabs: the number of apps, a dot while one of them plays
    var ts = JSON.stringify([appCat, I18.lang, GROUPS.map(function (g) {
      var items = list.filter(function (a) { return a.cat === g.k; });
      return [items.length, items.some(function (a) { return a.live && a.sound; })];
    })]);
    if (ts !== tabSig) { tabSig = ts; $('appTabs').innerHTML = GROUPS.map(function (g) {
      var items = list.filter(function (a) { return a.cat === g.k; }), playing = items.some(function (a) { return a.live && a.sound; });
      return '<button role="tab" data-cat="' + g.k + '" aria-selected="' + (g.k === appCat) + '"><svg class="ti" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"' + (g.k !== 'media' ? ' style="color:var(--' + (g.k === 'game' ? 'g' : 'c') + ')"' : '') + '>' + g.i + '</svg>' +
        '<span>' + tx(g.n) + '</span>' + (playing ? '<em class="pl" title="' + tx('грає') + '">' + items.length + ' ▸</em>' : '<em>' + items.length + '</em>') + '</button>';
    }).join('');
    $('appTabs').querySelectorAll('[data-cat]').forEach(function (b) { b.onclick = function () { if (b.dataset.cat !== appCat) setAppCat(b.dataset.cat); }; }); }
    $('appHint').textContent = appCat === 'media' ? tx('баланс не діє') + ' · ' + tx('Перетягніть програму в іншу групу.') : tx('Перетягніть програму в іншу групу.');
    var sig = JSON.stringify([appCat, list.map(function (a) { return [a.exe, a.cat, a.live, a.playing, a.running, a.removable, a.vol >= 0]; })]);
    if (sig !== appSig) {
      appSig = sig; appSl = {};
      var box = $('groups'), items = list.filter(function (a) { return a.cat === appCat; });
      box.dataset.cat = appCat;
      box.innerHTML = items.length ? items.map(function (a) {
        return '<div class="ar" data-exe="' + esc(a.exe) + '">' + iconHTML(a.exe, a.name) +
          '<div class="an"><div class="nm">' + esc(a.name) + '</div><div class="sub" data-st></div></div>' +
          (a.vol >= 0 ? '<div class="avs" style="flex:1;min-width:0;display:flex;align-items:center;gap:6px"><div data-vol></div><span class="av"></span></div>' : '<div style="flex:1"></div>') +
          '<button class="ib mb" data-rm title="' + tx('Прибрати зі списку') + '" aria-label="' + tx('Прибрати зі списку') + '">' + icon('trash', 16) + '</button></div>';
      }).join('') : '<div class="ge">' + tx('Перетягніть сюди програму або натисніть +') + '</div>';
      box.querySelectorAll('.ar').forEach(function (row) {
        var a = list.filter(function (x) { return x.exe === row.dataset.exe; })[0];
        var v = row.querySelector('[data-vol]');
        if (v) {
          var lab = row.querySelector('.av');
          var send = throttle(function (x) { api('/api/app', { exe: a.exe, vol: x }); }, 80);
          appSl[a.exe] = { s: Slider(v, { min: 0, max: 100, step: 1, wheel: 2, key: 5, cls: 'sm', value: a.vol, onInput: function (x) { lab.textContent = x + '%'; send(x); }, onChange: function (x) { send(x, true); } }), lab: lab };
        }
        // removed at once; "+" brings it back
        row.querySelector('[data-rm]').onclick = function (e) { e.stopPropagation(); api('/api/app/remove', { exe: a.exe }); };
        row.addEventListener('pointerdown', function (e) { rowDown(e, a, row); });
      });
    }
    list.forEach(function (a) {
      var x = appSl[a.exe];
      if (x && a.vol >= 0) { x.s.set(a.vol); if (!x.s.busy()) x.lab.textContent = a.vol + '%'; }
    });
    // стан оновлюємо на місці, не перебудовуючи рядки
    $('groups').querySelectorAll('.ar').forEach(function (row) {
      var a = list.filter(function (x) { return x.exe === row.dataset.exe; })[0], el = row.querySelector('[data-st]');
      if (!a || !el) return;
      var h = a.live ? (a.sound ? '<span class="pdot"></span>' + tx('грає') : tx('тихо')) : (a.running ? tx('тихо') : tx('не запущено'));
      if (el.innerHTML !== h) el.innerHTML = h;
    });
  }
  $('appAdd').onclick = function (e) { e.stopPropagation(); addApp(appCat, $('appAdd')); };
  // перетягування між групами: рядок кидають на вкладку групи; біля країв список прокручується сам
  var dd = null;
  function ddTarget() {
    var el = document.elementFromPoint(dd.cx, dd.cy), g = el && el.closest('#appTabs [data-cat]');
    $('appTabs').querySelectorAll('[data-cat]').forEach(function (x) { x.classList.toggle('drop', x === g && g.dataset.cat !== dd.a.cat); });
    dd.target = g ? g.dataset.cat : null;
  }
  // a far group in a long list: the view scrolls while the pointer rests near its top or bottom edge
  function ddScroll() {
    if (!dd || !dd.on) return;
    var v = $('mix'), vr = v.getBoundingClientRect(), edge = 56, d = 0;
    if (dd.cy < vr.top + edge) d = -Math.ceil((vr.top + edge - dd.cy) / 4);
    else if (dd.cy > vr.bottom - edge) d = Math.ceil((dd.cy - vr.bottom + edge) / 4);
    if (d) { var t = v.scrollTop; v.scrollTop += d; if (v.scrollTop !== t) ddTarget(); }
    dd.raf = requestAnimationFrame(ddScroll);
  }
  function rowDown(e, a, row) {
    if (e.button !== 0 || e.target.closest('.sl,button')) return;
    dd = { a: a, row: row, x: e.clientX, y: e.clientY, on: false, id: e.pointerId };
    row.setPointerCapture(e.pointerId);
    row.onpointermove = function (ev) {
      if (!dd) return;
      if (!dd.on) {
        if (Math.hypot(ev.clientX - dd.x, ev.clientY - dd.y) < 6) return;
        dd.on = true;
        dd.ghost = document.createElement('div'); dd.ghost.className = 'drag';
        dd.ghost.innerHTML = row.querySelector('.aico').outerHTML + '<span>' + esc(a.name) + '</span>';
        document.body.appendChild(dd.ghost); row.classList.add('ghost');
        $('appTabs').classList.add('dz');
        dd.raf = requestAnimationFrame(ddScroll);
      }
      dd.cx = ev.clientX; dd.cy = ev.clientY;
      dd.ghost.style.left = (ev.clientX + 12) + 'px'; dd.ghost.style.top = (ev.clientY + 10) + 'px';
      ddTarget();
    };
    row.onpointerup = row.onpointercancel = row.onlostpointercapture = function () {
      row.onpointermove = row.onpointerup = row.onpointercancel = row.onlostpointercapture = null;
      var d = dd;
      dd = null;
      if (d && d.on) {
        cancelAnimationFrame(d.raf);
        d.ghost.remove(); row.classList.remove('ghost');
        $('appTabs').classList.remove('dz');
        $('appTabs').querySelectorAll('[data-cat]').forEach(function (x) { x.classList.remove('drop'); });
        if (d.target && d.target !== d.a.cat) api('/api/app', { exe: d.a.exe, cat: d.target });
      }
      appSig = ''; tabSig = ''; renderApps(APPS);
    };
  }
  var pop = $('pop');
  function anyPop() { return document.querySelector('.pop:not(.off)') !== null; }
  function closePops() { document.querySelectorAll('.pop').forEach(function (p) { p.classList.add('off'); }); }
  function placePop(anchor) {
    var v = $('mix'), r = anchor.getBoundingClientRect(), vr = v.getBoundingClientRect(), h = pop.offsetHeight;
    pop.style.top = (r.bottom - vr.top + v.scrollTop + 4) + 'px';
    // no room below in the visible part of the view: open upwards
    if (r.bottom + 4 + h > vr.bottom && r.top - 4 - h > vr.top) pop.style.top = (r.top - vr.top + v.scrollTop - 4 - h) + 'px';
  }
  function addApp(cat, anchor) {
    if (!pop.classList.contains('off') && pop.dataset.for === '+' + cat) { closePops(); return; }
    closePops(); pop.dataset.for = '+' + cat;
    pop.innerHTML = '<div class="hd">' + tx('Шукаю відкриті програми…') + '</div>';
    placePop(anchor); pop.classList.remove('off');
    api('/api/procs').then(function (r) { return r.json(); }).then(function (l) {
      l = (l || []).filter(function (p) { return !p.listed; });
      pop.innerHTML = '<div class="hd">' + tx('Відкриті програми — оберіть, щоб додати до групи «%s»').replace('%s', tx(GROUPS.filter(function (g) { return g.k === cat; })[0].n)) + '</div>' +
        (l.length ? l.map(function (p, i) { return '<button class="it" data-i="' + i + '">' + iconHTML(p.exe, p.name) + '<span><div style="font-weight:500">' + esc(p.name) + '</div><div class="sub">' + esc(p.exe) + '</div></span></button>'; }).join('') :
          '<div class="hd">' + tx('Нових програм з вікнами не знайдено.') + '</div>');
      pop.querySelectorAll('[data-i]').forEach(function (b) { b.onclick = function () { api('/api/app', { exe: l[+b.dataset.i].exe, cat: cat }); closePops(); }; });
    });
  }
  document.addEventListener('click', function (e) { if (!e.target.closest('.pop')) closePops(); });

  // ---------- Equalizer APO: the equalizer of everything for any headphones, AutoEQ ----------
  var apoSig = '';
  function renderApo(s, hw) {
    var a = s.apo || {}, o = s.out || {}, dn = o.short || o.name || '', box = $('apoBox');
    var md = s.apoMod || {};
    var sig = JSON.stringify([a, md, o.id, dn, hw, I18.lang, !!S.cfg.apoHs, eqT, S.cfg.autoEq && S.cfg.autoEq[o.id]]);
    if (sig === apoSig) return;
    apoSig = sig;
    var note = function (t, btn) { return '<div class="note" style="margin:0 0 10px">' + esc(t) + (btn ? '<br>' + btn : '') + '</div>'; };
    var h = '';
    var mrow = function (title, sub, btn) { return '<div class="row aeq"><span class="ri">' + icon('layers', 15) + '</span><div class="grow"><div>' + title + '</div><div class="sub">' + sub + '</div></div>' + btn + '</div>'; };
    if (md.busy) {
      // installing or removing: the audio services restart and the device is gone for a few seconds
      h = '<div style="margin-bottom:8px">' + mrow(tx('Модуль Arcmix'), tx(md.busy === 'install' ? 'встановлюю на «%s» — звук на кілька секунд зникне' : 'прибираю з «%s» — звук на кілька секунд зникне').replace('%s', esc(md.busyName || dn)), '<button class="chip sm" disabled>…</button>') + '</div>';
    } else if (!a.dev && (md.inst || md.free)) {
      // the Arcmix module: on any device without Equalizer APO
      h = md.inst
        ? mrow(tx('Модуль Arcmix'), (md.live ? tx('працює на «%s»: EQ Windows без кабелів') : tx('встановлено — запрацює, щойно зазвучить звук')).replace('%s', esc(dn)), '<button class="chip sm" data-apo="modRemove">' + tx('Прибрати') + '</button>')
        : mrow(tx('Модуль Arcmix (тест)'), tx('власний еквалайзер для «%s» без кабелів і без Equalizer APO. Потрібен дозвіл адміністратора; звук на мить зникне.').replace('%s', esc(dn)), '<button class="chip sm acc" data-apo="modInstall">' + tx('Встановити') + '</button>');
      // a headset with its own equalizer: the curve of everything is added to it only if asked (apoHs)
      if (md.inst && md.hs) {
        var hsOn2 = !!S.cfg.apoHs;
        h += mrow(tx('EQ Windows на гарнітурі'), tx(hsOn2 ? 'діють обидва еквалайзери — їхні криві додаються' : 'вимкнено: працює лише еквалайзер гарнітури'),
          '<button class="sw" role="switch" id="apoHsSw" aria-checked="' + hsOn2 + '" aria-label="' + tx('EQ Windows на гарнітурі') + '"><i></i></button>');
      }
      h = '<div style="margin-bottom:8px">' + h + '</div>';
      // with the device's enhancements off Windows runs no effect there, the module included
      if (md.sysfx) h += note(tx('Для «%s» у Windows вимкнено «Покращення звуку», тому модуль Arcmix не працює. Увімкніть їх у властивостях пристрою.').replace('%s', dn), '<button class="chip" data-win style="margin-top:8px">' + tx('Налаштування звуку Windows') + '</button>');
    } else if (!a.installed) {
      // shown with a headset's own equalizer too: AutoEQ and other connections need Equalizer APO
      h = note(tx('Еквалайзер для будь-яких навушників — без кабелів і без затримки — через безкоштовний Equalizer APO. Встановіть його, а все інше налаштуєте тут.'),
        '<button class="chip acc" data-url="' + 'https://sourceforge.net/projects/equalizerapo/' + '" style="margin-top:8px">' + tx('Завантажити Equalizer APO') + '</button>');
    } else if (!a.ready) {
      h = note(tx('Equalizer APO встановлено. Дозвольте Arcmix керувати ним — один раз, Windows попросить дозволу адміністратора.'), '<button class="chip acc" data-apo="setup" style="margin-top:8px">' + tx('Налаштувати') + '</button>');
    } else if (!a.dev) {
      h = note(tx('Equalizer APO ще не працює на «%s». У вікні, що відкриється, позначте цей пристрій, натисніть OK і перезавантажте ПК.').replace('%s', dn), '<button class="chip acc" data-apo="devices" style="margin-top:8px">' + tx('Обрати пристрої') + '</button>');
    } else if (a.sysfxOff) {
      h = note(tx('Для «%s» у Windows вимкнено «Покращення звуку», тому Equalizer APO не працює. Увімкніть їх у властивостях пристрою.').replace('%s', dn), '<button class="chip" data-win style="margin-top:8px">' + tx('Налаштування звуку Windows') + '</button>');
    } else if (a.samples) {
      // the samples of Equalizer APO change the sound next to our curve: one step clears them
      h = note(tx('В Equalizer APO досі діють зразкові налаштування: −6 дБ і підсилення басу. Вони змінюють звук разом з кривою Arcmix.'), '<button class="chip acc" data-apo="setup" style="margin-top:8px">' + tx('Прибрати зразки') + '</button>');
    } else if (eqT === 'all') {
      // working: all of it on the Everything tab — on a headset with its own equalizer the switch first,
      // then the correction of these headphones (AutoEQ)
      var row = function (ic, title, sub, end) { return '<div class="row aeq"><span class="ri">' + icon(ic, 15) + '</span><div class="grow"><div>' + title + '</div><div class="sub">' + sub + '</div></div>' + end + '</div>'; };
      var hsOn = !!S.cfg.apoHs;
      if (a.hs) h += row('layers', tx('Equalizer APO на гарнітурі'), tx(hsOn ? 'діють обидва еквалайзери — їхні криві додаються' : 'вимкнено: працює лише еквалайзер гарнітури'),
        '<button class="sw" role="switch" id="apoHsSw" aria-checked="' + hsOn + '" aria-label="' + tx('Equalizer APO на гарнітурі') + '"><i></i></button>');
      if (!a.hs || hsOn) {
        var ae = a.autoEq, aon = ae && !!(S.cfg.autoEq && S.cfg.autoEq[o.id] && S.cfg.autoEq[o.id].on);
        h += row('headphones', tx('Корекція AutoEQ'), ae ? esc(ae[0]) : tx('рівне звучання для вашої моделі навушників'),
          (ae ? '<button class="sw" role="switch" id="aeqSw" aria-checked="' + aon + '" aria-label="' + tx('Корекція AutoEQ') + '"><i></i></button>' : '') +
          '<button class="chip sm" id="aeqPick">' + tx(ae ? 'Змінити' : 'Обрати') + '</button>');
      }
      h = '<div style="margin-bottom:8px">' + h + '</div>';
    }
    box.innerHTML = h;
    box.classList.toggle('hide', !h);
    box.querySelectorAll('[data-url]').forEach(function (b) { b.onclick = function () { api('/api/open', { url: b.dataset.url }); }; });
    box.querySelectorAll('[data-apo]').forEach(function (b) { b.onclick = function () { api('/api/apo', { op: b.dataset.apo }); }; });
    var w = box.querySelector('[data-win]'); if (w) w.onclick = function () { api('/api/winsound', {}); };
    var sw = $('aeqSw'); if (sw) sw.onclick = function () { var on = sw.getAttribute('aria-checked') !== 'true'; sw.setAttribute('aria-checked', on); api('/api/autoeq', { dev: o.id, on: on }); };
    var pk = $('aeqPick'); if (pk) pk.onclick = function (e) { e.stopPropagation(); aeqOpen(dn); };
    var hs = $('apoHsSw'); if (hs) hs.onclick = function () { patch({ apoHs: !S.cfg.apoHs }); };
  }
  // the AutoEQ picker: a search over the measured models, the device's own name to start with
  var aeqT = 0, aeqList = [];
  function aeqOpen(q) {
    var p = $('aeqPop');
    if (!p.classList.contains('off')) { closePops(); return; }
    closePops();
    p.innerHTML = '<div class="pal-s" style="height:40px;padding:0 8px;margin:-6px -6px 4px">' + icon('search', 15) + '<input class="pal-in" id="aeqQ" autocomplete="off" spellcheck="false" placeholder="' + esc(tx('Модель навушників…')) + '" aria-label="' + esc(tx('Модель навушників…')) + '"></div><div id="aeqL"></div>' +
      '<div class="sub" style="padding:6px 8px 2px">' + tx('Виміри й криві — проєкт AutoEQ (Jaakko Pasanen).') + '</div>';
    p.style.top = ($('apoBox').offsetTop + $('apoBox').offsetHeight) + 'px';
    p.classList.remove('off');
    var inp = $('aeqQ');
    inp.value = (q || '').replace(/\(.*\)/g, '').trim();
    inp.oninput = function () { clearTimeout(aeqT); aeqT = setTimeout(aeqFind, 250); };
    inp.onclick = function (e) { e.stopPropagation(); };
    inp.focus();
    aeqFind();
  }
  function aeqFind() {
    var q = ($('aeqQ') || {}).value || '';
    $('aeqL').innerHTML = '<div class="hd">' + tx('Шукаю…') + '</div>';
    api('/api/autoeq', { q: q }).then(function (r) { return r.json(); }).then(function (d) {
      if (!$('aeqL')) return;
      aeqList = d.list || [];
      $('aeqL').innerHTML = d.err ? '<div class="hd">' + tx('Не вдалося завантажити список. Перевірте інтернет.') + '</div>' :
        (aeqList.length ? aeqList.map(function (m, i) { return '<button class="it" data-i="' + i + '"><span class="nm">' + esc(m.n) + '</span><span class="sub">' + esc(m.s) + '</span></button>'; }).join('') : '<div class="hd">' + tx('Нічого не знайдено') + '</div>');
      $('aeqL').querySelectorAll('[data-i]').forEach(function (b) {
        b.onclick = function (e) {
          e.stopPropagation();
          var m = aeqList[+b.dataset.i], o = (S && S.out) || {};
          b.querySelector('.sub').textContent = tx('Завантажую…');
          api('/api/autoeq', { dev: o.id, path: m.p, name: m.n, src: m.s }).then(function (r) { return r.json(); }).then(function (x) {
            if (x.err) { b.querySelector('.sub').textContent = tx('Не вдалося завантажити'); return; }
            closePops();
          });
        };
      });
    }).catch(function () { });
  }

  // ---------- еквалайзер (вбудований у гарнітуру, 10 смуг) ----------
  var STD = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
  var EQI = { bands: 10, freqs: STD, min: -12, max: 12 }, FREQS = [];
  function fLabel(f) { return f >= 1000 ? (Math.round(f / 100) / 10 + 'k').replace('.0k', 'k') : String(Math.round(f)); }
  function setEQI(e) {
    e = e || { bands: 10, freqs: STD, min: -12, max: 12 };
    var changed = e.bands !== EQI.bands || String(e.freqs) !== String(EQI.freqs) || e.min !== EQI.min || e.max !== EQI.max;
    EQI = e; FREQS = e.freqs.map(fLabel);
    $('eqVals').style.gridTemplateColumns = 'repeat(' + e.bands + ',1fr)';
    return changed;
  }
  function zeros() { return EQI.freqs.map(function () { return 0; }); }
  function clampEQ(v) { return Math.max(EQI.min, Math.min(EQI.max, Math.round(v * 2) / 2)); }
  // пресет на 10 смуг → смуги конкретної гарнітури (інтерполяція за логарифмом частоти)
  function presetFor(vals) {
    return EQI.freqs.map(function (f) {
      var x = Math.log2(f), xs = STD.map(Math.log2);
      if (x <= xs[0]) return clampEQ(vals[0]);
      for (var i = 0; i < 9; i++) if (x <= xs[i + 1]) return clampEQ(vals[i] + (vals[i + 1] - vals[i]) * (x - xs[i]) / (xs[i + 1] - xs[i]));
      return clampEQ(vals[9]);
    });
  }
  setEQI();
  var PRESETS = [
    ['flat', 'Рівний', [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
    ['bass', 'Більше басу', [3.5, 4, 1, -1.5, -1.5, -1, -1, -1, -1, 5.5]],
    // music by genre, gentle (within ±4 dB): a colour, not a different headphone
    ['jazz', 'Джаз', [1.5, 1.5, 1, 0.5, 0, -0.5, 0, 1, 1.5, 2]],
    ['metal', 'Метал', [2.5, 2, 0.5, -1, -1.5, 0, 1.5, 2.5, 2, 1]],
    ['edm', 'EDM', [4, 3.5, 1.5, 0, -1, -1, 0, 1, 2, 2.5]],
    ['focus', 'Фокус', [-5, -1, -3.5, -2.5, 4, 6, 3.5, -3.5, 0, -3.5]],
    ['steps', 'Кроки в іграх', [-4, -3, -2, -1, 0, 1, 3, 4.5, 3, 1]],
    ['voice', 'Чіткі голоси', [-3, -2.5, -1.5, 0, 1, 2.5, 3, 2, 0, -1]],
    ['movie', 'Кіно', [3, 2.5, 1, 0, 0, 1, 1.5, 1, 1.5, 2]]
  ];
  // eqP: a built-in key, 'u:<name>' (the user's, may be edited in place) or 'custom' (a changed curve not saved yet);
  // eqBase: what 'custom' came from, for "Cancel"
  var eqB = zeros(), eqP = 'flat', eqBase = '', eqDrag = -1, eqSent = 0, eqLocal = 0;
  var svg = $('eqSvg'), EW = 352, EH = 200, PX = 22, PY = 14;
  function ex(i) { return PX + i * (EW - 2 * PX) / Math.max(1, EQI.bands - 1); }
  function ey(v) { return PY + (12 - v) / 24 * (EH - 2 * PY); }
  function eqDraw() {
    var pts = eqB.map(function (v, i) { return [ex(i), ey(v)]; });
    var d = 'M' + pts[0][0] + ',' + pts[0][1];
    for (var i = 0; i < pts.length - 1; i++) {
      var p0 = pts[Math.max(0, i - 1)], p1 = pts[i], p2 = pts[i + 1], p3 = pts[Math.min(pts.length - 1, i + 2)];
      d += ' C' + (p1[0] + (p2[0] - p0[0]) / 6) + ',' + (p1[1] + (p2[1] - p0[1]) / 6) + ' ' + (p2[0] - (p3[0] - p1[0]) / 6) + ',' + (p2[1] - (p3[1] - p1[1]) / 6) + ' ' + p2[0] + ',' + p2[1];
    }
    var grid = '';
    [-12, -6, 0, 6, 12].forEach(function (v) { grid += '<line x1="' + PX + '" x2="' + (EW - PX) + '" y1="' + ey(v) + '" y2="' + ey(v) + '" style="stroke:var(--tr)" stroke-width="' + (v ? 1 : 1.5) + '"/><text x="2" y="' + (ey(v) + 4) + '" style="fill:var(--ov)" font-size="10">' + (v > 0 ? '+' + v : v) + '</text>'; });
    var area = d + ' L' + ex(EQI.bands - 1) + ',' + ey(0) + ' L' + ex(0) + ',' + ey(0) + ' Z';
    svg.innerHTML = '<defs><linearGradient id="eqg" x1="0" y1="0" x2="1" y2="0"><stop offset="0" style="stop-color:var(--g)"/><stop offset="1" style="stop-color:var(--c)"/></linearGradient></defs>' + grid +
      '<path d="' + area + '" fill="url(#eqg)" opacity=".16"/>' +
      '<path d="' + d + '" fill="none" stroke="url(#eqg)" stroke-width="3" stroke-linecap="round"/>' +
      pts.map(function (p, i) { return '<circle cx="' + p[0] + '" cy="' + p[1] + '" r="' + (eqDrag === i ? 8 : 6.5) + '" style="fill:var(--solid);stroke:' + (i < EQI.bands / 2 ? 'var(--g)' : 'var(--c)') + '" stroke-width="3"/>'; }).join('');
    $('eqVals').innerHTML = eqB.map(function (v, i) { return '<span><b>' + (v > 0 ? '+' : '') + v + '</b>' + FREQS[i] + '</span>'; }).join('');
    if (typeof presetMarks === 'function') presetMarks();
  }
  var eqT = 'hw'; // що налаштовуємо: гарнітуру чи програмний еквалайзер категорії (game / chat / media)
  var eqHsRunning; // the headset was on at the last render (undefined before the first)
  // while dragging at most ~4 curves a second: every curve command makes the headset click faintly
  function eqSend(final) {
    var now = Date.now(); eqLocal = now;
    if (final || now - eqSent > 250) { eqSent = now; api('/api/eq', { bands: eqB, preset: eqP, target: eqT === 'hw' ? '' : eqT }); }
  }
  // крива, пресет і «власний» поточної цілі зі стану програми
  function eqLoad(s) {
    if (eqT === 'hw') {
      if (s.eq && s.eq.length === EQI.bands) { eqB = s.eq.slice(); eqP = s.eqPreset || 'flat'; }
      return;
    }
    var se = ((s.cfg || {}).softEq || {})[eqT] || {};
    eqB = se.bands && se.bands.length === 10 ? se.bands.slice() : zeros();
    eqP = se.preset || 'flat';
  }
  // what the equalizer card shows for the chosen tab: no curve where nothing would apply it (no equalizer
  // at all, or the Everything tab on a headset where Equalizer APO is switched off and there is no routing)
  var EQV = { hwEq: false, canSW: false, rtSW: false, apoAvail: false };
  function eqView() {
    if (!S) return;
    var a = S.apo || {};
    // a headset with its own equalizer under Equalizer APO or under the module (the module reports it
    // even when Equalizer APO is not installed)
    var hs = !!(a.hs || (S.apoMod && S.apoMod.inst && S.apoMod.hs));
    var none = !EQV.hwEq && !EQV.canSW, off = eqT === 'all' && hs && !S.cfg.apoHs && EQV.apoAvail && !EQV.rtSW;
    $('eqBody').classList.toggle('hide', none || off);
    // the Everything tab through Equalizer APO: its own note instead of the one about routing
    var viaApo = eqT === 'all' && EQV.apoAvail && (!hs || S.cfg.apoHs);
    $('eqNoteApo').classList.toggle('hide', !viaApo);
    // through the module or through Equalizer APO: the note names which
    var viaMod = !!(S.apoMod && S.apoMod.inst && !a.dev);
    $('eqNoteApo').textContent = tx(viaMod ? 'Діє на весь звук цього пристрою через модуль Arcmix — без кабелів і без затримки.' : 'Діє на весь звук цього пристрою через Equalizer APO — без кабелів і без затримки.');
    if (viaApo) $('eqNoteSW').classList.add('hide');
    renderApo(S, EQV.hwEq);
  }
  function setTarget(t) {
    eqT = t;
    setEQI(t === 'hw' ? ((S && S.device) || {}).eq : null);
    eqB = zeros(); eqP = 'flat'; eqBase = '';
    if (S) { eqLoad(S); renderHW(S.device || {}); }
    $('eqTarget').querySelectorAll('button').forEach(function (b) { b.setAttribute('aria-pressed', b.dataset.t === t); });
    $('eqNoteHW').classList.toggle('hide', t !== 'hw'); $('eqNoteSW').classList.toggle('hide', t === 'hw');
    eqDraw();
    eqView();
  }
  // значки пресетів (16 px, контурні)
  var PICON = {
    flat: '<path d="M3 12h18"/><path d="M3 7h18" opacity=".35"/><path d="M3 17h18" opacity=".35"/>',
    bass: '<path d="M2 14c2.5 0 2.5-8 5-8s2.5 12 5 12 2.5-6 5-6 2.5 2 5 2"/>',
    smiley: '<path d="M9 18V5l11-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="17" cy="16" r="3"/>',
    jazz: '<path d="M9 18V5l11-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="17" cy="16" r="3"/>',
    metal: '<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>',
    edm: '<path d="M2 12h3l2-6 3 12 3-9 2 5 2-2h4"/>',
    focus: '<circle cx="12" cy="12" r="7"/><circle cx="12" cy="12" r="2.5"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3"/>',
    steps: '<path d="M5 15.5c-.9-2.6-1.5-4.6-.7-7C5 6.3 6.3 5 7.6 5.2 9 5.4 9.5 7.3 9 9.4c-.5 2.2-1.3 3.6-1 6.1"/><path d="M5.2 18.3a1.9 1.9 0 0 0 3.7-.6l-.1-1.6-3.6.5z"/><path d="M19 11.5c.9-2.6 1.5-4.6.7-7C19 2.3 17.7 1 16.4 1.2 15 1.4 14.5 3.3 15 5.4c.5 2.2 1.3 3.6 1 6.1"/><path d="M18.8 14.3a1.9 1.9 0 0 1-3.7-.6l.1-1.6 3.6.5z"/>',
    voice: '<path d="M3 12h2M7 8v8M11 4v16M15 8v8M19 10v4M21 12h1"/>',
    movie: '<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M7.5 4v16M16.5 4v16M3 9h4.5M3 15h4.5M16.5 9H21M16.5 15H21"/>',
    custom: '<path d="M5 21v-6M5 11V3M12 21v-9M12 8V3M19 21v-4M19 13V3M3 15h4M10 8h4M17 17h4"/>',
    user: '<path d="M12 3l2.6 5.6 6.1.7-4.5 4.2 1.2 6L12 16.6 6.6 19.5l1.2-6-4.5-4.2 6.1-.7z"/>',
    game: '<rect x="2" y="7" width="20" height="11" rx="5.5"/><path d="M7 10.5v4M5 12.5h4"/><circle cx="15.5" cy="12" r="1" fill="currentColor"/><circle cx="18" cy="14" r="1" fill="currentColor"/>'
  };
  // вбудовані пресети гарнітури: значок за назвою (ігрові — геймпад)
  var HWICON = { 'Рівний': 'flat', 'Більше басу': 'bass', 'Фокус': 'focus', 'Музика (V)': 'smiley', 'Кроки (FPS)': 'steps', 'Власний': 'custom', 'Стрім': 'voice' };
  function pchip(attr, icon, label) {
    return '<button class="ichip" ' + attr + ' title="' + esc(label) + '"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + PICON[icon] + '</svg><span>' + esc(label) + '</span></button>';
  }
  $('eqReset').onclick = function () { eqB = zeros(); eqP = 'flat'; eqDraw(); eqSend(true); };
  // вбудовані пресети гарнітури (Nova Pro, GameDAC)
  var hwSig = '', hwOn = false;
  function renderHW(dev) {
    var e = dev.eq || {}, list = eqT === 'hw' ? (e.presets || []) : [], box = $('hwPresets');
    $('hwPresetCard').classList.toggle('hide', !list.length);
    var sig = JSON.stringify(list);
    if (sig !== hwSig) {
      hwSig = sig;
      box.innerHTML = list.map(function (o) { return pchip('data-v="' + o.v + '"', HWICON[o.label] || 'game', tx(o.label)); }).join('');
      box.querySelectorAll('button').forEach(function (b) { b.onclick = function () { api('/api/eqpreset', { value: Number(b.dataset.v) }); }; });
    }
    var cur = dev.hwPreset;
    hwOn = list.length > 0 && cur !== null && cur !== undefined && cur !== e.customSlot;
    box.querySelectorAll('button').forEach(function (b) { b.setAttribute('aria-pressed', hwOn && Number(b.dataset.v) === cur); });
    presetMarks();
  }
  // a hand change: the user's preset stays chosen (and shows it is changed); a built-in one turns into an unsaved curve
  function toCustom() { if (eqP.indexOf('u:') === 0) return; if (eqP !== 'custom') eqBase = eqP; eqP = 'custom'; }
  function svgPt(e) { var r = svg.getBoundingClientRect(); return [(e.clientX - r.left) / r.width * EW, (e.clientY - r.top) / r.height * EH]; }
  function nearest(x) { var best = 0; for (var i = 1; i < EQI.bands; i++) if (Math.abs(ex(i) - x) < Math.abs(ex(best) - x)) best = i; return best; }
  function eqSetAt(i, y) {
    var v = 12 - (y - PY) / (EH - 2 * PY) * 24;
    eqB[i] = clampEQ(v); toCustom(); eqDraw();
  }
  svg.setAttribute('tabindex', '-1');
  svg.addEventListener('pointerdown', function (e) { svg.focus({ preventScroll: true }); var p = svgPt(e); eqDrag = nearest(p[0]); svg.setPointerCapture(e.pointerId); eqSetAt(eqDrag, p[1]); eqSend(); });
  svg.addEventListener('pointermove', function (e) { if (eqDrag >= 0) { eqSetAt(eqDrag, svgPt(e)[1]); eqSend(); } });
  svg.addEventListener('pointerup', function () { if (eqDrag >= 0) { eqDrag = -1; eqDraw(); eqSend(true); } });
  svg.addEventListener('wheel', function (e) { if (document.activeElement !== svg) return; e.preventDefault(); var i = nearest(svgPt(e)[0]); eqB[i] = clampEQ(eqB[i] + (e.deltaY < 0 ? .5 : -.5)); toCustom(); eqDraw(); eqSend(true); }, { passive: false });
  svg.addEventListener('dblclick', function (e) { var i = nearest(svgPt(e)[0]); eqB[i] = 0; toCustom(); eqDraw(); eqSend(true); });
  // власні пресети й рядок пресетів
  var presSig = '';
  function interpFrom(freqs, vals) {
    var xs = freqs.map(Math.log2);
    return EQI.freqs.map(function (f) {
      var x = Math.log2(f);
      if (x <= xs[0]) return clampEQ(vals[0]);
      for (var i = 0; i < xs.length - 1; i++) if (x <= xs[i + 1]) return clampEQ(vals[i] + (vals[i + 1] - vals[i]) * (x - xs[i]) / (xs[i + 1] - xs[i]));
      return clampEQ(vals[vals.length - 1]);
    });
  }
  function userPresets() { return (S && S.cfg.eqUser) || []; }
  function userByName(n) { return userPresets().filter(function (p) { return p.name === n; })[0]; }
  function curOf(k) { // the curve of a preset key on the current device, null if there is no such preset
    if (k.indexOf('u:') === 0) { var u = userByName(k.slice(2)); return u ? interpFrom(u.freqs, u.bands) : null; }
    var p = PRESETS.filter(function (x) { return x[0] === k; })[0];
    return p ? presetFor(p[2]) : null;
  }
  function pick(k) { var c = curOf(k); if (!c) return; eqB = c; eqP = k; eqBase = ''; nameMode(''); eqDraw(); eqSend(true); }
  // the chosen user preset differs from what is saved in it
  function userChanged() { var c = eqP.indexOf('u:') === 0 && curOf(eqP); return !!c && String(c) !== String(eqB); }
  // the preset button shows the active one; the list opens under it
  function presetLabel(k) {
    if (k === 'custom') return tx('Крива змінена');
    if (k.indexOf('u:') === 0) return k.slice(2);
    var p = PRESETS.filter(function (x) { return x[0] === k; })[0];
    return p ? tx(p[1]) : tx('Крива змінена'); // a preset of an earlier build that is gone: the curve stays
  }
  function presetIcon(k) { return k === 'custom' ? 'custom' : k.indexOf('u:') === 0 ? 'user' : (PICON[k] ? k : 'custom'); }
  function picon(k) { return '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + PICON[k] + '</svg>'; }
  function presetMarks() {
    var k = hwOn ? 'hw' : eqP;
    $('eqDropI').innerHTML = picon(hwOn ? 'game' : presetIcon(k));
    $('eqDropT').textContent = hwOn ? tx('Пресети, вбудовані в гарнітуру') : presetLabel(k) + (userChanged() ? ' •' : '');
    if (!$('eqPop').classList.contains('off')) eqPopRender();
    renderBar();
  }
  function eqPopRender() {
    function row(k, cur) {
      return '<button class="it' + (cur ? ' acc' : '') + '" data-k="' + esc(k) + '" role="option" aria-selected="' + cur + '">' + picon(presetIcon(k)) + '<span class="nm">' + esc(presetLabel(k)) + '</span>' + (cur ? '<span class="ck">✓</span>' : '') + '</button>';
    }
    var act = hwOn ? '' : eqP, h = '';
    if (act) h += row(act, true) + '<div class="psep"></div>'; // the active one on top
    PRESETS.forEach(function (p) { if (p[0] !== act) h += row(p[0], false); });
    var up = userPresets().filter(function (p) { return 'u:' + p.name !== act; });
    h += '<div class="hd">' + tx('Мої пресети') + '</div>';
    up.forEach(function (p) { h += row('u:' + p.name, false); });
    h += '<button class="it" data-new><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg><span class="nm">' + tx('Новий пресет') + '</span></button>';
    var p = $('eqPop');
    p.innerHTML = h;
    p.querySelectorAll('[data-k]').forEach(function (b) { b.onclick = function () { closePops(); if (b.dataset.k !== eqP || hwOn) pick(b.dataset.k); }; });
    p.querySelector('[data-new]').onclick = function () { closePops(); nameMode('new'); };
  }
  $('eqDrop').onclick = function (e) {
    e.stopPropagation();
    var p = $('eqPop'), b = $('eqDrop');
    if (!p.classList.contains('off')) { closePops(); return; }
    closePops(); eqPopRender();
    p.style.top = (b.offsetTop + b.offsetHeight + 4) + 'px';
    p.classList.remove('off');
  };
  // saving is green, deleting red, the rest neutral
  function barBtn(id, label, acc) { var k = id === 'eqBDel' ? ' no' : acc ? ' ok' : ''; return '<button class="chip' + k + '" id="' + id + '">' + tx(label) + '</button>'; }
  // what can be done with the current curve: save / cancel changes, rename or delete the user's preset
  function renderBar() {
    var bar = $('eqBar'), u = eqP.indexOf('u:') === 0 ? eqP.slice(2) : '', h = '', t = '';
    if (nameFor) { bar.classList.add('hide'); return; }
    if (u && userChanged()) { t = tx('Пресет змінено'); h = barBtn('eqBSave', 'Зберегти', true) + barBtn('eqBUndo', 'Скасувати'); }
    else if (u) { h = barBtn('eqBRen', 'Перейменувати') + barBtn('eqBDel', 'Видалити'); }
    else if (eqP === 'custom') { t = tx('Крива змінена'); h = barBtn('eqBNew', 'Зберегти як новий', true) + (eqBase && curOf(eqBase) ? barBtn('eqBUndo', 'Скасувати') : ''); }
    bar.classList.toggle('hide', !h);
    $('eqBarT').textContent = t; $('eqBarB').innerHTML = h;
    var on = function (id, f) { var b = $(id); if (b) b.onclick = f; };
    on('eqBSave', function () { api('/api/equser', { op: 'save', name: u, bands: eqB.slice(), freqs: EQI.freqs.slice() }).then(function () { eqSend(true); }); });
    on('eqBUndo', function () { pick(u ? eqP : eqBase); });
    on('eqBRen', function () { nameMode('rename', u); });
    on('eqBDel', function () { api('/api/equser', { op: 'delete', name: u }); eqP = 'custom'; eqBase = ''; eqSend(true); presetMarks(); });
    on('eqBNew', function () { nameMode('new'); });
  }
  // the name field: a new preset from the current curve, or a new name for the chosen one
  var nameFor = '';
  function nameMode(mode, cur) {
    nameFor = mode;
    $('eqNew').classList.toggle('hide', !mode);
    if (mode) { $('eqName').value = cur || ''; $('eqName').focus(); $('eqName').select(); }
    renderBar();
  }
  function nameOk() {
    var n = $('eqName').value.trim(), u = eqP.slice(2);
    if (!n) { $('eqName').focus(); return; }
    if (userPresets().some(function (p) { return p.name.toLowerCase() === n.toLowerCase() && !(nameFor === 'rename' && p.name === u); })) { $('eqName').focus(); $('eqName').select(); return; }
    if (nameFor === 'rename') api('/api/equser', { op: 'rename', name: u, to: n });
    else api('/api/equser', { op: 'save', name: n, bands: eqB.slice(), freqs: EQI.freqs.slice() });
    eqP = 'u:' + n; eqBase = ''; nameMode(''); eqSend(true);
  }
  function renderPresets() {
    var sig = JSON.stringify([userPresets().map(function (p) { return p.name; }), I18.lang]);
    if (sig !== presSig) presSig = sig;
    presetMarks();
  }
  $('eqName').placeholder = tx('Назва пресету');
  $('eqSaveP').onclick = nameOk;
  $('eqNameX').onclick = function () { nameMode(''); };
  $('eqName').addEventListener('keydown', function (e) { if (e.key === 'Enter') nameOk(); if (e.key === 'Escape') nameMode(''); });
  $('eqTarget').querySelectorAll('button').forEach(function (b) { b.onclick = function () { setTarget(b.dataset.t); }; });

  // ---------- профілі ----------
  $('profName').placeholder = tx('Назва профілю');
  var profSig = '';
  // the shown name of an app (exe) for a profile bound to it
  function appLabel(exe) {
    var a = ((S && S.apps) || []).filter(function (x) { return (x.exe || '').toLowerCase() === exe; })[0];
    return a ? a.name : exe.replace(/\.exe$/i, '');
  }
  function renderProfApps() {
    var seen = {}, opts = '<option value="">' + esc(tx('Без прив’язки до програми')) + '</option>';
    ((S && S.apps) || []).forEach(function (a) {
      var e = (a.exe || '').toLowerCase();
      if (!e || seen[e]) return;
      seen[e] = 1;
      opts += '<option value="' + esc(e) + '">' + esc(tx('Вмикати, поки у фокусі: %s').replace('%s', a.name)) + '</option>';
    });
    $('profApp').innerHTML = opts;
  }
  // the header button: apply a profile in one click, manage them on the Sound tab
  function profPopRender() {
    var list = (S && S.cfg.profiles) || [], apps = (S && S.cfg.profApps) || {}, p = $('profPop');
    p.innerHTML = '<div class="hd">' + tx('Профілі') + '</div>' + (list.length ? list.map(function (n, i) {
      return '<button class="it" data-i="' + i + '">' + icon('user', 15) + '<span class="nm">' + esc(n) + '</span>' + (apps[n] ? '<span class="sub">' + esc(appLabel(apps[n])) + '</span>' : '') + '</button>';
    }).join('') : '<div class="hd">' + tx('Профілів ще немає') + '</div>') +
      '<div class="psep"></div><button class="it" data-m>' + icon('sliders', 15) + '<span class="nm">' + tx('Керувати профілями') + '</span></button>';
    p.querySelectorAll('[data-i]').forEach(function (b) { b.onclick = function () { api('/api/profile', { op: 'apply', name: list[+b.dataset.i] }); closePops(); }; });
    p.querySelector('[data-m]').onclick = function () { closePops(); show('sound', 'profCard'); };
  }
  $('profBtn').onclick = function (e) {
    e.stopPropagation();
    var p = $('profPop');
    if (!p.classList.contains('off')) { closePops(); return; }
    closePops(); profPopRender(); p.classList.remove('off');
  };
  function renderProfiles(list) {
    var apps = (S && S.cfg && S.cfg.profApps) || {};
    var sig = JSON.stringify([list, apps]);
    if (sig === profSig) return;
    profSig = sig;
    $('profList').innerHTML = list.map(function (n, i) {
      return '<button class="chip" data-i="' + i + '" title="' + tx('Правий клік, а потім клік — видалити') + '"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 3h12v18l-6-4-6 4z"/></svg><span>' + esc(n) + '</span></button>';
    }).join('') + '<button class="chip" id="profAdd" style="color:var(--g)"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg>' + tx('Зберегти поточні') + '</button>';
    $('profList').querySelectorAll('[data-i]').forEach(function (b) {
      b.onclick = function () {
        if (b.dataset.armed) { api('/api/profile', { op: 'delete', name: list[+b.dataset.i] }); return; }
        api('/api/profile', { op: 'apply', name: list[+b.dataset.i] });
      };
      b.oncontextmenu = function (e) {
        e.preventDefault();
        if (b.dataset.armed) { api('/api/profile', { op: 'delete', name: list[+b.dataset.i] }); return; }
        b.dataset.armed = '1'; b.querySelector('span').textContent = tx('Видалити?'); b.style.background = 'var(--red)'; b.style.color = '#fff';
        setTimeout(function () { delete b.dataset.armed; b.querySelector('span').textContent = list[+b.dataset.i]; b.style.background = ''; b.style.color = ''; }, 3000);
      };
    });
    // a profile bound to an app says which one
    $('profList').querySelectorAll('[data-i]').forEach(function (b) {
      var a = apps[list[+b.dataset.i]];
      if (a) { b.insertAdjacentHTML('beforeend', '<i class="papp">' + esc(appLabel(a)) + '</i>'); b.title = tx('Вмикати, поки у фокусі: %s').replace('%s', appLabel(a)) + ' · ' + b.title; }
    });
    $('profAdd').onclick = function () { $('profNew').classList.toggle('hide'); if (!$('profNew').classList.contains('hide')) { renderProfApps(); $('profName').focus(); } };
  }
  function profSave() {
    var n = $('profName').value.trim();
    if (!n) { $('profName').focus(); return; }
    api('/api/profile', { op: 'save', name: n, app: $('profApp').value }); $('profName').value = ''; $('profNew').classList.add('hide');
  }
  $('profSave').onclick = profSave;
  $('profName').addEventListener('keydown', function (e) { if (e.key === 'Enter') profSave(); });

  // ---------- автоприглушення ----------
  function duckLabel(v) { $('duckV').textContent = v >= 100 ? tx('Вимк.') : v + '%'; }
  var duckSl = Slider($('duck'), { min: 10, max: 100, step: 10, value: 100, onInput: duckLabel, onChange: function (v) { patch({ duckLevel: v }); } });
  // індикатор на Arctis 7+ лише світиться або ні — яскравість він не змінює
  $('swLed').onclick = function () { if (S) patch({ muteLed: S.cfg.muteLed === 0 ? 3 : 0 }); };
  $('swDuckMedia').onclick = function () { patch({ duckMedia: !S.cfg.duckMedia }); };
  function renderDuck(s) {
    var l = s.cfg.duckLevel || 100, d = s.duck || {};
    duckSl.set(l); if (!duckSl.busy()) duckLabel(l);
    setSw('swDuckMedia', !!s.cfg.duckMedia);
    var st = $('duckSt');
    if (l >= 100) { st.textContent = tx('Вимкнено'); st.className = 'sub'; }
    else if (!d.watching) { st.textContent = tx('Чекаю на програму в групі «Чат» (наприклад, Discord).'); st.className = 'sub'; }
    else if (d.active) { st.textContent = tx('У чаті говорять — гру приглушено'); st.className = 'sub st-ok'; }
    else { st.textContent = tx('У чаті тиша — гра звучить як завжди'); st.className = 'sub'; }
    $('qDuck').classList.toggle('on', l < 100); $('qDuck').setAttribute('aria-checked', l < 100);
    $('qDuckV').textContent = l >= 100 ? tx('Вимкнено') : (d.active ? tx('приглушено') : tx('до %s').replace('%s', l + '%'));
    // Windows сама приглушує інші звуки під час дзвінків — показуємо, щоб не плутати з ChatMix
    var wc = s.winCall || {}, amt = wc.pref === 3 ? '' : (wc.pref === 0 ? '−100 %' : wc.pref === 2 ? '−50 %' : '−80 %');
    var nowWin = !!amt && wc.active;
    $('winDuck').classList.toggle('hide', !amt);
    $('winDuck').classList.toggle('warn', nowWin);
    if (amt) {
      $('winDuckNow').textContent = nowWin ? tx('Зараз триває дзвінок у %s — Windows приглушує інші звуки (%a). Arcmix тут ні до чого.').replace('%s', wc.who || 'Discord').replace('%a', amt) : '';
      $('winDuckNow').classList.toggle('hide', !nowWin);
      $('winDuckT').textContent = tx('Під час дзвінка (наприклад, у голосовому каналі Discord) Windows сама приглушує інші звуки до кінця дзвінка (%s). Це не автоприглушення Arcmix: щоб вимкнути, на вкладці «Зв’язок» оберіть «Нічого не робити».').replace('%s', amt);
    }
    $('qDuck').classList.toggle('warn', nowWin);
    if (nowWin) $('qDuckV').textContent = 'Windows ' + amt;
  }

  // рівень звуку чату наживо: видно, що саме вмикає приглушення
  function duckDraw(d) {
    (function () {
      var on = S && (S.cfg.duckLevel || 100) < 100 && d.watching;
      $('dkLvl').style.width = (on ? pct60(smooth('duck', d.db)) : 0) + '%';
      $('dkLvl').style.background = d.voice ? 'var(--g)' : 'var(--ol)';
      $('dkThr').style.left = pct60(d.thr) + '%'; $('dkThr').style.display = on ? '' : 'none';
      $('dkDb').textContent = on ? tx('звук чату') + ' ' + dbTxt(d.db) + ' ' + tx('дБ') : '';
      // звук чату повторює іншу програму (напр. Discord «звучить» як відео в Chrome) — не розмова
      var ep = on && d.echo ? d.echo.split(' ← ') : null, nm = function (x) { return (x || '').replace(/\.exe$/i, ''); };
      $('dkEcho').classList.toggle('hide', !ep);
      if (ep) $('dkEcho').textContent = tx('Рівень %a рухається разом зі звуком %b — це не розмова, гру не приглушую.').replace('%a', nm(ep[0])).replace('%b', nm(ep[1]));
    })();
  }
  $('winDuckGo').onclick = function () { api('/api/winducking', {}); };

  // ---------- маршрутизація та обробка мікрофона ----------
  var DEVS = null, devsCb = [];
  document.querySelectorAll('[data-get-cable]').forEach(function (b) { b.onclick = function () { api('/api/open', { url: 'https://vb-audio.com/Cable/' }); }; });
  // VB-CABLE A+B may not be bundled: it is sold only in the VB-Audio shop (donationware)
  document.querySelectorAll('[data-get-ab]').forEach(function (b) { b.onclick = function () { api('/api/open', { url: 'https://shop.vb-audio.com/en/audio-apps/12-vb-cable-ab.html' }); }; });
  function loadDevs(cb) {
    if (cb) devsCb.push(cb);
    api('/api/audiodevs').then(function (r) { return r.json(); }).then(function (d) {
      DEVS = d || {}; if (S) { renderAudioFx(S); renderCableWarn(S); }
      var l = devsCb; devsCb = []; l.forEach(function (f) { f(); });
    }).catch(function () { });
  }
  function virt() { return ((DEVS && DEVS.render) || []).filter(function (d) { return d.virtual; }); }
  // a cable switched off in Windows is listed too: picking it switches it on (audio::enable_cable)
  function cabName(d) { return d.name + (d.off ? ' · ' + tx('вимкнено у Windows') : ''); }
  function opt(v, label, cur, dis) { return '<option value="' + esc(v) + '"' + (v === cur ? ' selected' : '') + (dis ? ' disabled' : '') + '>' + esc(label) + '</option>'; }
  function fillSel(id, items, cur) {
    var el = $(id), html = items.join('');
    if (cur && html.indexOf('value="' + esc(cur) + '"') < 0) html += opt(cur, tx('(пристрій недоступний)'), cur);
    if (el.dataset.h !== html && document.activeElement !== el) { el.innerHTML = html; el.dataset.h = html; }
    if (document.activeElement !== el) el.value = cur || '';
  }
  function pipeText(p, ready) {
    if (!p || !p.on) return [tx('Вимкнено'), ''];
    if (p.err) return [tx('Помилка') + ': ' + (p.hint ? tx(p.hint) : p.err), 'st-err'];
    if (!ready) return [tx('Оберіть віртуальний кабель'), ''];
    if (p.running) return [tx('Працює') + ' · ' + tx('затримка') + ' ~' + p.latency + ' ' + tx('мс'), 'st-ok'];
    return [tx('Запуск…'), ''];
  }
  function dbLabel(v, off) { return off ? tx('Вимк.') : v + ' ' + tx('дБ'); }
  var fxSend = throttle(function (o) { api('/api/micfx', o); }, 120);
  // NVIDIA: what it takes out (noise, room echo or both) and how strongly
  var nvSl = Slider($('nvStr'), { min: 0, max: 100, step: 5, value: 100, onInput: function (v) { $('nvStrV').textContent = v + '%'; fxSend({ nvStrength: v }); }, onChange: function (v) { fxSend({ nvStrength: v }, true); } });
  var gateSl = Slider($('mfxGate'), { min: -80, max: -20, step: 1, value: -50, onInput: function (v) { $('mfxGateV').textContent = dbLabel(v, v <= -80); fxSend({ gate: v <= -80 ? -100 : v }); }, onChange: function (v) { fxSend({ gate: v <= -80 ? -100 : v }, true); } });
  // compressor: threshold, ratio (tenths here) and make-up gain
  var compDbSl = Slider($('compDb'), { min: -60, max: 0, step: 1, value: -24, onInput: function (v) { $('compDbV').textContent = dbLabel(v); fxSend({ compDb: v }); }, onChange: function (v) { fxSend({ compDb: v }, true); } });
  var compRatioSl = Slider($('compRatio'), { min: 15, max: 100, step: 5, value: 30, onInput: function (v) { $('compRatioV').textContent = v / 10 + ':1'; fxSend({ compRatio: v / 10 }); }, onChange: function (v) { fxSend({ compRatio: v / 10 }, true); } });
  var compGainSl = Slider($('compGain'), { min: 0, max: 18, step: 1, value: 6, onInput: function (v) { $('compGainV').textContent = '+' + dbLabel(v); fxSend({ compGain: v }); }, onChange: function (v) { fxSend({ compGain: v }, true); } });
  $('swComp').onclick = function () { api('/api/micfx', { comp: !(S.cfg.micFx || {}).comp }); };
  // the movable stages between the low cut and the limiter (dsp::MIC_STAGES): the chain shows their
  // settings in a fixed layout; the processing order is a setting of its own (micTab page "order")
  var STAGES = ['denoise', 'comp', 'plugins', 'gate'], STAGE_N = { denoise: 'Шумодав', plugins: 'VST-плагіни', gate: 'Поріг шумодаву', comp: 'Компресор' };
  var ordSig = '', ordDrag = false;
  function renderOrder(o) {
    var l = [];
    (o || []).concat(STAGES).forEach(function (k) { if (STAGES.indexOf(k) >= 0 && l.indexOf(k) < 0) l.push(k); });
    $('ordSum').textContent = l.map(function (k) { return tx(STAGE_N[k]); }).join(' → ');
    var sig = l.join() + I18.lang;
    if (ordDrag || sig === ordSig) return;
    ordSig = sig;
    $('ordList').innerHTML = l.map(function (k, i) {
      return '<div class="od" data-id="' + k + '"><button class="dh" data-act="grip" aria-label="' + esc(tx('Перетягнути')) + ' ' + esc(tx(STAGE_N[k])) + '"><svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="9" cy="6" r="1.6"/><circle cx="15" cy="6" r="1.6"/><circle cx="9" cy="12" r="1.6"/><circle cx="15" cy="12" r="1.6"/><circle cx="9" cy="18" r="1.6"/><circle cx="15" cy="18" r="1.6"/></svg></button><span class="pn">' + (i + 2) + '</span><div class="dn"><span class="t">' + tx(STAGE_N[k]) + '</span></div></div>';
    }).join('');
    $('ordList').querySelectorAll('.dh').forEach(function (h) {
      listDrag(h, $('micTab'), function (busy) { ordDrag = busy; }, function (ids) {
        [].forEach.call($('ordList').children, function (r, i) { r.querySelector('.pn').textContent = i + 2; });
        ordSig = ''; api('/api/micfx', { order: ids });
      });
    });
  }
  // auto-off of the headset: a list of the times the supported headsets take (0 — never; below 0 — not set
  // by the app yet, the headset keeps its own)
  var OFFS = [0, 1, 5, 10, 15, 30, 60, 90];
  function offLabel(v) { return v === 0 ? tx('Ніколи') : v > 0 ? v + ' ' + tx('хв') : '—'; }
  $('offDrop').onclick = function (e) {
    e.stopPropagation();
    var p = $('offPop'), b = $('offDrop'), cur = S ? S.cfg.autoOff : -1;
    if (!p.classList.contains('off')) { closePops(); return; }
    closePops();
    p.innerHTML = OFFS.map(function (v) {
      var on = v === cur;
      return '<button class="it' + (on ? ' acc' : '') + '" data-v="' + v + '" role="option" aria-selected="' + on + '"><span class="nm">' + offLabel(v) + '</span>' +
        (on ? '<span class="ck"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m5 12 5 5 9-10"/></svg></span>' : '') + '</button>';
    }).join('');
    p.querySelectorAll('[data-v]').forEach(function (it) { it.onclick = function () { patch({ autoOff: Number(it.dataset.v) }); closePops(); }; });
    p.style.top = (b.offsetTop + b.offsetHeight + 4) + 'px';
    p.classList.remove('off');
  };
  $('ordReset').onclick = function () { ordSig = ''; api('/api/micfx', { order: [] }); };
  // a short list reordered by a row's handle: the neighbours move around the held row (the held one
  // stays in the DOM, so its pointer capture lasts); ↑ / ↓ on the handle move it one place
  function listDrag(h, scroller, busy, done) {
    var row = h.closest('[data-id]'), list = row.parentNode, st = null;
    var ids = function () { return [].map.call(list.children, function (r) { return r.dataset.id; }); };
    h.addEventListener('keydown', function (e) {
      var d = { ArrowUp: -1, ArrowDown: 1 }[e.key];
      if (!d) return;
      e.preventDefault();
      var l = ids(), i = l.indexOf(row.dataset.id), j = i + d;
      if (j < 0 || j >= l.length) return;
      l[i] = l[j]; l[j] = row.dataset.id;
      done(l);
    });
    h.addEventListener('pointerdown', function (e) {
      if (e.button) return;
      e.preventDefault();
      h.setPointerCapture(e.pointerId);
      st = { before: ids().join(), grab: e.clientY - row.getBoundingClientRect().top };
      busy(true);
      row.classList.add('lift');
    });
    h.addEventListener('pointermove', function (e) {
      if (!st) return;
      row.style.transform = '';
      var base = row.getBoundingClientRect(), yc = e.clientY - st.grab + base.height / 2;
      var rows = [].slice.call(list.children), target = null;
      for (var k = 0; k < rows.length; k++) {
        var r = rows[k].getBoundingClientRect();
        if (rows[k] !== row && yc < r.top + r.height / 2) { target = rows[k]; break; }
      }
      if ((target === null && list.lastElementChild !== row) || (target && row.nextElementSibling !== target)) {
        var pos = rows.map(function (r) { return r.getBoundingClientRect().top; });
        var down = target === null || rows.indexOf(target) > rows.indexOf(row);
        while (row.nextElementSibling !== target) {
          if (down) list.insertBefore(row.nextElementSibling, row);
          else list.insertBefore(row.previousElementSibling, row.nextElementSibling);
        }
        rows.forEach(function (r, i) {
          if (r === row) return;
          var d = pos[i] - r.getBoundingClientRect().top;
          if (!d) return;
          r.style.transition = 'none'; r.style.transform = 'translateY(' + d + 'px)';
          requestAnimationFrame(function () { r.style.transition = 'transform .16s var(--ease)'; r.style.transform = ''; });
        });
        base = row.getBoundingClientRect();
      }
      var lb = list.getBoundingClientRect();
      var t = Math.max(lb.top - base.top, Math.min(lb.bottom - base.bottom, e.clientY - st.grab - base.top));
      row.style.transition = 'none'; row.style.transform = 'translateY(' + t + 'px)';
      var sr = scroller.getBoundingClientRect();
      if (e.clientY < sr.top + 28) scroller.scrollTop -= 8; else if (e.clientY > sr.bottom - 28) scroller.scrollTop += 8;
    });
    function end() {
      if (!st) return;
      var changed = ids().join() !== st.before;
      st = null;
      row.classList.remove('lift');
      row.style.transition = 'transform .16s var(--ease)'; row.style.transform = '';
      busy(false);
      if (changed) setTimeout(function () { done(ids()); }, 170);
    }
    h.addEventListener('pointerup', end);
    h.addEventListener('lostpointercapture', end);
  }
  var ceilSl = Slider($('mfxCeil'), { min: -30, max: 0, step: 1, value: -6, onInput: function (v) { $('mfxCeilV').textContent = dbLabel(v, v >= 0); fxSend({ ceil: v }); }, onChange: function (v) { fxSend({ ceil: v }, true); } });
  function renderAudioFx(s) {
    var c = s.cfg, mf = c.micFx || {}, rc = c.route || {}, vs = virt(), have = DEVS !== null, noCable = have && !vs.length;
    // the cables off (Settings → Devices) stop the processing whatever its own switch says
    var t = mf.on && c.cables === false ? [tx('Вимкнено разом з віртуальними кабелями'), ''] : pipeText(s.micFx, !!mf.out);
    $('mfxStatus').textContent = t[0]; $('mfxStatus').className = 'sub ' + t[1];
    setSw('swMfx', !!mf.on); setSw('swDenoise', !!mf.denoise);
    // the kind of denoiser: RNNoise or NVIDIA (its SDK installed by the user, an RTX graphics card)
    var kind = mf.denoiseKind === 'nv' ? 'nv' : 'rnn', nv = s.nv || {};
    $('dnBox').classList.toggle('hide', !mf.denoise);
    pressed('dnKind', kind);
    $('dnHint').textContent = tx(kind === 'nv' ? 'NVIDIA RTX · найчистіший голос, на відеокарті; програма займатиме ~200 МБ пам’яті більше' : 'RNNoise · найменша затримка, +10 мс');
    var nvNote = kind === 'nv' && nv.state !== 'ready' ? (nv.state === 'loading' ? tx('Шумодав NVIDIA завантажується (перший раз — до пів хвилини). Поки що працює RNNoise.')
      : nv.state === 'failed' ? tx('Шумодав NVIDIA не запустився — працює RNNoise. Потрібна відеокарта NVIDIA RTX.')
      : tx('Потрібна відеокарта NVIDIA RTX і NVIDIA Audio Effects (безкоштовно, з сайту NVIDIA): оберіть свою серію RTX у розділі Audio Effects. Поки його немає, працює RNNoise.')) : '';
    $('dnModel').classList.toggle('hide', !nvNote);
    $('dnModelT').textContent = nvNote;
    $('dnGet').classList.toggle('hide', nv.state !== 'missing');
    var nvOn = kind === 'nv' && nv.state !== 'missing', nvm = nv.modes || [];
    $('nvOpts').classList.toggle('hide', !nvOn);
    if (nvOn) {
      pressed('nvMode', mf.nvMode || 'denoise');
      $('nvMode').querySelectorAll('button').forEach(function (b) { b.disabled = nvm.length > 0 && nvm.indexOf(b.dataset.v) < 0; });
      var nst = mf.nvStrength == null ? 100 : mf.nvStrength;
      nvSl.set(nst); if (!nvSl.busy()) $('nvStrV').textContent = nst + '%';
    }
    $('qNs').classList.toggle('on', !!(mf.on && mf.denoise)); $('qNs').setAttribute('aria-checked', !!(mf.on && mf.denoise));
    $('qNsV').textContent = !(mf.on && mf.denoise) ? tx('Вимкнено') : cableTrouble(s) ? tx('Немає кабелю') : (s.micFx && s.micFx.running ? (kind === 'nv' && nv.state === 'ready' ? 'NVIDIA RTX' : 'RNNoise') : (s.micFx && s.micFx.err ? tx('Помилка') : tx('Запуск…')));
    // routing: on, and which categories play through a cable
    var rcq = c.route || {}, rcats = ['game', 'chat', 'media'].filter(function (k) { return rcq[k] && !(mf.on && rcq[k] === mf.out); });
    $('qRt').classList.toggle('on', !!rcq.on); $('qRt').setAttribute('aria-checked', !!rcq.on);
    $('qRtV').textContent = !rcq.on ? tx('Вимкнено') : (s.route && s.route.running ? rcats.map(function (k) { return tx({ game: 'Гра', chat: 'Чат', media: 'Медіа' }[k]); }).join(', ') : (s.route && s.route.err ? tx('Помилка') : tx('Запуск…')));
    $('rtSum').textContent = $('qRtV').textContent;
    $('mfxNoCable').classList.toggle('hide', !noCable);
    $('vstSafe').classList.toggle('hide', !s.cfg.vstSafe);
    $('mfxBody').classList.toggle('hide', !mf.on || noCable);
    var used = [rc.on && rc.game, rc.on && rc.chat, rc.on && rc.media];
    if (have) {
      var md = c.micDev || '';
      fillSel('mfxIn', [opt('', tx('Як у Windows') + (defName() ? ' — ' + defName() : ''), md)].concat(((DEVS.capture) || []).filter(function (d) { return !d.virtual || d.id === md; }).map(function (d) { return opt(d.id, d.name, md); })), md);
      fillSel('mfxOut', [opt('', tx('— оберіть кабель —'), mf.out)].concat(vs.map(function (d) { return opt(d.id, cabName(d), mf.out, used.indexOf(d.id) >= 0); })), mf.out);
    }
    var pair = (vs.filter(function (d) { return d.id === mf.out; })[0] || {}).pair;
    // with a known pair the app makes it Windows' default microphone while processing runs (audio::sync_fx_mic)
    $('mfxHint').textContent = mf.out ? (pair ? tx('Поки обробка ввімкнена, мікрофоном Windows стоїть «%s»: Discord, гра й OBS з мікрофоном за замовчуванням чують оброблений голос.').replace('%s', pair) : tx('У Discord, грі чи OBS оберіть мікрофоном інший кінець цього кабелю.')) : '';
    gateSl.set(Math.max(-80, mf.gate)); if (!gateSl.busy()) $('mfxGateV').textContent = dbLabel(Math.max(-80, mf.gate), mf.gate <= -80);
    ceilSl.set(Math.min(0, mf.ceil)); if (!ceilSl.busy()) $('mfxCeilV').textContent = dbLabel(Math.min(0, mf.ceil), mf.ceil >= 0);
    setSw('swComp', !!mf.comp); $('compBox').classList.toggle('hide', !mf.comp);
    var cd = mf.compDb == null ? -24 : mf.compDb, cr = Math.round((mf.compRatio || 3) * 10), cg = mf.compGain == null ? 6 : mf.compGain;
    compDbSl.set(cd); if (!compDbSl.busy()) $('compDbV').textContent = dbLabel(cd);
    compRatioSl.set(cr); if (!compRatioSl.busy()) $('compRatioV').textContent = cr / 10 + ':1';
    compGainSl.set(cg); if (!compGainSl.busy()) $('compGainV').textContent = '+' + dbLabel(cg);
    renderOrder(mf.order);
    renderVst(s);
    t = rc.on && c.cables === false ? [tx('Вимкнено разом з віртуальними кабелями'), ''] : pipeText(s.route, ['game', 'chat', 'media'].some(function (k) { return rc[k] && !(mf.on && rc[k] === mf.out); }));
    $('rtStatus').textContent = t[0]; $('rtStatus').className = 'sub ' + t[1];
    setSw('swRt', !!rc.on); setSw('swRtAuto', !!rc.auto);
    $('rtNoCable').classList.toggle('hide', !noCable);
    $('rtBody').classList.toggle('hide', !rc.on || noCable);
    // the cable of the processed microphone is not routed (Config::route_cable): say so when it is the
    // only cable or a category points at it
    var mcab = mf.on ? vs.filter(function (d) { return d.id === mf.out; })[0] : null;
    var busy = have && !!mcab && (vs.length === 1 || [rc.game, rc.chat, rc.media].indexOf(mcab.id) >= 0);
    $('rtBusy').classList.toggle('hide', !busy);
    if (busy) $('rtBusyT').textContent = tx('«%s» передає мікрофон з обробкою, тому для гри, чату й медіа не підходить: інакше співрозмовники чули б ваш звук. Кожній категорії потрібен свій кабель.').replace('%s', mcab.name);
    // where more cables come from, while there is none left for routing
    $('rtMore').classList.toggle('hide', !(have && vs.filter(function (d) { return !(mf.on && d.id === mf.out); }).length === 0));
    $('rtManual').classList.toggle('hide', s.appRoute !== false || !rc.auto);
    if (have) {
      var micOut = mf.on ? mf.out : '';
      // a cable taken by the microphone or another category cannot be picked; its label says by what
      var cab = function (k) {
        return [opt('', tx('— не направляти —'), rc[k])].concat(vs.map(function (d) {
          var by = d.id === micOut ? 'Мікрофон' : ({ game: 'Гра', chat: 'Чат', media: 'Медіа' })[['game', 'chat', 'media'].filter(function (o) { return o !== k && d.id === rc[o]; })[0]];
          return opt(d.id, cabName(d) + (by ? ' · ' + tx(by) : ''), rc[k], !!by);
        }));
      };
      fillSel('rtGame', cab('game'), rc.game);
      fillSel('rtChat', cab('chat'), rc.chat);
      fillSel('rtMedia', cab('media'), rc.media);
      var defOut = ((DEVS.render) || []).filter(function (d) { return d.default; })[0];
      fillSel('rtOut', [opt('', tx('Як у Windows') + (defOut ? ' — ' + defOut.name : ''), rc.out)].concat(((DEVS.render) || []).filter(function (d) { return !d.virtual; }).map(function (d) { return opt(d.id, d.name, rc.out); })), rc.out);
    }
  }
  $('mfxIn').onchange = function () { api('/api/mic', { dev: this.value }); };
  // a cable switched off in Windows is switched on when picked: the lists anew after that
  function cabPicked(id) { if (virt().some(function (d) { return d.id === id && d.off; })) setTimeout(function () { loadDevs(); }, 1500); }
  $('mfxOut').onchange = function () { api('/api/micfx', { out: this.value }); cabPicked(this.value); };
  $('swDenoise').onclick = function () { api('/api/micfx', { denoise: !(S.cfg.micFx || {}).denoise }); };
  $('dnKind').querySelectorAll('button').forEach(function (b) { b.onclick = function () { api('/api/micfx', { denoiseKind: b.dataset.v }); }; });
  $('nvMode').querySelectorAll('button').forEach(function (b) { b.onclick = function () { api('/api/micfx', { nvMode: b.dataset.v }); }; });
  // перший кабель, не зайнятий маршрутизацією
  function freeMicCable() { var rc = S.cfg.route || {}; return virt().filter(function (d) { return !(rc.on && (d.id === rc.game || d.id === rc.chat || d.id === rc.media)); })[0]; }
  function toggleMfx(o) {
    var mf = S.cfg.micFx || {};
    o = o || {}; o.on = !mf.on;
    if (o.on && !mf.out) {
      var free = freeMicCable();
      if (free) o.out = free.id;
    }
    api('/api/micfx', o);
  }
  $('swMfx').onclick = function () { toggleMfx(); };
  // the processing is on but its cable is not there: none installed (VB-CABLE not installed or its driver
  // not loaded), the chosen one gone, or it is disabled in Windows — '' when the cable is fine
  function cableTrouble(s) {
    var c = s.cfg, mf = c.micFx || {};
    if (!mf.on || c.cables === false || DEVS === null) return '';
    var vs = virt();
    if (!vs.length) return 'none';
    if (!mf.out) return '';
    var d = vs.filter(function (x) { return x.id === mf.out; })[0];
    return !d ? 'gone' : d.off ? 'off' : '';
  }
  // an app records from a virtual cable that gets no microphone while processing is off
  var cabAct = '';
  function renderCableWarn(s) {
    var cl = s.cable || {}, c = s.cfg, mf = c.micFx || {}, f = c.features || {};
    var defCab = ((DEVS && DEVS.capture) || []).some(function (d) { return d.default && d.virtual; });
    var cabOff = c.cables === false && !!mf.on && (!!cl.who || defCab);
    var trouble = cableTrouble(s);
    var show = !!trouble || cabOff || (!!cl.who && (!mf.on || !mf.out) && (f.denoise !== false || f.vst !== false));
    $('cableWarn').classList.toggle('hide', !show);
    cabAct = trouble || (cabOff ? 'cables' : 'fx');
    $('cableWarnGo').textContent = tx({ none: 'Завантажити VB-CABLE', gone: 'Обрати кабель', off: 'Увімкнути кабель', cables: 'Увімкнути віртуальні кабелі', fx: 'Увімкнути обробку мікрофона' }[cabAct]);
    if (trouble === 'none') $('cableWarnT').textContent = tx('Шумодав і обробка мікрофона не працюють: немає віртуального кабелю. Встановіть безкоштовний VB-CABLE і перезапустіть ПК.');
    else if (trouble === 'gone') $('cableWarnT').textContent = tx('Шумодав і обробка мікрофона не працюють: кабель, обраний для мікрофона, не знайдено. Встановіть VB-CABLE знову або оберіть інший кабель.');
    else if (trouble === 'off') $('cableWarnT').textContent = tx('Шумодав і обробка мікрофона не працюють: кабель, обраний для мікрофона, вимкнено у Windows.');
    else if (cabOff) $('cableWarnT').textContent = tx('Віртуальні кабелі вимкнено: у кабель не йде оброблений мікрофон, і програми, що слухають кабель, вас не почують.');
    else if (show) $('cableWarnT').textContent = tx('%s бере мікрофон з «%s», але туди зараз нічого не йде: обробку мікрофона вимкнено.').replace('%s', cl.who).replace('%s', cl.name);
  }
  $('cableWarnGo').onclick = function () {
    var mf = (S && S.cfg.micFx) || {};
    if (cabAct === 'none') { api('/api/open', { url: 'https://vb-audio.com/Cable/' }); return; }
    if (cabAct === 'gone') { show('micTab', 'mfxCard'); return; }
    if (cabAct === 'off') { api('/api/micfx', { out: mf.out }); cabPicked(mf.out); return; }
    if (cabAct === 'cables') { patch({ cables: true }); return; }
    var cl = (S && S.cable) || {}; if (cl.out) api('/api/micfx', { on: true, out: cl.out });
  };
  $('swRt').onclick = function () {
    var rc = S.cfg.route || {}, o = { on: !rc.on };
    if (o.on && !rc.game && !rc.chat && !rc.media) { // самі обираємо вільні кабелі: перший — грі, другий — чату, третій — медіа
      var mo = (S.cfg.micFx || {}).on ? (S.cfg.micFx || {}).out : '', free = virt().filter(function (d) { return d.id !== mo; });
      if (free[0]) o.game = free[0].id;
      if (free[1]) o.chat = free[1].id;
      if (free[2]) o.media = free[2].id;
      [o.game, o.chat, o.media].forEach(function (x) { if (x) cabPicked(x); });
    }
    api('/api/route', o);
  };
  $('swRtAuto').onclick = function () { api('/api/route', { auto: !(S.cfg.route || {}).auto }); };
  ['rtGame', 'rtChat', 'rtMedia', 'rtOut'].forEach(function (id) {
    $(id).onchange = function () { var o = {}; o[{ rtGame: 'game', rtChat: 'chat', rtMedia: 'media', rtOut: 'out' }[id]] = this.value; api('/api/route', o); cabPicked(this.value); };
  });
  // Рівень мікрофона зі шкалою −60…0 дБ і зонами: фон / тихо / добре / голосно / перевантаження.
  // Коли працює обробка — ще й тонка смуга «що чують інші» та риски порогу й межі.
  function pct60(db) { return Math.max(0, Math.min(100, (db + 60) / 60 * 100)); }
  $('vuScale').innerHTML = [-60, -50, -40, -30, -20, -10, 0].map(function (d) { return '<span style="left:' + pct60(d) + '%">' + (d ? '−' + (-d) : '0') + '</span>'; }).join('');
  var VUZ = [[-1, 'перевантаження', 'var(--red)'], [-6, 'голосно', 'var(--warn)'], [-30, 'добре', 'var(--okc)'], [-50, 'тихий звук', 'var(--okc)'], [-999, 'тиша', 'var(--ol)']];
  function vuZone(db) { for (var i = 0; i < VUZ.length; i++) if (db >= VUZ[i][0]) return VUZ[i]; return VUZ[4]; }
  var vuHold = -120, vuHoldAt = 0, vuWordDb = -120, vuWordAt = 0;
  function dbTxt(v) { return v <= -99 ? '−∞' : (v < 0 ? '−' : '') + Math.abs(Math.round(v)); }
  function vuDraw(m) {
    var run = !!m.run;
    (function () {
      if (!S) return;
      var raw = run ? (m.ok ? m.in : -120) : (m.ok ? m.db : -120), db = smooth('in', raw), now = Date.now(), mf = S.cfg.micFx || {};
      if (raw > vuHold || now - vuHoldAt > 1200) { vuHold = raw; vuHoldAt = now; }
      if (raw > vuWordDb || now - vuWordAt > 700) { vuWordDb = raw; vuWordAt = now; } // the word does not flicker
      var z = vuZone(db), zw = vuZone(vuWordDb);
      $('vuIn').style.width = pct60(db) + '%'; $('vuIn').style.background = z[2];
      $('vuPk').style.left = pct60(vuHold) + '%';
      $('vuOutBox').style.visibility = run ? '' : 'hidden'; $('vuLegOut').style.visibility = run ? '' : 'hidden';
      $('vuGate').classList.toggle('hide', !run || mf.gate <= -100); $('vuCeil').classList.toggle('hide', !run || mf.ceil >= 0);
      if (run) {
        $('vuOut').style.width = pct60(smooth('out', m.out)) + '%';
        $('vuGate').style.left = pct60(Math.max(-60, mf.gate)) + '%'; $('vuCeil').style.left = pct60(mf.ceil) + '%';
      }
      var gated = run && !m.open && vuWordDb > -50;
      $('vuWord').textContent = m.ok ? (gated ? tx('нижче порогу') : tx(zw[1])) : tx('мікрофон не знайдено');
      $('vuWord').style.color = m.ok && !gated ? zw[2] : 'var(--ov)';
      $('vuDbIn').textContent = m.ok ? dbTxt(db) + ' ' + tx('дБ') : '—';
      $('vuDbOut').textContent = run ? dbTxt(m.out) + ' ' + tx('дБ') : '—';
    })();
  }

  // ---------- VST-плагіни ----------
  var VSTERR = { '32-bit': '32-бітний плагін — потрібна 64-бітна версія', 'not a VST2 plugin': 'Це не VST2-плагін', 'not a VST3 plugin': 'Це не VST3-плагін', 'not an effect': 'Це не ефект (інструмент або без входу)', 'unsupported (no 32-bit float)': 'Плагін не підтримує потрібний формат звуку', 'cannot create plugin': 'Плагін не запустився', 'cannot activate plugin': 'Плагін не запустився', 'InitDll failed': 'Плагін не запустився' };
  var vstSig = '';
  function renderVst(s) {
    var list = s.vst || [], slots = ((s.cfg.micFx || {}).plugins) || [], box = $('vstList');
    var sig = JSON.stringify([list, slots.map(function (x) { return [x.id, x.on]; })]);
    if (sig === vstSig) return;
    vstSig = sig;
    var on = {}; slots.forEach(function (x) { on[x.id] = x.on; });
    box.innerHTML = list.map(function (v, i) {
      var err = v.err ? tx(VSTERR[v.err] || 'Не вдалося завантажити') + (VSTERR[v.err] ? '' : ': ' + v.err) : '';
      return '<div class="vrow" data-id="' + esc(v.id) + '">' +
        '<button class="sw" role="switch" data-act="toggle" aria-checked="' + (!!on[v.id]) + '" aria-label="' + esc(v.name) + '"><i></i></button>' +
        '<div class="vn"><div>' + esc(v.name) + '</div>' + (err ? '<div class="sub st-err">' + esc(err) + '</div>' : '') + '</div>' +
        (v.editor ? '<button class="chip" data-act="edit">' + tx('Вікно') + '</button>' : '') +
        (i > 0 ? '<button class="ib" data-act="up" title="' + tx('Вище') + '" aria-label="' + tx('Вище') + '"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5M6 11l6-6 6 6"/></svg></button>' : '') +
        '<button class="ib" data-act="remove" title="' + tx('Прибрати') + '" aria-label="' + tx('Прибрати') + '"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><path d="M6 6l12 12M18 6L6 18"/></svg></button>' +
        '</div>';
    }).join('');
    box.querySelectorAll('[data-act]').forEach(function (b) {
      b.onclick = function () {
        var id = b.closest('.vrow').dataset.id, act = b.dataset.act;
        api('/api/vst', { op: act, id: id, on: act === 'toggle' ? b.getAttribute('aria-checked') !== 'true' : false });
      };
    });
  }
  $('vstAdd').onclick = function () {
    var pick = $('vstPick');
    if (!pick.classList.contains('hide')) { pick.classList.add('hide'); return; }
    pick.innerHTML = '<div class="sub" style="padding:8px">' + tx('Шукаю плагіни…') + '</div>'; pick.classList.remove('hide');
    api('/api/vst/scan').then(function (r) { return r.json(); }).then(function (l) {
      l = l || [];
      pick.innerHTML = (l.length ? l.map(function (p, i) { return '<button data-i="' + i + '"><div>' + esc(p.name) + '</div><div class="sub" style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">' + esc(p.path) + '</div></button>'; }).join('')
        : '<div class="sub" style="padding:8px">' + tx('У стандартних теках VST плагінів не знайдено.') + '</div>') +
        '<button data-file style="color:var(--g);font-weight:500">' + tx('Обрати файл…') + '</button>';
      pick.querySelectorAll('[data-i]').forEach(function (b) { b.onclick = function () { api('/api/vst', { op: 'add', path: l[+b.dataset.i].path }); pick.classList.add('hide'); }; });
      pick.querySelector('[data-file]').onclick = function () { api('/api/vst/pick', {}); pick.classList.add('hide'); };
    }).catch(function () { pick.classList.add('hide'); });
  };

  // ---------- «послухати себе» ----------
  function renderMon(m) {
    m = m || {};
    var rec = m.mode === 'record', play = m.mode === 'play', live = m.mode === 'live';
    $('monRecT').textContent = rec ? tx('Зупинити запис') : (m.has ? tx('Записати знову') : tx('Записати 5 с'));
    $('monRec').classList.toggle('on', rec);
    $('monPlayT').textContent = play ? tx('Зупинити') : tx('Прослухати');
    $('monPlayI').setAttribute('d', play ? 'M2 2h8v8H2z' : 'M2.5 1.5l8 4.5-8 4.5z');
    $('monPlay').disabled = !m.has && !play; $('monPlay').style.opacity = (m.has || play) ? 1 : .45;
    $('monBar').style.width = ((rec || play) ? Math.round(m.progress * 100) : 0) + '%';
    $('monProg').classList.toggle('idle', !rec && !play);
    setSw('swMonLive', live);
    $('monHint').textContent = (m.tap ? tx('Чути так, як вас чують інші — після обробки мікрофона.') : tx('Чути мікрофон без обробки. Увімкніть обробку нижче, щоб почути шумодав.')) + ' ' + tx('Слухайте в навушниках, щоб не було луни.');
  }
  $('monRec').onclick = function () { api('/api/monitor', { op: S && S.monitor && S.monitor.mode === 'record' ? 'stop' : 'record' }); };
  $('monPlay').onclick = function () { api('/api/monitor', { op: S && S.monitor && S.monitor.mode === 'play' ? 'stop' : 'play' }); };
  $('swMonLive').onclick = function () { api('/api/monitor', { op: S && S.monitor && S.monitor.mode === 'live' ? 'stop' : 'live' }); };
  function monTick() { api('/api/monitor').then(function (r) { return r.json(); }).then(function (m) { if (S) { S.monitor = m; renderMon(m); } }).catch(function () { }); }

  // ---------- клавіші: «натисни, щоб говорити» й гарячі клавіші ----------
  function keyName(c) {
    if (!c) return '—';
    if (c === 4) return tx('Середня кнопка миші');
    if (c === 5) return tx('Бічна кнопка миші') + ' 1';
    if (c === 6) return tx('Бічна кнопка миші') + ' 2';
    var named = { 8: 'Backspace', 9: 'Tab', 13: 'Enter', 16: 'Shift', 17: 'Ctrl', 18: 'Alt', 19: 'Pause', 20: 'Caps Lock', 32: 'Space', 33: 'Page Up', 34: 'Page Down', 35: 'End', 36: 'Home', 37: '←', 38: '↑', 39: '→', 40: '↓', 45: 'Insert', 46: 'Delete', 144: 'Num Lock', 145: 'Scroll Lock', 186: ';', 187: '=', 188: ',', 189: '-', 190: '.', 191: '/', 192: '`', 219: '[', 220: '\\', 221: ']', 222: '"' };
    if (named[c]) return named[c];
    if (c >= 112 && c <= 135) return 'F' + (c - 111);
    if (c >= 96 && c <= 105) return 'Num ' + (c - 96);
    if ((c >= 48 && c <= 57) || (c >= 65 && c <= 90)) return String.fromCharCode(c);
    return '#' + c;
  }
  function comboKeys(h) {
    var k = [];
    if (h.mod & 2) k.push('Ctrl'); if (h.mod & 1) k.push('Alt'); if (h.mod & 4) k.push('Shift'); if (h.mod & 8) k.push('Win');
    k.push(keyName(h.vk));
    return k;
  }
  // while a key is being picked the app's hotkeys and push-to-talk are off: pressing the same keys sets them
  // instead of doing their job
  var capKind = '', capName = '';
  function capStart(kind, name) { if (!capKind) api('/api/hkcapture', { on: true }); capKind = kind; capName = name || ''; renderKeys(); }
  function capEnd() { if (capKind) api('/api/hkcapture', { on: false }); capKind = ''; capName = ''; if (S) renderKeys(); }
  document.addEventListener('visibilitychange', function () { if (document.hidden && capKind) capEnd(); });
  document.addEventListener('keydown', function (e) {
    if (!capKind) return;
    e.preventDefault(); e.stopPropagation();
    if (e.keyCode === 27) { capEnd(); return; }
    if (capKind === 'ptt') { patch({ pttKey: e.keyCode, ptt: true }); capEnd(); return; }
    if ([16, 17, 18, 91, 92].indexOf(e.keyCode) >= 0) return; // чекаємо на основну клавішу
    var mod = (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.shiftKey ? 4 : 0) | (e.metaKey ? 8 : 0);
    if (!mod && !(e.keyCode >= 112 && e.keyCode <= 135)) return; // без модифікатора — лише F1…F24
    var o = {}, hk = (S && S.cfg.hotkeys) || {}, nk = { mod: mod, vk: e.keyCode };
    o[capName] = nk;
    // the combination of another action: the two swap
    HK.forEach(function (h) { var x = hk[h[0]]; if (h[0] !== capName && x && x.mod === mod && x.vk === e.keyCode && hk[capName]) o[h[0]] = hk[capName].vk ? hk[capName] : { mod: 0, vk: 0 }; });
    patch({ hotkeys: o }); capEnd();
  }, true);
  document.addEventListener('mousedown', function (e) {
    var m = { 1: 4, 3: 5, 4: 6 }[e.button];
    if (capKind === 'ptt' && m) { e.preventDefault(); e.stopPropagation(); patch({ pttKey: m, ptt: true }); capEnd(); }
  }, true);
  document.addEventListener('mouseup', function (e) { if (e.button === 3 || e.button === 4) e.preventDefault(); }, true);
  $('pttSet').onclick = function () { if (capKind === 'ptt') capEnd(); else capStart('ptt'); };
  $('pttClear').onclick = function () { if (capKind === 'ptt') capEnd(); patch({ pttKey: 0, ptt: false }); };
  $('swPtt').onclick = function () {
    var on = !S.cfg.ptt;
    patch({ ptt: on });
    if (!on && capKind === 'ptt') capEnd();
  };
  var HK = [['game', 'Більше гри'], ['chat', 'Більше чату'], ['balance', 'Порівну'], ['mic', 'Вимкнути / увімкнути мікрофон'], ['out', 'Наступний пристрій виводу'], ['outPrev', 'Попередній пристрій виводу'], ['in', 'Наступний пристрій вводу'], ['inPrev', 'Попередній пристрій вводу']];
  function renderKeys() {
    if (!S) return;
    var c = S.cfg, hk = c.hotkeys || {}, err = S.hotkeyErr || [];
    $('pttClear').classList.toggle('hide', !c.pttKey || capKind === 'ptt');
    if (capKind !== 'ptt') { $('pttKeyName').textContent = keyName(c.pttKey); $('pttSet').textContent = c.pttKey ? tx('Змінити') : tx('Обрати клавішу'); }
    else { $('pttKeyName').textContent = tx('Натисніть клавішу…'); $('pttSet').textContent = tx('Скасувати'); }
    $('pttSet').classList.toggle('acc', !c.pttKey && capKind !== 'ptt');
    $('pttRow').classList.toggle('hide', !c.ptt && capKind !== 'ptt');
    $('micHkName').textContent = hk.mic ? comboKeys(hk.mic).join(' + ') : '';
    $('hkList').innerHTML = HK.map(function (h) {
      var cap = capKind === 'hk' && capName === h[0], keys = hk[h[0]] && hk[h[0]].vk ? comboKeys(hk[h[0]]) : ['—'];
      return '<div class="hk"><div class="grow"><div>' + tx(h[1]) + '</div>' + (err.indexOf(h[0]) >= 0 ? '<div class="sub st-err">' + tx('Зайнято іншою програмою — оберіть інше поєднання') + '</div>' : '') + '</div>' +
        '<button class="keys' + (cap ? ' cap' : '') + '" data-hk="' + h[0] + '" title="' + tx('Змінити') + '">' + (cap ? '<kbd>' + tx('Натисніть…') + '</kbd>' : keys.map(function (k) { return '<kbd>' + esc(k) + '</kbd>'; }).join('') +
        '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 20h4L19 9l-4-4L4 16z"/></svg>') + '</button>' + '</div>';
    }).join('');
    $('hkList').querySelectorAll('[data-hk]').forEach(function (b) { b.onclick = function () { if (capKind === 'hk' && capName === b.dataset.hk) capEnd(); else capStart('hk', b.dataset.hk); }; });
  }

  // ---------- налаштування ----------
  $('swLow').onclick = function () { patch({ lowBattery: !S.cfg.lowBattery }); };
  $('swAuto').onclick = function () { patch({ autostart: !S.cfg.autostart }); };
  $('swFull').onclick = function () { patch({ fullNotify: !S.cfg.fullNotify }); };
  $('swOled').onclick = function () { patch({ customOled: !S.cfg.customOled }); };
  $('swSwitch').onclick = function () { patch({ autoSwitch: !S.cfg.autoSwitch }); };
  $('swTaskbar').onclick = function () { patch({ taskbar: !S.cfg.taskbar }); };
  $('trayShow').querySelectorAll('button').forEach(function (b) { b.onclick = function () { patch({ trayBattery: b.dataset.v === 'battery' }); }; });
  $('swSwitchMic').onclick = function () { patch({ switchMic: !S.cfg.switchMic }); };
  $('swCables').onclick = function () { patch({ cables: S.cfg.cables === false }); };
  $('swOnLost').onclick = function () { patch({ outOnLost: S.cfg.outOnLost === false }); };
  $('outOnNew').querySelectorAll('button').forEach(function (b) { b.onclick = function () { patch({ outOnNew: b.dataset.v }); }; });
  $('swOsd').onclick = function () { patch({ osd: !S.cfg.osd }); };
  $('osdCorner').querySelectorAll('button').forEach(function (b) { b.onclick = function () { patch({ osdCorner: b.dataset.v }); }; });
  $('swMicHk').onclick = function () { patch({ micHotkey: !S.cfg.micHotkey }); };
  $('swSounds').onclick = function () { patch({ sounds: !S.cfg.sounds }); };
  $('vstSafeOff').onclick = function () { patch({ vstSafe: false }); };
  $('swWinCtl').onclick = function () { patch({ winCtl: !S.cfg.winCtl }); };
  $('quit').onclick = function () { api('/api/quit', {}); };
  $('expBtn').onclick = function () { api('/api/settings', { op: 'export' }); };
  $('impBtn').onclick = function () { api('/api/settings', { op: 'import' }); };
  // the settings folder: next to Arcmix.exe or in %APPDATA% (moved at once, the app goes on)
  $('swPortable').onclick = function () {
    var sub = $('portSub');
    api('/api/settings', { op: S && S.portable ? 'appdata' : 'portable' }).then(function (r) { return r.json(); }).then(function (o) {
      if (o.err) { sub.textContent = tx('Не вдалося перенести:') + ' ' + o.err; sub.className = 'sub st-err'; }
    }).catch(function () {});
  };
  $('resetBtn').onclick = function () {
    var b = this;
    if (b.dataset.armed) { api('/api/reset', {}).then(function () { setTimeout(function () { location.reload(); }, 300); }); return; }
    b.dataset.armed = '1'; b.textContent = tx('Точно скинути?'); b.style.background = 'var(--red)'; b.style.color = '#fff';
    setTimeout(function () { delete b.dataset.armed; b.textContent = tx('Скинути'); b.style.background = ''; b.style.color = ''; }, 3000);
  };
  [['modeSeg', 'mode', String], ['wheelSeg', 'wheelMode', String], ['sideSeg', 'sidetone', Number], ['gainSeg', 'micGain', Number], ['lowSeg', 'lowLevel', Number]].forEach(function (d) {
    $(d[0]).querySelectorAll('button').forEach(function (b) { b.onclick = function () { var o = {}; o[d[1]] = d[2](b.dataset.v); patch(o); }; });
  });
  // теми
  var themesEl = $('themes');
  var penSvg = '<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 20h4L19 9l-4-4L4 16z"/></svg>';
  function themeButtons(box, size, onPick) {
    THEMES.forEach(function (t) {
      var b = document.createElement('button'); b.dataset.k = t.k; b.title = tx(t.n); b.setAttribute('aria-label', tx(t.n));
      if (t.badge) b.innerHTML = '<span class="tbadge">' + t.badge + '</span>';
      if (t.k === 'custom') b.innerHTML = '<span class="tedit">' + penSvg + '</span>'; // the editable one stands out
      b.onclick = function () { onPick(t.k); }; box.appendChild(b);
    });
  }
  themeButtons(themesEl, 36, function (k) { patch({ theme: k }); });
  // a small picture of a theme as it looks: window background, a card, game and chat colours, a pressed chip
  function swatch(t, mode) {
    var k = makeTokens(t, mode);
    var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 36 36"><rect width="36" height="36" fill="' + k.s + '"/>' +
      '<rect x="4" y="7" width="28" height="22" rx="5" fill="' + k.s1 + '"/>' +
      '<rect x="7" y="11" width="13" height="5" rx="2.5" fill="' + k.g + '"/><rect x="21.5" y="11" width="7.5" height="5" rx="2.5" fill="' + k.c + '"/>' +
      '<rect x="7" y="19" width="13" height="6" rx="3" fill="' + k.gc + '"/></svg>';
    return 'url("data:image/svg+xml,' + encodeURIComponent(svg) + '") center / cover';
  }
  // each box keeps its own signature: the wizard builds its buttons anew
  function themeMarks(box, cur, cfg) {
    var sig = JSON.stringify([cfg.mode, cfg.customA, cfg.customB, cfg.customOled, (S && S.win) || null, darkMQ && darkMQ.matches]);
    var paint = sig !== box.dataset.sig; box.dataset.sig = sig;
    box.querySelectorAll('button').forEach(function (b) {
      if (paint) {
        var c = { theme: b.dataset.k, mode: cfg.mode, customA: cfg.customA, customB: cfg.customB, customOled: cfg.customOled };
        b.style.background = swatch(themeOf(c), modeOf(c));
      }
      b.style.boxShadow = b.dataset.k === cur ? '0 0 0 2px var(--solid), 0 0 0 4px var(--on)' : 'inset 0 0 0 1px rgba(127,127,127,.25)';
    });
  }
  // власні кольори: відтінок / насиченість / яскравість
  var slot = 'a', hslT = 0;
  var SLOTK = { a: 'customA', b: 'customB', s: 'customS' }, SLOTE = { a: 'slotA', b: 'slotB', s: 'slotS' };
  function slotHex() { return slot === 'a' ? (S.cfg.customA || '#FF6A3D') : slot === 'b' ? (S.cfg.customB || '#3DC8FF') : (S.cfg.customS || '#151618'); }
  function hslInput() {
    paintHSL();
    var hex = r2h(hsl2r(hH.get() / 360, hS.get() / 100, hL.get() / 100));
    $(SLOTE[slot]).style.background = hex;
    $('hexIn').value = hex.toUpperCase();
    clearTimeout(hslT); hslT = setTimeout(function () { var o = {}; o[SLOTK[slot]] = hex; patch(o); }, 90);
  }
  var hH = Slider($('hH'), { min: 0, max: 360, step: 1, cls: 'hue', key: 5, wheel: 4, onInput: hslInput });
  var hS = Slider($('hS'), { min: 0, max: 100, step: 1, cls: 'hue', key: 5, wheel: 2, onInput: hslInput });
  var hL = Slider($('hL'), { min: 3, max: 95, step: 1, cls: 'hue', key: 5, wheel: 2, onInput: hslInput });
  function syncHSL() {
    var h = r2hsl(h2r(slotHex())); hH.set(Math.round(h[0] * 360)); hS.set(Math.round(h[1] * 100)); hL.set(Math.round(h[2] * 100)); paintHSL();
    if (document.activeElement !== $('hexIn')) $('hexIn').value = slotHex().toUpperCase();
  }
  function paintHSL() {
    var H = hH.get(), Sa = hS.get(), Li = hL.get();
    hH.track.style.background = 'linear-gradient(90deg,' + [0, 60, 120, 180, 240, 300, 360].map(function (x) { return 'hsl(' + x + ',' + Sa + '%,' + Li + '%)'; }).join(',') + ')';
    hS.track.style.background = 'linear-gradient(90deg,hsl(' + H + ',0%,' + Li + '%),hsl(' + H + ',100%,' + Li + '%))';
    hL.track.style.background = 'linear-gradient(90deg,hsl(' + H + ',' + Sa + '%,3%),hsl(' + H + ',' + Sa + '%,50%),hsl(' + H + ',' + Sa + '%,95%))';
  }
  document.querySelectorAll('.slot').forEach(function (b) {
    b.onclick = function () { slot = b.dataset.slot; document.querySelectorAll('.slot').forEach(function (x) { x.setAttribute('aria-pressed', x === b); }); syncHSL(); };
  });
  // код кольору: #RRGGBB (з # чи без) — будь-який колір, зокрема відповідник Pantone
  $('hexIn').addEventListener('input', function () {
    var v = this.value.trim().replace(/^#?/, '#');
    if (!/^#[0-9a-fA-F]{6}$/.test(v)) return;
    var o = {}; o[SLOTK[slot]] = v.toUpperCase(); patch(o);
    $(SLOTE[slot]).style.background = v;
  });
  $('hexIn').addEventListener('blur', function () { this.value = slotHex().toUpperCase(); });
  [['cornSeg', 'corners']].forEach(function (x) {
    $(x[0]).querySelectorAll('button').forEach(function (b) { b.onclick = function () { var o = {}; o[x[1]] = b.dataset.v; patch(o); }; });
  });
  $('creditLink').onclick = function () { api('/api/open', { url: 'https://github.com/Swiftlyx/OpenChatMix' }); };
  $('authorLink').onclick = function () { api('/api/open', { url: 'https://github.com/Fenderenko/Arcmix' }); };
  $('updGet').onclick = function () { api('/api/open', { url: 'https://github.com/Fenderenko/Arcmix/releases/latest' }); };
  // update check only on request: the server asks GitHub for the latest release
  $('updCheck').onclick = function () {
    var b = $('updCheck'), st = $('updStatus');
    b.disabled = true; b.textContent = tx('Перевіряю…');
    $('updGet').classList.add('hide');
    api('/api/update', {}).then(function (r) { return r.json(); }).then(function (u) {
      st.textContent = u.newer ? tx('Доступна версія %s').replace('%s', u.latest) : (u.latest ? tx('Встановлено найновішу версію') : tx('Не вдалося перевірити оновлення'));
      $('updGet').classList.toggle('hide', !u.newer);
    }).catch(function () { st.textContent = tx('Не вдалося перевірити оновлення'); }).then(function () {
      st.classList.remove('hide'); b.disabled = false; b.textContent = tx('Перевірити оновлення');
    });
  };
  // мова
  (function () {
    var sel = $('langSel');
    sel.innerHTML = '<option value="">' + esc(tx('Як у Windows')) + '</option>' +
      (I18.langs || []).map(function (l) { return '<option value="' + l.code + '">' + esc(l.name) + '</option>'; }).join('');
    sel.value = I18.pick || '';
    sel.onchange = function () { api('/api/cfg', { lang: sel.value }).then(function () { location.reload(); }); };
  })();
  // налаштування гарнітур, описаних конфігураціями (усі, крім Arctis 7+)
  var genSig = '', genSl = {};
  function devSet(st, v) { api('/api/dev', { api: st.api, field: st.field, value: Number(v) }); }
  function renderGen(list) {
    var box = $('genBox'), sig = JSON.stringify(list.map(function (x) { return [x.api, x.field, x.kind, x.opts]; }));
    if (sig !== genSig) {
      genSig = sig; genSl = {};
      box.innerHTML = list.map(function (st, i) {
        var id = 'gs' + i;
        if (st.kind === 'toggle') return '<div class="row"><div class="grow">' + esc(tx(st.label)) + '</div><button class="sw" role="switch" id="' + id + '"><i></i></button></div>';
        if (st.kind === 'slider') return '<div class="slv"><span>' + esc(tx(st.label)) + '</span><b id="' + id + 'v"></b></div><div id="' + id + '"></div>';
        var many = st.opts.length > 5;
        return '<div class="sub" style="margin:10px 0 6px">' + esc(tx(st.label)) + '</div><div class="' + (many ? 'chips' : 'seg') + '" id="' + id + '">' +
          st.opts.map(function (o) { return '<button class="' + (many ? 'chip' : '') + '" data-v="' + o.v + '">' + esc(tx(o.label)) + '</button>'; }).join('') + '</div>';
      }).join('');
      list.forEach(function (st, i) {
        var el = $('gs' + i);
        if (st.kind === 'toggle') el.onclick = function () { devSet(st, el.getAttribute('aria-checked') === 'true' ? st.min : st.max); };
        else if (st.kind === 'slider') {
          var send = throttle(function (v) { devSet(st, v); }, 200);
          genSl[i] = Slider(el, { min: st.min, max: st.max, step: 1, value: st.min, onInput: function (v) { $('gs' + i + 'v').textContent = v; send(v); }, onChange: function (v) { send(v, true); } });
        } else el.querySelectorAll('button').forEach(function (b) { b.onclick = function () { devSet(st, b.dataset.v); }; });
      });
    }
    list.forEach(function (st, i) {
      var el = $('gs' + i), has = st.value !== null && st.value !== undefined;
      if (!el) return;
      if (st.kind === 'toggle') setSw('gs' + i, has && st.value >= st.max);
      else if (st.kind === 'slider') { if (has) genSl[i].set(st.value); if (!genSl[i].busy()) $('gs' + i + 'v').textContent = has ? st.value : '—'; }
      else el.querySelectorAll('button').forEach(function (b) { b.setAttribute('aria-pressed', has && Number(b.dataset.v) === st.value); });
    });
  }

  // ---------- звіт ----------
  $('repBtn').onclick = function () {
    $('repStatus').textContent = tx('Створюю…');
    api('/api/report', {}).then(function (r) { return r.json(); }).then(function (o) {
      if (o.err) { $('repStatus').textContent = tx('Помилка') + ': ' + o.err; $('repStatus').className = 'sub st-err'; return; }
      $('repStatus').textContent = tx('Архів на робочому столі:') + ' ' + o.path.split(/[\\/]/).pop();
      $('repStatus').className = 'sub st-ok';
    }).catch(function () { $('repStatus').textContent = tx('Помилка'); });
  };

  // ---------- знайомство при першому запуску ----------
  var obStep = -1, OB_LAST = 4, obModSig = '';
  function obRender() {
    var b = $('obBody');
    $('obDots').querySelectorAll('i').forEach(function (d, i) { d.classList.toggle('on', i <= obStep); });
    $('obBack').classList.toggle('hide', obStep === 0);
    $('obNext').textContent = obStep === OB_LAST ? tx('Готово') : tx('Далі');
    if (obStep === 0) {
      var live = (S.apps || []).filter(function (a) { return a.live || a.running; });
      b.innerHTML = '<h1>' + tx('Вітаємо в Arcmix') + '</h1><div class="sub" style="font-size:13px;line-height:1.45">' + tx('Баланс між грою і чатом змінює коліщатко на гарнітурі або повзунок у вікні. Позначте, які програми — це чат; решта вважатиметься грою.') + '</div>' +
        (live.length ? live.map(function (a, i) {
          return '<div class="obr">' + iconHTML(a.exe, a.name) + '<div class="grow">' + esc(a.name) + '</div><div class="seg" style="width:168px;flex:none">' +
            ['game', 'chat', 'media'].map(function (c, k) { return '<button data-i="' + i + '" data-cat="' + c + '" aria-pressed="' + (a.cat === c) + '">' + tx(['Гра', 'Чат', 'Медіа'][k]) + '</button>'; }).join('') + '</div></div>';
        }).join('') : '<div class="note">' + tx('Програм зі звуком поки немає — їх можна розкласти пізніше на головному екрані.') + '</div>');
      b.querySelectorAll('[data-cat]').forEach(function (x) { x.onclick = function () { api('/api/app', { exe: live[+x.dataset.i].exe, cat: x.dataset.cat }); x.parentNode.querySelectorAll('button').forEach(function (y) { y.setAttribute('aria-pressed', y === x); }); }; });
    } else if (obStep === 1) { // how apps get the microphone chosen in Arcmix: a virtual cable + microphone processing
      var mf = S.cfg.micFx || {}, vs = virt();
      b.innerHTML = '<h1>' + tx('Мікрофон') + '</h1><div class="sub" style="font-size:13px;line-height:1.45">' + tx('Arcmix керує гучністю й вимкненням мікрофона, обраного на головному екрані. Discord, гра чи OBS беруть мікрофон, обраний у них самих. Щоб вони чули мікрофон з Arcmix разом із шумодавом, оберіть у них віртуальний кабель і ввімкніть тут обробку мікрофона: вона передає мікрофон у кабель.') + '</div>' +
        (DEVS === null ? '<div class="note">' + tx('Шукаю пристрої…') + '</div>' : vs.length ?
          '<div class="obr"><div class="grow">' + tx('Обробляти мікрофон') + '</div><button class="sw" role="switch" id="obMfx" aria-checked="' + !!mf.on + '" aria-label="' + tx('Обробляти мікрофон') + '"><i></i></button></div><div class="sub" id="obMfxHint" style="font-size:12px;line-height:1.4"></div>' : '');
      if (DEVS === null) { loadDevs(function () { if (obStep === 1) obRender(); }); return; }
      if (!vs.length) { // no cable: the same notice and download button as on the Microphone tab
        var nc = $('mfxNoCable').cloneNode(true);
        nc.removeAttribute('id'); nc.classList.remove('hide');
        nc.querySelector('[data-get-cable]').onclick = function () { api('/api/open', { url: 'https://vb-audio.com/Cable/' }); };
        b.appendChild(nc);
        return;
      }
      var obHint = function (on) {
        var m = S.cfg.micFx || {}, c = on && (m.out ? virt().filter(function (d) { return d.id === m.out; })[0] : freeMicCable());
        $('obMfxHint').textContent = c && c.pair ? tx('Поки обробка ввімкнена, мікрофоном Windows стоїть «%s»: Discord, гра й OBS з мікрофоном за замовчуванням чують оброблений голос.').replace('%s', c.pair) : '';
      };
      obHint(!!mf.on);
      $('obMfx').onclick = function () {
        var on = this.getAttribute('aria-checked') !== 'true';
        this.setAttribute('aria-checked', on);
        if (on !== !!(S.cfg.micFx || {}).on) toggleMfx();
        obHint(on);
      };
    } else if (obStep === 2) { // the equalizer of everything: the Arcmix module on the current output (skipping is "Next")
      var md = S.apoMod || {}, ap = S.apo || {}, o = S.out || {}, dn = o.short || o.name || '';
      obModSig = JSON.stringify([md, ap.dev, o.id, dn, I18.lang]);
      var st;
      if (md.busy) st = '<div class="sub">' + esc(tx(md.busy === 'install' ? 'встановлюю на «%s» — звук на кілька секунд зникне' : 'прибираю з «%s» — звук на кілька секунд зникне').replace('%s', md.busyName || dn)) + '</div>';
      else if (ap.dev) st = '<div class="sub">' + esc(tx('На «%s» уже працює Equalizer APO — модуль не потрібен.').replace('%s', dn)) + '</div>';
      else if (md.inst) st = '<div class="sub">' + esc(tx(md.live ? 'працює на «%s»: EQ Windows без кабелів' : 'встановлено — запрацює, щойно зазвучить звук').replace('%s', dn)) + '</div>';
      else if (md.free) st = '<button class="chip acc" id="obMod">' + tx('Встановити') + '</button>';
      else st = '<div class="sub">' + tx('Шукаю пристрої…') + '</div>';
      b.innerHTML = '<h1>' + tx('Еквалайзер для всього звуку') + '</h1><div class="sub" style="font-size:13px;line-height:1.45">' + tx('Крива «EQ Windows» діє на все, що грає на пристрої, — без кабелів і без затримки. Для цього Arcmix додає свій модуль у звук Windows: Windows попросить дозволу адміністратора, а звук на мить зникне. Прибрати модуль можна будь-коли на вкладці «Звук».') + '</div>' +
        '<div class="obr">' + icon('layers', 16) + '<div class="grow">' + esc(dn || tx('Пристрій виводу Windows')) + '</div>' + st + '</div>';
      var mb = $('obMod'); if (mb) mb.onclick = function () { mb.disabled = true; api('/api/apo', { op: 'modInstall' }); };
    } else if (obStep === 3) {
      b.innerHTML = '<h1>' + tx('Оберіть вигляд') + '</h1><div class="sub" style="font-size:13px">' + tx('Тему можна змінити будь-коли в налаштуваннях.') + '</div><div class="obt" id="obThemes"></div><div class="seg" id="obMode"><button data-v="dark">' + tx('Темна') + '</button><button data-v="light">' + tx('Світла') + '</button><button data-v="auto">' + tx('Як у Windows') + '</button></div>';
      themeButtons($('obThemes'), 36, function (k) { patch({ theme: k }); });
      themeMarks($('obThemes'), S.cfg.theme, S.cfg);
      $('obMode').querySelectorAll('button').forEach(function (x) { x.setAttribute('aria-pressed', x.dataset.v === (S.cfg.mode || 'dark')); x.onclick = function () { patch({ mode: x.dataset.v }); }; });
    } else {
      b.innerHTML = '<h1>' + tx('Майже готово') + '</h1><div class="sub" style="font-size:13px;line-height:1.45">' + tx('Програма живе в треї біля годинника. Клік по значку відкриває це вікно, колесо миші над ним — показує заряд.') + '</div>' +
        '<div class="obr"><div class="grow">' + tx('Запускати разом з Windows') + '</div><button class="sw" role="switch" id="obAuto" aria-checked="' + !!S.cfg.autostart + '"><i></i></button></div>';
      $('obAuto').onclick = function () { var on = this.getAttribute('aria-checked') !== 'true'; this.setAttribute('aria-checked', on); patch({ autostart: on }); };
    }
  }
  $('obNext').onclick = function () { if (obStep < OB_LAST) { obStep++; obRender(); } else { patch({ onboarded: true }); $('ob').classList.add('hide'); obStep = -2; } };
  $('obBack').onclick = function () { if (obStep > 0) { obStep--; obRender(); } };

  // ---------- level meters: streams from the app (only of what is visible), drawn every frame ----------
  var MS = {}, MV = {}, SM = {}, meterRaf = 0;
  // a fast rise and a smooth fall, as level meters do
  function smooth(k, v) { var o = SM[k]; SM[k] = o === undefined || v >= o ? v : o + (v - o) * 0.2; return SM[k]; }
  var DRAW = { mic: micMeterDraw, fx: vuDraw, duck: duckDraw };
  function meterLoop() {
    var any = false;
    for (var k in MS) { any = true; if (MV[k]) DRAW[k](MV[k]); }
    meterRaf = any ? requestAnimationFrame(meterLoop) : 0;
  }
  function meterStream(k, on) {
    if (on && !MS[k]) {
      MS[k] = new EventSource('/api/meters?k=' + TOK + '&m=' + k);
      MS[k].onmessage = function (e) { try { MV[k] = JSON.parse(e.data); } catch (x) { } };
      if (!meterRaf) meterRaf = requestAnimationFrame(meterLoop);
    }
    if (!on && MS[k]) { MS[k].close(); delete MS[k]; delete MV[k]; }
  }
  var tMon = 0;
  function pollers() {
    var vis = !document.hidden;
    var wantMic = vis && tab === 'mix', wantFx = vis && tab === 'micTab', wantDuck = vis && tab === 'sound';
    var m = S && S.monitor, wantMon = vis && m && (m.mode === 'record' || m.mode === 'play');
    if (!wantMic && MS.mic) $('micLvl').style.width = '0';
    if (!wantFx && MS.fx) { $('vuIn').style.width = '0'; $('vuOut').style.width = '0'; }
    meterStream('mic', wantMic); meterStream('fx', wantFx); meterStream('duck', wantDuck);
    if (wantMon && !tMon) tMon = setInterval(monTick, 150);
    if (!wantMon && tMon) { clearInterval(tMon); tMon = 0; }
  }
  document.addEventListener('visibilitychange', pollers);
  // сторож вікна: після кожного показу повідомляємо програмі, що сторінка справді намалювалась
  var aliveBusy = false;
  function aliveSoon() {
    if (aliveBusy || document.hidden) return;
    aliveBusy = true;
    requestAnimationFrame(function () {
      requestAnimationFrame(function () {
        aliveBusy = false;
        var d = devicePixelRatio || 1;
        api('/api/alive', { w: innerWidth * d, h: innerHeight * d });
      });
    });
  }
  document.addEventListener('visibilitychange', aliveSoon);

  // ---------- стан ----------
  function render(s) {
    var first = !S; S = s;
    var c = s.cfg;
    applyTheme(c);
    var cur = themeOf(c);
    setTtl('themeName', tx('Тема') + ': ' + tx(cur.n));
    themeMarks(themesEl, c.theme, c);
    $('customBox').classList.toggle('hide', c.theme !== 'custom');
    var isWin = c.theme === 'win', fixedMode = isWin || c.theme === 'black';
    $('winNote').classList.toggle('hide', !isWin); $('oledNote').classList.toggle('hide', c.theme !== 'black');
    $('modeLbl').classList.toggle('hide', fixedMode); $('modeSeg').classList.toggle('hide', fixedMode);
    $('oledRow').classList.toggle('hide', modeOf(c) !== 'dark'); setSw('swOled', !!c.customOled);
    $('slotA').style.background = c.customA || '#FF6A3D'; $('slotB').style.background = c.customB || '#3DC8FF'; $('slotS').style.background = c.customS || '#151618';
    // the look: corners
    document.documentElement.dataset.corn = c.corners || 'round';
    pressed('cornSeg', c.corners || 'round');
    if (c.theme === 'custom' && !(hH.busy() || hS.busy() || hL.busy())) syncHSL();
    // the tray icon: what it shows, and the wheel over it flips that for a while (or moves the balance)
    pressed('trayShow', c.trayBattery ? 'battery' : 'icon');
    pressed('wheelSeg', c.wheelMode || 'battery');
    $('wheelSeg').querySelector('[data-v=battery]').textContent = tx(c.trayBattery ? 'Іконка' : 'Заряд');
    $('wheelHint').textContent = { battery: tx(c.trayBattery ? 'Прокрутка над іконкою ненадовго показує іконку замість заряду.' : 'Прокрутка над іконкою ненадовго показує заряд.'), balance: tx('Прокрутка над іконкою змінює баланс гра / чат, як повзунок у вікні.'), off: '' }[c.wheelMode || 'battery'];
    pressed('modeSeg', c.mode === 'light' || c.mode === 'auto' ? c.mode : 'dark'); pressed('sideSeg', c.sidetone); $('offDropT').textContent = offLabel(c.autoOff); pressed('gainSeg', c.micGain); setSw('swLed', c.muteLed !== 0);
    setSw('swLow', c.lowBattery); setSw('swAuto', c.autostart); setSw('swFull', c.fullNotify); setSw('swPortable', !!s.portable);
    pressed('lowSeg', c.lowLevel || 20); $('lowSeg').classList.toggle('hide', !c.lowBattery);
    $('battEst').textContent = s.battHours > 0 && !s.charging ? tx('Прогноз роботи від батареї:') + ' ≈ ' + s.battHours + ' ' + tx('год') : tx('Прогноз роботи з’явиться, коли заряд почне знижуватися.');
    setSw('swSwitch', c.autoSwitch); setSw('swSwitchMic', c.switchMic); $('switchMicRow').classList.toggle('hide', !c.autoSwitch);
    $('switchOpts').classList.toggle('hide', !c.autoSwitch); setSw('swCables', c.cables !== false); pressed('outOnNew', c.outOnNew || 'order'); setSw('swOnLost', c.outOnLost !== false);
    setSw('swTaskbar', c.taskbar);
    OUTS = s.outs || []; outPopRender();
    setSw('swOsd', c.osd); pressed('osdCorner', c.osdCorner || 'br'); $('osdCornerRow').classList.toggle('hide', !c.osd); setSw('swMicHk', c.micHotkey); setSw('swPtt', c.ptt); setSw('swSounds', c.sounds); setSw('swWinCtl', c.winCtl);
    renderKeys();
    var pinT = tx(c.keepOpen ? 'Відкріпити вікно' : 'Закріпити вікно');
    $('pinBtn').setAttribute('aria-pressed', !!c.keepOpen); $('pinBtn').title = pinT; $('pinBtn').setAttribute('aria-label', pinT);
    renderProfiles(c.profiles || []);
    renderDuck(s);
    renderFeatures(c);
    renderMon(s.monitor);
    $('ver').textContent = tx('Версія') + ' ' + s.version;
    $('grpAbout').querySelector('.sub').textContent = tx('Версія') + ' ' + s.version;
    var dev = s.device || {};
    renderOut(s);
    setTtl('devTitle', dev.name || tx('Гарнітура'));
    renderHW(dev);
    var xb = s.batteries || {}, names = { left: tx('Л'), right: tx('П'), 'case': tx('Кейс'), dock: tx('База') };
    $('xbatt').textContent = Object.keys(xb).map(function (k) { return (names[k] || k) + ' ' + xb[k] + '%'; }).join(' · ');
    $('hwMute').classList.toggle('hide', s.micMuted !== true);
    // categories that really play through a cable: the microphone's cable is not routed
    var rc0 = c.route || {}, mfc = c.micFx || {}, rcfg = {};
    ['game', 'chat', 'media'].forEach(function (k) { rcfg[k] = rc0[k] && !(mfc.on && rc0[k] === mfc.out) ? rc0[k] : ''; });
    // the curve of everything: through the cables, or through Equalizer APO on the output device
    // on a headset with its own equalizer Equalizer APO works only if asked (apoHs)
    var ap = s.apo || {}, apoAvail = !!(ap.ready && ap.dev && !ap.sysfxOff) || !!(s.apoMod && s.apoMod.inst && !ap.dev);
    var rtSW = !!(s.route && s.route.running && (rcfg.game || rcfg.chat || rcfg.media));
    var canSW = rtSW || apoAvail;
    rcfg.all = canSW ? 'all' : '';
    // the headset's own equalizer only on a headset tested with Arcmix; any other gets the software one.
    // A parametric one comes only when a tester turned it on (untestedEq; otherwise the app does not
    // report it at all)
    var hwEq = dev.eq && (dev.native !== false || dev.eq.kind === 'parametric') ? dev.eq : null;
    $('eqTarget').classList.toggle('hide', !canSW);
    if (canSW) {
      $('eqTarget').querySelector('[data-t=hw]').classList.toggle('hide', !hwEq);
      $('eqTarget').querySelector('[data-t=hw]').textContent = 'EQ' + (dev.name ? ' ' + dev.name : '');
      ['game', 'chat', 'media'].forEach(function (k) { $('eqTarget').querySelector('[data-t=' + k + ']').classList.toggle('hide', !rcfg[k]); });
    }
    if (eqT !== 'hw' && (!canSW || !rcfg[eqT])) setTarget('hw');
    // no headset equalizer: the software one of everything
    if (eqT === 'hw' && !hwEq && canSW) setTarget('all');
    // the headset is off or gone: what plays is the curve of everything, so it comes into focus (once
    // per change: the headset tab can still be picked by hand)
    var hsRunning = !!(s.connected && s.powered);
    if (hsRunning !== eqHsRunning && !hsRunning && eqT === 'hw' && canSW) setTarget('all');
    eqHsRunning = hsRunning;
    if (eqT === 'hw' && setEQI(hwEq)) { eqB = zeros(); eqDraw(); }
    // the card is always there: without an equalizer to tune it says how to get the software one
    EQV = { hwEq: !!hwEq, canSW: canSW, rtSW: rtSW, apoAvail: apoAvail };
    $('eqCard').classList.remove('hide');
    eqView();
    $('eqNoteRt').classList.toggle('hide', canSW || !feat('route'));
    renderAudioFx(s);
    renderCableWarn(s);
    $('nativeBox').classList.toggle('hide', dev.native === false);
    // users of other headsets do not see settings of a headset Arcmix cannot talk to
    $('devCard').classList.toggle('hide', !s.connected);
    // the headset's settings page: named after it, only while it is connected
    $('grpHs').classList.toggle('hide', !s.connected);
    $('grpHs').querySelector('.grow div').textContent = (s.device || {}).name || tx('Гарнітура');
    if (!s.connected && PG.settings === 'hs') setPage('settings', '');
    $('battCard').classList.toggle('hide', !(s.connected && s.haveBattery));
    $('devNone').classList.toggle('hide', !!s.connected);
    renderGen(dev.native === false ? (dev.settings || []) : []);
    var bo = s.out || {}, bt = !bo.headset && bo.batt >= 0;
    var pct = bt ? bo.batt : s.battery, lvl = pct / 25;
    // the colours of the headset LED (as icon::battery_color): older Arctis — red below 20 %, yellow below 50 %;
    // Nova — red up to 5 %, orange up to 14 %, yellow below 50 %
    var nova = !bt && /nova/i.test(dev.name || '');
    var bcol = (nova ? pct <= 5 : pct < 20) ? 'var(--red)' : nova && pct <= 14 ? 'var(--orange)' : pct < 50 ? 'var(--warn)' : 'var(--g)';
    document.querySelectorAll('#bchip .bc').forEach(function (b, i) { b.style.fill = i < lvl ? bcol : 'var(--tr)'; });
    $('bpct').textContent = bt ? bo.batt + '%' : s.connected && s.haveBattery ? s.battery + '%' : '—';
    // the hours left (or charging) after the charge
    var hrs = !bt && s.connected && s.haveBattery ? (s.charging ? '⚡ ' + tx('заряджається') : (s.battHours > 0 ? '≈' + (s.battHours >= 10 ? Math.round(s.battHours) : s.battHours) + ' ' + tx('год') : '')) : '';
    $('heroSub').textContent = hrs; $('heroSub').classList.toggle('hide', !hrs);
    if (!dragging) { pos = s.pos; drawPos(pos, !first); }
    var m = s.mic || {};
    $('micCard').style.opacity = m.ok ? 1 : .55;
    $('micName').textContent = c.micDev || c.winCtl ? (m.ok ? m.name : tx('мікрофон не знайдено')) : (m.ok ? m.name : tx('Як у Windows'));
    // what the others hear: processed into a cable, or the microphone as is
    var mfm = c.micFx || {}, cab = (virt().filter(function (d) { return d.id === mfm.out; })[0] || {}).name;
    var dn = mfm.denoise && feat('denoise') ? (mfm.denoiseKind === 'nv' && (s.nv || {}).state === 'ready' ? tx('Шумодав') + ' NVIDIA' : tx('Шумодав') + ' RNNoise') : tx('з обробкою');
    $('micSub').textContent = mfm.on && mfm.out ? dn + (cab ? ' · ' + cab : '') : tx('без обробки');
    renderBalKeys(c);
    $('micDev').title = m.ok ? m.name : '';
    drawMic(m);
    drawOut(s.out || {});
    $('outCard').classList.toggle('hide', !c.winCtl);
    if (eqDrag < 0 && Date.now() - eqLocal > 1500) { eqLoad(s); eqDraw(); }
    renderPresets(); eqDraw();
    renderApps(s.apps || []);
    aliveSoon();
    if (!c.onboarded && obStep === -1) { obStep = 0; $('ob').classList.remove('hide'); obRender(); }
    else if (obStep === 2 && JSON.stringify([s.apoMod || {}, (s.apo || {}).dev, (s.out || {}).id, (s.out || {}).short || (s.out || {}).name || '', I18.lang]) !== obModSig) obRender(); // the module step follows the install
    else if (obStep === 3) { themeMarks($('obThemes'), c.theme, c); $('obMode').querySelectorAll('button').forEach(function (x) { x.setAttribute('aria-pressed', x.dataset.v === (c.mode || 'dark')); }); }
    pollers();
  }

  setTarget('hw');
  eqDraw();
  api('/api/ping?v=' + innerWidth + 'x' + innerHeight);
  api('/api/state').then(function (r) { return r.json(); }).then(function (st) { render(st); loadDevs(); });
  var es = new EventSource('/api/events?k=' + TOK);
  es.onmessage = function (e) { try { render(JSON.parse(e.data)); } catch (x) { } };
})();
