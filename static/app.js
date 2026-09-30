// OpenLina front end: plain JS on top of the JSON API. Mod data comes from uploads, so it is only
// ever inserted as text (see h()), never as HTML.
'use strict';

const SECTIONS = {
  items: { label: 'ITEMS', color: '#e2c35a', hint: 'e.g. 5 shots instead of 3', icon: 'portal' },
  modifiers: { label: 'MODIFIERS', color: '#e8883a', hint: 'e.g. only wrap top and bottom', icon: 'wrap' },
  levels: { label: 'LEVEL MODS', color: '#419ecd', hint: 'e.g. turn every 15 s', icon: 'drum' },
  general: { label: 'GENERAL', color: '#b58fb8', hint: 'e.g. open with F1', icon: 'menu' },
  core: { label: 'CORE', color: '#a8a8a8', hint: '', icon: 'menu' },
};
const ORDER = ['items', 'modifiers', 'levels', 'general'];

// Pixel icons for the section tabs (12x12, one char per pixel).
const PAL = { k: '#0d0d0d', w: '#ffffff', g: '#a8a8a8', d: '#5e6175', b: '#419ecd', o: '#e8883a', m: '#79617b', r: '#b0506a', y: '#e2c35a' };
const ICONS = {
  portal: ['.....oo.....', '....o..o....', '....o..o....', '.....oo.....', '............', '.kkkkkkkkk..', '.kgggggggbk.', '.kgwwwgggbbk', '.kkkkdgkkbk.', '....kdk..k..', '....kdk.....', '....kkk.....'],
  wrap: ['.....oo.....', '.....oo.....', '...oooooo...', '....oooo....', '.....oo.....', '.gggggggggg.', '.g........g.', '.gggggggggg.', '.....oo.....', '...oooooo...', '....oooo....', '.....oo.....'],
  drum: ['....rr..y...', '..rr.....y..', '.r.......yy.', '.r..........', 'r...........', 'r....bb....r', 'r....bb....r', 'r..........r', '.r........r.', '.r........r.', '..rr....rr..', '....rrrr....'],
  menu: ['............', '.kkkkkkkkkk.', '.kwwwwwwwwk.', '.kkkkkkkkkk.', '.kmm.mmmmmk.', '.kkkkkkkkkk.', '.kmm.mmmmk..', '.kkkkkkkkkk.', '.kmm.mmmmmk.', '.kkkkkkkkkk.', '............', '............'],
};

// ------------------------------------------------------------------ helpers

function h(tag, attrs, ...kids) {
  const el = document.createElementNS(tag === 'svg' || tag === 'rect' ? 'http://www.w3.org/2000/svg' : 'http://www.w3.org/1999/xhtml', tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === null || v === undefined || v === false) continue;
    if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'style' && typeof v === 'object') for (const [p, pv] of Object.entries(v)) { if (p.startsWith('--')) el.style.setProperty(p, pv); else el.style[p] = pv; }
    else if (k === 'value') el.value = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

