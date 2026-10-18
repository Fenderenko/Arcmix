#!/usr/bin/env python3
"""Перевіряє, що кожен український текст інтерфейсу має переклад усіма мовами з web/i18n.json.
Використання: python3 tools/check_i18n.py [--keys]   (--keys — лише вивести знайдені ключі)"""
import json, re, sys, os
from html.parser import HTMLParser

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CYR = re.compile('[Ѐ-ӿ]')
WORD = re.compile(r'^([^A-Za-zÀ-ɏЀ-ӿ]*)([\s\S]*?)([^A-Za-zÀ-ɏЀ-ӿ.)»]*)$')

def html_keys():
    src = open(os.path.join(ROOT, 'web/index.html'), encoding='utf-8').read()
    keys = set()
    body, script = src.split('<script>', 1)
    class P(HTMLParser):
        skip = False
        def handle_endtag(self, tag):
            if tag == 'style':
                self.skip = False
        def handle_data(self, d):
            if self.skip:
                return
            m = WORD.match(d)
            if m and CYR.search(m.group(2)):
                keys.add(m.group(2))
        def handle_starttag(self, tag, attrs):
            if tag == 'style':
                self.skip = True
            for k, v in attrs:
                if k in ('aria-label', 'title') and v and CYR.search(v):
                    keys.add(v)
    P(convert_charrefs=True).feed(re.sub(r'<!--.*?-->', '', body, flags=re.S))
    code = re.sub(r'//[^\n]*', '', script)
    for m in re.finditer(r"tx\((?:\[([^\]]*)\]|'((?:[^'\\]|\\.)*)')", code):
        if m.group(1):
            keys.update(re.findall(r"'([^']*)'", m.group(1)))
        else:
            keys.add(m.group(2))
    for m in re.finditer(r"n: '([^']*)'", code):
        keys.add(m.group(1))
    # усі інші рядки з кирилицею в коді сторінки (напр. таблиці повідомлень) теж мають переклад
    for m in re.finditer(r"'((?:[^'\\\n]|\\.)*)'", code):
        if CYR.search(m.group(1)):
            keys.add(m.group(1))
    for m in re.finditer(r"\['\w+', '([^']*)', \[", code):
        keys.add(m.group(1))
    return {k for k in keys if CYR.search(k)}

def rust_keys():
    # Cyrillic string literals of rust/src outside comments and test modules; format strings
    # ("{} хв") are composed at run time and skipped
    keys = set()
    for dp, _, fs in os.walk(os.path.join(ROOT, 'rust', 'src')):
        for f in fs:
            if not f.endswith('.rs'):
                continue
            src = open(os.path.join(dp, f), encoding='utf-8').read()
            src = src.split('#[cfg(test)]', 1)[0]
            src = re.sub(r'//[^\n]*', '', src)
            for m in re.finditer(r'"((?:[^"\\\n]|\\.)*)"', src):
                v = m.group(1)
                if CYR.search(v) and '{' not in v:
                    keys.add(v.replace('\\n', '\n'))
    return keys

def main():
    data = json.load(open(os.path.join(ROOT, 'web/i18n.json'), encoding='utf-8'))
    langs = [l['code'] for l in data['langs'] if l['code'] != 'uk']
    keys = html_keys() | rust_keys()
    keys.discard('хв')  # додається в переклади окремо
    if '--keys' in sys.argv:
        for k in sorted(keys):
            print(k)
        return
    bad = 0
    for k in sorted(keys | {'хв'}):
        m = data['strings'].get(k)
        miss = [l for l in langs if not (m or {}).get(l)]
        if miss:
            bad += 1
            print('немає перекладу:', repr(k), miss)
    unused = sorted(set(data['strings']) - keys - {'хв'})
    for k in unused:
        print('зайвий ключ:', repr(k))
    print(f'ключів: {len(keys)}, мов: {len(langs) + 1}, проблем: {bad}')
    sys.exit(1 if bad else 0)

main()
