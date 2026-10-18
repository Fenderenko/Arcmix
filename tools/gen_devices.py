#!/usr/bin/env python3
"""Перетворює YAML-конфігурації пристроїв Linux-Arctis-Manager на один JSON
(з уже розгорнутим `extends:`) для вбудовування в програму.
Використання: gen_devices.py <тека device-configs> <вихідний json>"""
import json, os, sys, yaml

src, out = sys.argv[1], sys.argv[2]

def load(name, seen=()):
    if name in seen:
        raise SystemExit('cycle: ' + name)
    with open(os.path.join(src, name + '.yaml'), encoding='utf-8') as f:
        raw = yaml.safe_load(f) or {}
    base_name = raw.pop('extends', None)
    if not base_name:
        return raw
    base = load(base_name, seen + (name,))
    for k in ('constants', 'structs', 'apis', 'transforms', 'sync_events'):
        if k in raw:
            m = dict(base.get(k) or {})
            m.update(raw[k] or {})
            base[k] = m
    if 'sync_read' in raw:
        base['sync_read'] = raw['sync_read']
    if 'lifecycle' in raw:
        lc = dict(base.get('lifecycle') or {})
        for h in ('init', 'post_init', 'shutdown'):
            if h in (raw['lifecycle'] or {}):
                lc[h] = raw['lifecycle'][h]
        base['lifecycle'] = lc
    for k in ('device', 'representation'):
        if k in raw:
            base[k] = raw[k]
    return base

def fix(o):
    # JSON-ключі завжди рядки; ключі sync_events (байти команд) лишаємо десятковими
    if isinstance(o, dict):
        return {str(k): fix(v) for k, v in o.items()}
    if isinstance(o, list):
        return [fix(v) for v in o]
    return o

devs = []
for fn in sorted(os.listdir(src)):
    if not fn.endswith('.yaml') or fn.startswith('base_'):
        continue
    cfg = load(fn[:-5])
    if not cfg.get('device'):
        continue
    cfg['file'] = fn
    devs.append(fix(cfg))
with open(out, 'w', encoding='utf-8') as f:
    json.dump(devs, f, ensure_ascii=False, separators=(',', ':'))
print(len(devs), 'devices,', sum(len(d['device'].get('variants') or []) for d in devs), 'variants')