// Replace an element's children (arrays are flattened, null/false skipped, like h()).
function fill(el, ...kids) {
  el.replaceChildren();
  for (const kid of kids.flat(Infinity)) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

function pixelIcon(name, size) {
  const rows = ICONS[name] || ICONS.menu;
  const rects = [];
  rows.forEach((row, y) => [...row].forEach((c, x) => { if (PAL[c]) rects.push(h('rect', { x, y, width: 1, height: 1, fill: PAL[c] })); }));
  return h('svg', { width: size, height: size, viewBox: '0 0 12 12', 'shape-rendering': 'crispEdges', 'aria-hidden': 'true' }, rects);
}

function modIcon(m, cls) {
  const color = (SECTIONS[m.section] || SECTIONS.core).color;
  if (m.icon) return h('img', { class: 'icon px ' + (cls || ''), src: m.icon, alt: '', style: { '--c': color } });
  const box = h('span', { class: 'icon ' + (cls || ''), style: { '--c': color } });
  box.append(pixelIcon((SECTIONS[m.section] || SECTIONS.core).icon, '100%'));
  return box;
}

async function api(path, opts = {}) {
  const res = await fetch(path, { ...opts, headers: { 'Content-Type': 'application/json', ...(opts.headers || {}) } });
  const body = res.headers.get('content-type')?.includes('json') ? await res.json() : null;
  if (!res.ok) throw new Error((body && body.error) || `${res.status} ${res.statusText}`);
  return body;
}

const store = {
  get(key, def) { try { return JSON.parse(localStorage.getItem('openlina.' + key)) ?? def; } catch { return def; } },
  set(key, v) { try { localStorage.setItem('openlina.' + key, JSON.stringify(v)); } catch { /* private mode */ } },
};

// The pack being assembled: { mods: { id: { name, section, icon, request } }, sections: { items: '…' } }
const cart = {
  data: store.get('cart', { mods: {}, sections: {} }),
  save() { store.set('cart', this.data); updateCount(); },
  has(id) { return !!this.data.mods[id]; },
  toggle(m) {
    if (this.has(m.id)) delete this.data.mods[m.id];
    else this.data.mods[m.id] = { name: m.name, section: m.section, icon: m.icon, request: '' };
    this.save();
  },
  setRequest(id, text) { if (this.data.mods[id]) { this.data.mods[id].request = text; store.set('cart', this.data); } },
  setSection(s, text) { this.data.sections[s] = text; store.set('cart', this.data); },
  ids() { return Object.keys(this.data.mods); },
};

function updateCount() {
  const n = cart.ids().length;
  const el = document.getElementById('pack-count');
  if (el) el.textContent = n;
}

function statsChips(stats) {
  return Object.entries(stats || {}).map(([k, v]) => h('span', { class: 'chip' }, `${k.replace(/_/g, ' ')} ${Array.isArray(v) ? v.join(', ') : v}`.trim().toUpperCase()));
}

function statusBadge(status) {
  if (status === 'reviewed') return null;
  return h('span', { class: 'badge status-' + status }, status.toUpperCase());
}

function voteBox(m, onChange, big) {
  const color = (SECTIONS[m.section] || SECTIONS.core).color;
  const box = h('div', { class: 'votes' + (big ? ' bigvote' : ''), style: { '--c': color } });
  const render = () => {
    fill(box, 
      h('button', { type: 'button', class: 'up' + (m.my_vote === 1 ? ' on' : ''), 'aria-label': `Vote ${m.name} up`, 'aria-pressed': String(m.my_vote === 1), onclick: () => cast(1) }, '▲'),
      h('span', { 'aria-label': 'score' }, m.score),
      h('button', { type: 'button', class: 'down' + (m.my_vote === -1 ? ' on' : ''), 'aria-label': `Vote ${m.name} down`, 'aria-pressed': String(m.my_vote === -1), onclick: () => cast(-1) }, '▼'),
    );
  };
  const cast = async (dir) => {
    const value = m.my_vote === dir ? 0 : dir;
    try {
      const r = await api(`/api/mods/${encodeURIComponent(m.id)}/vote`, { method: 'POST', body: JSON.stringify({ value }) });
      m.score = r.score; m.my_vote = r.my_vote;
      render();
      if (onChange) onChange();
    } catch (e) { alert('Vote failed: ' + e.message); }
  };
  render();
  return box;
}

function addButton(m, cls, labels, onChange) {
  const color = (SECTIONS[m.section] || SECTIONS.core).color;
  const b = h('button', { type: 'button', class: 'add ' + (cls || ''), style: { '--c': color } });
  const render = () => {
    const on = cart.has(m.id);
    b.className = 'add ' + (cls || '') + (on ? ' on' : '');
    b.textContent = on ? labels[1] : labels[0];
    b.setAttribute('aria-pressed', String(on));
  };
  b.addEventListener('click', () => { cart.toggle(m); render(); if (onChange) onChange(); });
  render();
  return b;
}

function fmtSize(n) { return n > 1 << 20 ? (n / (1 << 20)).toFixed(1) + ' MB' : Math.max(1, Math.round(n / 1024)) + ' KB'; }
function fmtDate(t) { return new Date(t * 1000).toISOString().slice(0, 10); }
function setFooter(t) { document.getElementById('footer-label').textContent = t; }

// ------------------------------------------------------------------ browse

async function browse(app) {
  let section = ORDER.includes(location.hash.slice(1)) ? location.hash.slice(1) : store.get('section', 'items');
  let sort = 'top', q = '', data = null, timer = null;

  const hero = h('section', { class: 'hero' },
    h('div', {},
      h('h1', { 'aria-label': 'OpenLina' }, [['O', -5, 4], ['P', 4, 0], ['E', -3, 6], ['N', 6, -2], [' ', 0, 0], ['L', -4, 3], ['I', 3, -4], ['N', -6, 5], ['A', 5, 0]]
        .map(([c, r, y]) => c === ' ' ? h('span', { style: { width: '22px' } }) : h('span', { style: { transform: `rotate(${r}deg) translateY(${y}px)` }, 'aria-hidden': 'true' }, c))),
      h('p', {}, 'Mods for Mosa Lina, made by humans and their agents. Pick mods, add change requests, export a pack, then run ', h('code', {}, 'openlina install pack.zip'), '.')),
    h('div', { class: 'build' }, h('span', { class: 'muted' }, 'WORKS WITH STEAM BUILD'), h('b', { id: 'build' }, '…')));
  const tabs = h('nav', { class: 'tabs', 'aria-label': 'Sections' });
  const search = h('input', { type: 'search', id: 'search', placeholder: 'Search', oninput: (e) => { q = e.target.value; clearTimeout(timer); timer = setTimeout(load, 200); } });
  const sortBtns = h('div', { class: 'seg', role: 'group', 'aria-label': 'Sort' });
  const cards = h('div', { class: 'cards' });
  const aside = h('aside', { class: 'cart', 'aria-label': 'Your pack' });
  fill(app, hero, tabs, h('div', { class: 'browse' },
    h('div', {}, h('div', { class: 'tools' }, h('label', { for: 'search' }, 'SEARCH'), search, sortBtns), cards), aside));

  function renderTabs() {
    fill(tabs, ...ORDER.map((key) => {
      const s = SECTIONS[key];
      return h('button', { type: 'button', class: 'tab' + (key === section ? ' on' : ''), style: { '--c': s.color }, 'aria-pressed': String(key === section),
        onclick: () => { section = key; store.set('section', key); history.replaceState(null, '', '#' + key); load(); } },
        pixelIcon(s.icon, 44), h('span', {}, h('b', {}, s.label), h('small', {}, `${data ? data.counts[key] ?? 0 : '…'} mods`)));
    }));
    fill(sortBtns, ...[['top', 'TOP'], ['new', 'NEW']].map(([k, l]) =>
      h('button', { type: 'button', class: 'btn' + (sort === k ? ' on' : ''), 'aria-pressed': String(sort === k), onclick: () => { sort = k; load(); } }, l)));
    search.placeholder = 'Search ' + SECTIONS[section].label.toLowerCase();
    setFooter(SECTIONS[section].label);
  }

  function card(m) {
    const s = SECTIONS[m.section] || SECTIONS.core;
    const link = '/mods/' + encodeURIComponent(m.id);
    const thumb = h('a', { class: 'thumb', href: link, 'aria-label': 'Open ' + m.name },
      m.gifs.length ? h('img', { src: m.gifs[0], alt: `${m.name} in action`, loading: 'lazy', class: 'px' }) : h('span', { class: 'noimg' }, pixelIcon(s.icon, 72)),
      m.gifs.length ? h('span', { class: 'tag' }, 'GIF · ' + m.gifs.length) : null,
      m.status !== 'reviewed' ? h('span', { class: 'tag right' }, m.status.toUpperCase()) : null);
    return h('article', { class: 'card', style: { '--c': s.color } }, thumb,
      h('div', { class: 'body' },
        h('div', { class: 'row' }, modIcon(m), h('div', { style: { minWidth: 0 } }, h('h2', {}, h('a', { href: link }, m.name)), h('span', { class: 'small muted' }, 'by ' + (m.authors.join(', ') || m.uploaded_by)))),
        h('p', {}, m.description),
        h('div', { class: 'chips' }, statsChips(m.stats)),
        h('div', { class: 'foot' }, voteBox(m), addButton(m, '', ['ADD', 'ADDED'], renderCart))));
  }

  async function load() {
    renderTabs();
    try {
      data = await api(`/api/mods?section=${section}&sort=${sort}&q=${encodeURIComponent(q)}`);
    } catch (e) {
      fill(cards, h('p', { class: 'err' }, 'Could not load mods: ' + e.message));
      return;
    }
    document.getElementById('build').textContent = data.game_build;
    renderTabs();
    fill(cards, ...data.mods.map(card),
      data.mods.length === 0 ? h('p', { class: 'muted' }, q ? 'Nothing matches.' : 'No mods in this section yet.') : null,
      h('article', { class: 'made' }, pixelIcon('menu', 60), h('h2', {}, 'MADE A MOD?'),
        h('p', {}, 'Your agent can upload it for you with ', h('code', {}, 'lina publish'), '. New uploads show as unreviewed until checked.'),
        h('a', { href: '/agents', style: { fontFamily: 'var(--pixel)', fontSize: '18px' } }, 'HOW IT WORKS')));
  }

  function renderCart() {
    const ids = cart.ids();
    fill(aside, h('h2', {}, 'YOUR PACK'),
      ids.length === 0 ? h('p', { class: 'small muted' }, 'Nothing yet. Add mods from any section.') : null,
      ids.map((id) => {
        const it = cart.data.mods[id];
        const s = SECTIONS[it.section] || SECTIONS.core;
        return h('div', { class: 'cart-item' },
          h('div', { class: 'row' }, modIcon({ ...it, id }, 'sm'), h('div', {}, h('div', { class: 'name' }, it.name), h('div', { class: 'small muted' }, s.label)),
            h('button', { type: 'button', class: 'x', 'aria-label': 'Remove ' + it.name, onclick: () => { cart.toggle({ id }); renderCart(); load(); } }, '×')),
          h('label', { class: 'req' }, 'CHANGE REQUEST (OPTIONAL)',
            h('textarea', { class: 'input', rows: 2, placeholder: s.hint, maxlength: 2000, oninput: (e) => cart.setRequest(id, e.target.value) }, it.request || '')));
      }),
      h('a', { class: 'export-link', href: '/pack' }, 'EXPORT PACK'),
      h('p', { class: 'small muted', style: { margin: 0, lineHeight: 1.6 } }, 'ZIP for players (the openlina helper patches your own copy of the game), JSON for agents.'));
  }

  renderCart();
  await load();
}

// ------------------------------------------------------------------ mod page

async function modPage(app) {
  const id = decodeURIComponent(location.pathname.split('/')[2] || '');
  let m;
  try { m = await api('/api/mods/' + encodeURIComponent(id)); } catch (e) {
    fill(app, h('p', { class: 'err' }, e.message), h('a', { href: '/' }, 'Back to all mods'));
    return;
  }
  const s = SECTIONS[m.section] || SECTIONS.core;
  document.title = `OpenLina: ${m.name}`;
  setFooter(m.name.toUpperCase());
  let current = 0;
  const stage = h('div', { class: 'stage' });
  const thumbs = h('div', { class: 'thumbs', role: 'tablist', 'aria-label': 'Showcase gifs' });
  const gifName = (u) => decodeURIComponent(u.split('/').pop()).replace(/\.gif$/i, '').replace(/[-_]/g, ' ').toUpperCase();
  function renderStage() {
    fill(stage, m.gifs.length
      ? h('img', { src: m.gifs[current], alt: `${m.name}: ${gifName(m.gifs[current])}` })
      : h('div', { class: 'noimg' }, 'No showcase gif yet'),
    m.gifs.length ? h('span', { class: 'tag' }, `GIF ${current + 1} / ${m.gifs.length} · ${gifName(m.gifs[current])}`) : null);
    fill(thumbs, ...(m.gifs.length > 1 ? m.gifs.map((g, i) => h('button', { type: 'button', role: 'tab', class: i === current ? 'on' : '', style: { '--c': s.color }, 'aria-selected': String(i === current), onclick: () => { current = i; renderStage(); } },
      h('img', { src: g, alt: '', loading: 'lazy' }), gifName(g))) : []));
  }
  renderStage();

  const stats = [...Object.entries(m.stats || {}).map(([k, v]) => [k.replace(/_/g, ' '), Array.isArray(v) ? v.join(', ') : String(v)])];
  stats.push(['Version', m.version]);
  if (m.game_builds.length) stats.push(['Game build', m.game_builds.join(', ')]);
  if (m.requires.length) stats.push(['Requires', m.requires.join(', ')]);
  if (m.conflicts.length) stats.push(['Conflicts with', m.conflicts.join(', ')]);

  const request = h('textarea', { class: 'input', rows: 3, maxlength: 2000, placeholder: s.hint || 'What should an agent change?', oninput: (e) => cart.setRequest(m.id, e.target.value) }, cart.data.mods[m.id]?.request || '');
  const reqWrap = h('label', { class: 'req', style: { fontFamily: 'var(--pixel)', fontSize: '18px' } }, 'REQUEST A CHANGE', request,
    h('span', { class: 'small muted', style: { fontFamily: 'var(--mono)' } }, 'Goes into your pack. Your agent applies it when it installs the pack.'));
  const syncReq = () => { request.disabled = !cart.has(m.id); request.placeholder = cart.has(m.id) ? (s.hint || '') : 'Add the mod to your pack first'; };
  syncReq();

  fill(app, 
    h('nav', { class: 'crumbs', 'aria-label': 'Breadcrumb' }, h('a', { href: '/', style: { color: 'var(--grey)' } }, 'OPENLINA'), h('span', { class: 'sep' }, '/'),
      h('a', { href: '/#' + m.section, style: { color: s.color } }, s.label), h('span', { class: 'sep' }, '/'), h('span', {}, m.name.toUpperCase())),
    h('div', { class: 'detail' },
      h('section', { 'aria-label': 'Showcase' }, stage, thumbs,
        h('p', { class: 'small muted', style: { lineHeight: 1.6 } }, 'Showcase gifs are recorded by the game itself from a scripted run (', h('code', {}, 'lina gif'), '), so they show what the mod really does.')),
      h('aside', { class: 'stack', style: { '--c': s.color } },
        h('div', { class: 'title-block' }, modIcon(m, 'lg'),
          h('div', { class: 'stack', style: { gap: '6px' } }, h('h1', {}, m.name.toUpperCase()),
            h('span', { class: 'small muted' }, `by ${m.authors.join(', ') || m.uploaded_by} · v${m.version}`),
            h('div', { class: 'row', style: { gap: '6px' } }, h('span', { class: 'badge fill', style: { background: s.color } }, s.label.replace(' MODS', '')), statusBadge(m.status)))),
        h('p', { style: { margin: 0, lineHeight: 1.6, color: '#e6e6e6' } }, m.description),
        h('table', { class: 'kv' }, h('caption', {}, 'STATS'), h('tbody', {}, stats.map(([k, v]) => h('tr', {}, h('th', { scope: 'row' }, k), h('td', {}, v.toUpperCase()))))),
        h('div', { class: 'row' }, voteBox(m, null, true), addButton(m, 'wide', ['ADD TO PACK', 'IN YOUR PACK'], syncReq)),
        reqWrap,
        m.options.length ? h('table', { class: 'kv' }, h('caption', {}, 'OPTIONS'), h('tbody', {}, m.options.map((o) =>
          h('tr', {}, h('th', { scope: 'row' }, h('code', {}, o.key), h('div', { class: 'small' }, o.description)), h('td', { style: { whiteSpace: 'nowrap' } }, JSON.stringify(o.default)))))) : null,
        h('div', { class: 'pkg' }, h('b', {}, 'PACKAGE'),
          h('span', {}, 'id ', h('code', {}, `${m.id}@${m.version}`), ` · ${fmtSize(m.size)}`),
          h('span', { class: 'muted' }, 'sha256 ', h('code', {}, m.sha256.slice(0, 16) + '…')),
          h('a', { href: m.package }, 'Download package'),
          h('span', { class: 'muted' }, 'Versions: ', m.versions.map((v) => `${v.version} (${v.status}, ${fmtDate(v.created)})`).join(' · '))))));
}

// ------------------------------------------------------------------ pack page

async function packPage(app) {
  setFooter('EXPORT');
  let format = store.get('format', 'zip');
  const ids = cart.ids();
  const groups = h('section', { class: 'stack', 'aria-label': 'Pack contents' },
    h('h1', { class: 'shadow-title', style: { fontSize: 'clamp(40px, 5vw, 56px)' } }, 'YOUR PACK'),
    h('p', { class: 'muted', style: { margin: 0, lineHeight: 1.6 } }, 'Change requests are optional. They travel with the export; an agent reads them and adapts the mods before installing.'));
  if (ids.length === 0) groups.append(h('div', { class: 'panel' }, h('p', { style: { margin: 0 } }, 'Your pack is empty. ', h('a', { href: '/' }, 'Browse mods'), ' and add some.')));
  for (const sec of Object.keys(SECTIONS)) {
    const mine = ids.filter((id) => cart.data.mods[id].section === sec);
    if (!mine.length) continue;
    const s = SECTIONS[sec];
    groups.append(h('div', { class: 'panel group', style: { '--c': s.color } }, h('h2', {}, s.label),
      mine.map((id) => {
        const it = cart.data.mods[id];
        return h('div', { class: 'pack-mod' }, modIcon({ ...it, id }),
          h('div', {}, h('div', { class: 'name' }, it.name), h('code', { class: 'small muted' }, id)),
          h('label', { class: 'req' }, `CHANGE REQUEST FOR ${it.name.toUpperCase()}`,
            h('textarea', { class: 'input', rows: 2, maxlength: 2000, placeholder: s.hint, oninput: (e) => cart.setRequest(id, e.target.value) }, it.request || '')),
          h('button', { type: 'button', class: 'x', 'aria-label': 'Remove ' + it.name, onclick: () => { cart.toggle({ id }); packPage(app); } }, '×'));
      }),
      ORDER.includes(sec) ? h('label', { class: 'req', style: { borderTop: '2px solid #1f1f1f', paddingTop: '12px' } }, `CHANGE REQUEST FOR ALL ${s.label}`,
        h('textarea', { class: 'input', rows: 2, maxlength: 2000, placeholder: 'e.g. make all of them a little stronger', oninput: (e) => cart.setSection(sec, e.target.value) }, cart.data.sections[sec] || '')) : null));
  }

  const side = h('aside', { class: 'stack', 'aria-label': 'Export' });
  const result = h('div', { class: 'stack' });
  function renderSide() {
    fill(side, 
      h('div', { class: 'fmt', role: 'group', 'aria-label': 'Format' },
        h('button', { type: 'button', class: format === 'zip' ? 'on' : '', 'aria-pressed': String(format === 'zip'), onclick: () => { format = 'zip'; store.set('format', format); renderSide(); } }, 'ZIP · PLAYERS'),
        h('button', { type: 'button', class: format === 'json' ? 'on' : '', 'aria-pressed': String(format === 'json'), onclick: () => { format = 'json'; store.set('format', format); renderSide(); } }, 'JSON · AGENTS')),
      format === 'zip'
        ? h('div', { class: 'stack' },
          h('div', { class: 'panel stack', style: { gap: '12px' } }, h('span', { style: { fontFamily: 'var(--pixel)', fontSize: '20px', color: 'var(--grey)' } }, 'INSIDE THE ZIP'),
            h('pre', { class: 'code', style: { padding: 0 } }, ['openlina-pack/', '  openlina            helper (when the site provides it)', '  modpack.toml        mods, options, requests', '  mods/', '    core/             added automatically', ...ids.map((id) => `    ${(id + '/').padEnd(18)}mod.toml, patch.wasm, assets/`)].join('\n')),
            h('span', { class: 'small muted' }, 'No game files inside. The helper patches your own copy of Mosa Lina and never changes the install.')),
          h('ol', { class: 'steps' }, h('li', {}, 'Unzip anywhere, then run ', h('code', {}, './openlina install .')),
            h('li', {}, 'Steam › Mosa Lina › Properties › Launch options: paste the line it prints.'),
            h('li', {}, 'Play. Clear the launch option to go back to vanilla.')))
        : h('p', { class: 'muted', style: { margin: 0, lineHeight: 1.6, fontSize: '13px' } }, 'Give the JSON or its link to your agent: ', h('code', { style: { color: '#fff' } }, 'lina pull <pack link>'), ' downloads the mods, applies the requests, tests and installs.'),
      h('button', { type: 'button', class: 'btn primary big', disabled: ids.length === 0, onclick: exportPack }, format === 'zip' ? 'DOWNLOAD ZIP' : 'CREATE JSON'),
      result);
  }

  async function exportPack() {
    fill(result, h('p', { class: 'muted' }, 'Creating the pack…'));
    const body = {
      mods: cart.ids().map((id) => ({ id, request: cart.data.mods[id].request || null })),
      section_requests: Object.fromEntries(Object.entries(cart.data.sections).filter(([k, v]) => ORDER.includes(k) && v && v.trim() && cart.ids().some((id) => cart.data.mods[id].section === k))),
    };
    let pack;
    try { pack = await api('/api/packs', { method: 'POST', body: JSON.stringify(body) }); } catch (e) {
      fill(result, h('p', { class: 'err' }, e.message));
      return;
    }
    const added = pack.mods.filter((m) => m.required_by && m.required_by.length);
    const note = added.length ? h('p', { class: 'small muted', style: { margin: 0 } }, 'Added because other mods need them: ' + added.map((m) => m.id).join(', ')) : null;
    const conflicts = pack.conflicts || [];
    const warn = conflicts.length ? h('div', { class: 'panel stack', style: { gap: '8px', borderColor: 'var(--yellow)' } },
      h('strong', { style: { color: 'var(--yellow)' } }, 'NEEDS AN AGENT'),
      h('p', { class: 'small', style: { margin: 0, lineHeight: 1.6 } }, conflicts.map(([a, b]) => `${a} conflicts with ${b}`).join(', ') + '. The pack is saved, but players can\'t install it as is. Give its link to your agent: ', h('code', {}, 'lina pull ' + pack.url), ' lists the conflict as a task, and the agent changes the mods so they work together.')) : null;
    if (warn) {
      // A zip wouldn't install: show the link for the agent instead.
      const b = h('button', { type: 'button', class: 'btn primary', onclick: (e) => navigator.clipboard.writeText(pack.url).then(() => { e.target.textContent = 'COPIED'; }, () => { e.target.textContent = 'COPY FAILED'; }) }, 'COPY LINK');
      fill(result, warn, h('div', { class: 'row' }, b), h('code', { class: 'small' }, pack.url), note);
      return;
    }
    if (format === 'zip') {
      location.href = pack.zip_url;
      fill(result, h('p', { style: { margin: 0 } }, 'Download started. ', h('a', { href: pack.zip_url }, 'Link'), ' · pack ', h('code', {}, pack.id)), note);
    } else {
      const json = JSON.stringify(pack, null, 2);
      const copy = (text, btn) => navigator.clipboard.writeText(text).then(() => { btn.textContent = 'COPIED'; }, () => { btn.textContent = 'COPY FAILED'; });
      const b1 = h('button', { type: 'button', class: 'btn primary', onclick: (e) => copy(json, e.target) }, 'COPY JSON');
      const b2 = h('button', { type: 'button', class: 'btn', style: { borderColor: 'var(--yellow)', color: 'var(--yellow)' }, onclick: (e) => copy(pack.url, e.target) }, 'COPY LINK');
      fill(result, h('pre', { class: 'code', style: { maxHeight: '420px' } }, json), h('div', { class: 'row' }, b1, b2), h('code', { class: 'small' }, pack.url), note);
    }
  }

  renderSide();
  fill(app, h('div', { class: 'pack' }, groups, side));
}

// ------------------------------------------------------------------ agents page

async function agentsPage(app) {
  setFooter('FOR AGENTS');
  const origin = location.origin;
  const endpoints = [
    ['GET', '/api/mods?section=items', 'list mods (q, sort=top|new)'],
    ['GET', '/api/mods/{id}', 'details, options, versions'],
    ['GET', '/api/mods/{id}/{ver}/package', 'download a package'],
    ['POST', '/api/packs', 'create a pack from ids + change requests'],
    ['GET', '/api/packs/{pack}', 'a pack as JSON'],
    ['GET', '/api/packs/{pack}/zip', 'a pack as a zip for players'],
    ['POST', '/api/mods', 'upload a package zip (token)'],
    ['POST', '/api/mods/{id}/vote', 'vote {"value": 1 | -1 | 0}'],
    ['GET', '/api/me', 'your uploads (token)'],
  ];
  const tokenInput = h('input', { class: 'input', type: 'password', placeholder: 'olt_…', autocomplete: 'off', value: store.get('token', '') });
  const uploads = h('div', {});
  async function loadMe() {
    const token = tokenInput.value.trim();
    store.set('token', token);
    if (!token) { fill(uploads, h('p', { class: 'small muted' }, 'Paste your token to see your uploads. It stays in this browser.')); return; }
    try {
      const me = await api('/api/me', { headers: { Authorization: 'Bearer ' + token } });
      fill(uploads, h('p', { class: 'small' }, `Signed in as ${me.name}${me.admin ? ' (admin, ' : ''}`, me.admin ? h('a', { href: '/review' }, 'review queue') : null, me.admin ? ')' : ''),
        me.uploads.length ? me.uploads.map((u) => h('div', { class: 'upload-row' }, h('a', { class: 'name', href: '/mods/' + encodeURIComponent(u.id), style: { color: '#fff', textDecoration: 'none' } }, u.name), h('span', { class: 'small muted' }, u.version), h('span', { class: 'badge status-' + u.status }, u.status.toUpperCase())))
          : h('p', { class: 'small muted' }, 'No uploads yet.'),
        h('button', { type: 'button', class: 'btn', style: { borderColor: 'var(--modifiers)', color: 'var(--modifiers)', alignSelf: 'flex-start' }, onclick: rotate }, 'NEW TOKEN'));
    } catch (e) { fill(uploads, h('p', { class: 'err' }, e.message)); }
  }
  async function rotate() {
    if (!confirm('Replace your token? The old one stops working, also for your agent.')) return;
    try {
      const r = await api('/api/me/token', { method: 'POST', headers: { Authorization: 'Bearer ' + tokenInput.value.trim() } });
      tokenInput.value = r.token;
      store.set('token', r.token);
      prompt('Your new token (shown once; give it to your agent with `lina login`):', r.token);
      loadMe();
    } catch (e) { alert(e.message); }
  }
  fill(app, 
    h('section', { class: 'stack', style: { marginBottom: '32px' } }, h('h1', { class: 'shadow-title', style: { fontSize: 'clamp(40px, 5vw, 56px)' } }, 'FOR AGENTS'),
      h('p', { class: 'muted', style: { margin: 0, maxWidth: '900px', lineHeight: 1.7 } }, 'Agents build mods with ', h('strong', { style: { color: '#fff' } }, 'openlina-kit'), ': a skill, docs and the ', h('code', { style: { color: '#fff' } }, 'lina'), ' CLI. They pull packs from here, apply your change requests, test in the real game, record showcase gifs, and upload after asking you.')),
    h('div', { class: 'agents' },
      h('section', { class: 'panel', style: { '--c': 'var(--items)' } }, h('h2', {}, '1 · GET THE KIT'),
        h('pre', { class: 'code', style: { padding: 0 } }, 'git clone …/openlina-kit\ncd openlina-kit\nnix develop          # rust, wasm target, gif tools\ncargo build --release\nlina setup           # reads your local game'),
        h('p', { class: 'small muted', style: { margin: 0 } }, 'Claude Code picks up the ', h('code', { style: { color: '#fff' } }, 'openlina-modding'), ' skill from the repo.')),
      h('section', { class: 'panel', style: { '--c': 'var(--modifiers)' } }, h('h2', {}, '2 · UPLOAD TOKEN'),
        h('p', { class: 'small muted', style: { margin: 0, lineHeight: 1.6 } }, 'Your agent uploads in your name with a token from the site maintainer. It asks you before every upload. You can replace the token any time.'),
        h('div', { class: 'row' }, tokenInput, h('button', { type: 'button', class: 'btn', onclick: loadMe }, 'USE')),
        h('pre', { class: 'code', style: { padding: 0 } }, `lina login ${origin} olt_…`)),
      h('section', { class: 'panel', style: { '--c': 'var(--levels)' } }, h('h2', {}, '3 · API'),
        h('table', { class: 'api' }, h('tbody', {}, endpoints.map(([m, p, w]) => h('tr', {}, h('td', {}, m), h('td', {}, h('code', {}, p)), h('td', {}, w))))),
        h('p', { class: 'small muted', style: { margin: 0 } }, 'OpenAPI spec at ', h('a', { href: '/api/openapi.json' }, '/api/openapi.json'), '. Voting is one vote per mod per network (salted IP hash, no raw IPs stored).')),
      h('section', { class: 'panel', style: { '--c': 'var(--general)' } }, h('h2', {}, '4 · YOUR UPLOADS'), uploads,
        h('p', { class: 'small muted', style: { margin: 0, lineHeight: 1.6 } }, 'Mods are code that runs in your game. New uploads stay marked UNREVIEWED until a maintainer checks them.'))));
  loadMe();
}

// ------------------------------------------------------------------ review page (admins)

async function reviewPage(app) {
  setFooter('REVIEW');
  const token = store.get('token', '');
  const auth = { Authorization: 'Bearer ' + token };
  let q;
  try { q = await api('/api/review', { headers: auth }); } catch (e) {
    fill(app, h('p', { class: 'err' }, e.message), h('p', {}, 'Set an admin token on the ', h('a', { href: '/agents' }, 'agents page'), ' first.'));
    return;
  }
  const set = async (v, status) => {
    try { await api(`/api/mods/${encodeURIComponent(v.id)}/${encodeURIComponent(v.version)}/review`, { method: 'POST', headers: auth, body: JSON.stringify({ status }) }); reviewPage(app); } catch (e) { alert(e.message); }
  };
  fill(app, h('h1', { class: 'shadow-title', style: { fontSize: '48px', marginBottom: '24px' } }, 'REVIEW QUEUE'),
    q.pending.length === 0 ? h('p', { class: 'muted' }, 'Nothing to review.') : null,
    h('div', { class: 'stack' }, q.pending.map((v) => h('div', { class: 'panel stack', style: { gap: '10px' } },
      h('div', { class: 'row' }, h('a', { href: '/mods/' + encodeURIComponent(v.id), style: { fontFamily: 'var(--pixel)', fontSize: '24px' } }, `${v.id} ${v.version}`),
        h('span', { class: 'small muted' }, `by ${v.uploaded_by} · ${fmtDate(v.created)} · ${fmtSize(v.size)}`)),
      h('pre', { class: 'code', style: { maxHeight: '260px' } }, v.manifest),
      h('div', { class: 'row' }, h('a', { href: v.package }, 'Download package'), h('code', { class: 'small muted' }, 'sha256 ' + v.sha256),
        h('button', { type: 'button', class: 'btn primary', onclick: () => set(v, 'reviewed') }, 'APPROVE'),
        h('button', { type: 'button', class: 'btn', style: { color: 'var(--bad)', borderColor: 'var(--bad)' }, onclick: () => set(v, 'rejected') }, 'REJECT'))))));
}

// ------------------------------------------------------------------ start

const PAGES = { browse, mod: modPage, pack: packPage, agents: agentsPage, review: reviewPage };
document.addEventListener('DOMContentLoaded', () => {
  const page = document.body.dataset.page;
  document.querySelector(`[data-nav="${page}"]`)?.classList.add('on');
  updateCount();
  (PAGES[page] || browse)(document.getElementById('app')).catch((e) => {
    fill(document.getElementById('app'), h('p', { class: 'err' }, 'Something went wrong: ' + e.message));
  });
});
