import {
  type CycleInfo, type BountyInfo, type CircuitInfo,
  type BaroInfo, type ArbitrationInfo, type ArbitrationSlot,
  S, fmtRemain, rarityCls, safeBaroId,
  CIRCUIT_CYCLE,
} from '../state';

export const BOUNTY_COLS = 4;

// Bounty board card, shown under the cycle cards when a 赏金 card is clicked.
// Branded orange header + rotation A/B/C selector + per-bounty blocks
// (number · title · 等级) each over a clean 4-column zebra reward grid, items
// colored by rarity (hover shows drop chance).
// Panel header shows the syndicate's proper name, while `syndicate` itself stays
// the open-world location (must match the cycle-card name for click-to-open).
export const SYNDICATE_DISPLAY: Record<string, string> = {
  '夜灵平野': '希图斯',
  '奥布山谷': '索拉里斯联盟',
  '扎里曼': '坚守者',
  '魔胎之境': '英择谛',
  '霍瓦尼亚': '六人组',
};

// Per-trader structural signatures to avoid DOM rebuild on every tick.
export const baroSigs = new Map<string, string>();

export let arbSig = '';

export function fmtHourRange(startMs: number, endMs: number): string {
  const fmt = (ms: number) => {
    const d = new Date(ms);
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };
  return `${fmt(startMs)}–${fmt(endMs)}`;
}

export function arbSlotHtml(slot: ArbitrationSlot, label: string): string {
  const fc = slot.faction.toLowerCase().replace(/[^a-z]/g, '');
  const tags = [slot.mission];
  if (slot.archwing) tags.push('Archwing');
  return `
    <div class="arb-slot">
      <span class="arb-slot-label">${label}</span>
      <span class="arb-node">${slot.node}</span>
      <span class="arb-planet">${slot.planet}</span>
      <span class="arb-mission">${tags.join('·')}</span>
      <span class="arb-faction faction-${fc}">${slot.faction}</span>
      <span class="arb-level">Lv ${slot.min_level}–${slot.max_level}</span>
    </div>`;
}

export function renderCycles(cycles: CycleInfo[]) {
  const container = document.getElementById('cycle-cards')!;
  const withBounty = new Set((S.currentData?.bounties ?? []).map(b => b.card));
  const hasCircuit = !!S.currentData?.circuit;
  container.innerHTML = cycles.map(c => {
    const isCircuit = c.name === CIRCUIT_CYCLE && hasCircuit;
    const clickable = withBounty.has(c.name) || isCircuit;
    const isOpen = isCircuit ? S.openCircuit : S.openBounty === c.name;
    const tag = isCircuit ? '回廊' : '赏金';
    return `
    <div class="cycle-card ${c.is_day ? 'day' : 'night'}${clickable ? ' clickable' : ''}${isOpen ? ' open' : ''}" data-cycle="${c.name}">
      <div class="card-name">${c.name}${clickable ? ` <span class="bounty-tag">${tag}</span>` : ''}</div>
      <div class="card-state">${c.state}</div>
      <div class="card-time">剩余 ${fmtRemain(c.remain_ms)}</div>
    </div>`;
  }).join('');
}

export function renderBountyPanel(bounties: BountyInfo[]) {
  const panel = document.getElementById('bounty-panel')!;
  // One card can host several boards (解剖圣所 hangs under 魔胎之境). Render a
  // section per BountyInfo whose `card` matches the open card.
  const list = S.openBounty ? bounties.filter(x => x.card === S.openBounty) : [];
  if (!list.length) {
    panel.classList.add('hidden');
    panel.innerHTML = '';
    return;
  }
  panel.classList.remove('hidden');

  const section = (b: BountyInfo, withClose: boolean) => {
    // 魔胎之境(Entrati) bounties reward Mother Tokens, not syndicate standing.
    const standingLabel = b.syndicate === '魔胎之境' ? '母亲石印' : '声望';
    // Single-rotation zones (e.g. 扎里曼/六人组/解剖圣所) have one fixed pool — no A/B/C selector.
    const singleRot = b.jobs.length > 0 && b.jobs.every(j => j.rotations.length <= 1);
    // Only the live active rotation is viewable; the other two are locked.
    const active = b.active_rotation || 'A';
    const blocks = b.jobs.map((j, i) => {
      const rot = j.rotations.find(r => r.label === active) ?? j.rotations[0];
      const items = rot?.items ?? [];
      let grid: string;
      if (items.length) {
        const rows: string[] = [];
        for (let r = 0; r < items.length; r += BOUNTY_COLS) {
          const cells = items.slice(r, r + BOUNTY_COLS).map(x =>
            `<div class="rw-cell ${rarityCls(x.rarity)}" title="${x.name} · ${x.chance}%">${x.name}</div>`);
          while (cells.length < BOUNTY_COLS) cells.push('<div class="rw-cell"></div>');
          rows.push(`<div class="rw-row${(r / BOUNTY_COLS) % 2 ? ' alt' : ''}">${cells.join('')}</div>`);
        }
        grid = rows.join('');
      } else {
        grid = '<div class="rw-row"><div class="rw-cell rw-empty">（暂无奖励数据）</div></div>';
      }
      return `
        <div class="bounty-block">
          <div class="bb-head">
            <span class="bb-num">${i + 1}</span>
            <span class="bb-title">${j.title}</span>
            <span class="bb-lv">等级 ${j.min_level}–${j.max_level}</span>
            ${j.standing > 0 ? `<span class="bb-standing">${standingLabel} ${j.standing}</span>` : ''}
          </div>
          <div class="rw-grid">${grid}</div>
        </div>`;
    }).join('');
    const rotBtns = singleRot
      ? '<span class="rot-single">单一奖励池</span>'
      : ['A', 'B', 'C'].map(r =>
          r === active
            ? `<button class="rot-btn on">轮次 ${r}（当前）</button>`
            : `<button class="rot-btn locked" disabled>轮次 ${r} 🔒</button>`).join('');
    return `
      <div class="bounty-section">
        <div class="bounty-card-head">
          <span class="bch-flame">🜂</span>
          <span class="bch-name">${SYNDICATE_DISPLAY[b.syndicate] ?? b.syndicate}</span>
          <span class="bch-count">${b.remain_str}</span>
          ${withClose ? '<button class="bounty-close" id="bounty-close">✕</button>' : ''}
        </div>
        <div class="bounty-toolbar">
          <div class="rot-tabs">${rotBtns}</div>
          <span class="rot-legend"><i class="rk-common">●</i>常见 <i class="rk-uncommon">●</i>罕见 <i class="rk-rare">●</i>稀有</span>
        </div>
        <div class="bounty-blocks">${blocks}</div>
        <div class="bounty-foot">${singleRot
          ? '该地点赏金为单一奖励池，不分轮次；稀有奖励仅在赏金后段阶段出现，最后阶段只出罕见+稀有。'
          : '三个奖励池每次刷新轮换（A/B/C），同一时间仅一个生效；稀有奖励仅在赏金后段阶段出现，最后阶段只出罕见+稀有。'}</div>
      </div>`;
  };

  panel.innerHTML = list.map((b, i) => section(b, i === 0)).join('');
  document.getElementById('bounty-close')!.addEventListener('click', () => {
    S.openBounty = null;
    renderBountyPanel(bounties);
    renderCycles(S.currentData?.cycles ?? []);
  });
}

// Duviri Circuit (无限回廊) panel — this week's selectable 战甲 (普通回廊) and
// Incarnon 武器 (钢铁之路回廊), with the weekly (Monday) refresh countdown.
// Shown under the cycle cards when the 双衍王境 card is clicked.
export function renderCircuitPanel(circuit: CircuitInfo | null) {
  const panel = document.getElementById('circuit-panel')!;
  if (!circuit || !S.openCircuit) {
    panel.classList.add('hidden');
    panel.innerHTML = '';
    return;
  }
  panel.classList.remove('hidden');
  const chips = (arr: string[]) => arr.length
    ? arr.map(n => `<span class="circuit-chip">${n}</span>`).join('')
    : '<span class="rw-empty">（暂无数据）</span>';
  panel.innerHTML = `
    <div class="bounty-card-head circuit-head">
      <span class="bch-flame">🌀</span>
      <span class="bch-name">无限回廊 · 本周奖励</span>
      <span class="bch-count">${circuit.remain_str}后刷新</span>
      <button class="bounty-close" id="circuit-close">✕</button>
    </div>
    <div class="circuit-section">
      <div class="circuit-label">普通回廊 · 战甲奖励</div>
      <div class="circuit-chips">${chips(circuit.normal)}</div>
    </div>
    <div class="circuit-section">
      <div class="circuit-label">钢铁回廊 · 灵化之源</div>
      <div class="circuit-chips">${chips(circuit.hard)}</div>
    </div>`;
  document.getElementById('circuit-close')!.addEventListener('click', () => {
    S.openCircuit = false;
    renderCircuitPanel(circuit);
    renderCycles(S.currentData?.cycles ?? []);
  });
}

export function renderBaro(baroList: BaroInfo[]) {
  const container = document.getElementById('baro-card')!;
  if (!baroList.length) { container.innerHTML = ''; baroSigs.clear(); return; }

  // Check whether any card's structure changed — if so, full rebuild.
  let needsRebuild = baroList.length !== baroSigs.size;
  if (!needsRebuild) {
    for (const baro of baroList) {
      const isOpen = S.openBaro === baro.location;
      const sig = `${baro.active}|${baro.location}|${baro.items.length}|${isOpen}`;
      if (sig !== baroSigs.get(baro.location)) { needsRebuild = true; break; }
    }
  }

  if (needsRebuild) {
    baroSigs.clear();
    let html = '';
    for (const baro of baroList) {
      const cdLabel = baro.active ? '离开倒计时' : '到达倒计时';
      const isOpen = S.openBaro === baro.location;
      const sig = `${baro.active}|${baro.location}|${baro.items.length}|${isOpen}`;
      baroSigs.set(baro.location, sig);
      const badge = baro.tag ? ` <span class="baro-badge">${baro.tag}</span>` : '';
      const cdId = safeBaroId(baro.location);

      if (baro.active) {
        const rows = baro.items.map(it => `
          <tr>
            <td>${it.name}</td>
            <td class="baro-ducats">${it.ducats > 0 ? it.ducats : '—'}</td>
            <td class="baro-credits">${it.credits.toLocaleString()}</td>
          </tr>
        `).join('');
        const tableHtml = isOpen ? `
          <div class="baro-table-wrap">
            <table class="baro-table">
              <thead><tr><th>物品</th><th>杜卡德</th><th>现金</th></tr></thead>
              <tbody>${rows}</tbody>
            </table>
          </div>` : '';
        html += `
        <div class="baro-panel active clickable${isOpen ? ' open' : ''}" data-baro-loc="${baro.location}">
          <div class="baro-head">
            <span class="baro-title">🛒 虚空商人 Baro Ki'Teer${badge}</span>
            <span class="baro-loc">${baro.location}</span>
            <span class="baro-countdown" id="baro-cd-${cdId}">${cdLabel} ${baro.remain_str}</span>
          </div>
          ${tableHtml}
        </div>`;
      } else {
        html += `
        <div class="baro-panel waiting" data-baro-loc="${baro.location}">
          <div class="baro-head">
            <span class="baro-title">🛒 虚空商人 Baro Ki'Teer${badge}</span>
            <span class="baro-loc">${baro.location}</span>
            <span class="baro-countdown" id="baro-cd-${cdId}">${cdLabel} ${baro.remain_str}</span>
          </div>
          <div class="baro-wait-note">尚未到达，到达后可点击展开货物清单</div>
        </div>`;
      }
    }
    container.innerHTML = html;
  }

  // Always patch countdown text in-place (no DOM rebuild — fast, no flicker).
  for (const baro of baroList) {
    const cdLabel = baro.active ? '离开倒计时' : '到达倒计时';
    const cd = document.getElementById(`baro-cd-${safeBaroId(baro.location)}`);
    if (cd) cd.textContent = `${cdLabel} ${baro.remain_str}`;
  }
}

export function renderArbitration(arb: ArbitrationInfo | null) {
  const el = document.getElementById('arbitration-card')!;
  if (!arb) { el.innerHTML = ''; arbSig = ''; return; }

  const HOUR = 3600 * 1000;
  const expiry = arb.expiry_ms;
  const currentLabel = fmtHourRange(expiry - HOUR, expiry);

  // Structural signature: rebuild card only when node or remain_str changes
  // (or when the card doesn't exist yet). Expand/collapse is handled below
  // without innerHTML so the transition doesn't flash.
  const structSig = arb.current.node;
  const card = el.querySelector('.arb-card') as HTMLElement | null;

  if (!card || structSig !== (el.dataset.arbStructSig || '')) {
    el.dataset.arbStructSig = structSig;
    el.innerHTML = `
      <div class="arb-card clickable${S.openArbitration ? ' open' : ''}">
        <div class="arb-head">
          <span class="arb-title">⚔ 仲裁</span>
          <span class="arb-countdown">${arb.remain_str}</span>
        </div>
        ${arbSlotHtml(arb.current, currentLabel)}
      </div>`;
    // If the panel was open before the rebuild, re-create upcoming section
    if (S.openArbitration) {
      const upcoming = document.createElement('div');
      upcoming.className = 'arb-upcoming';
      upcoming.innerHTML = arb.upcoming.map((s, i) =>
        arbSlotHtml(s, fmtHourRange(expiry + i * HOUR, expiry + (i + 1) * HOUR))
      ).join('');
      el.querySelector('.arb-card')!.appendChild(upcoming);
    }
    arbSig = `${structSig}|${S.openArbitration}`;
    return;
  }

  // Fast path: patch countdown text only (no DOM rebuild).
  const cd = el.querySelector('.arb-countdown')!;
  cd.textContent = arb.remain_str;

  // Toggle upcoming section without touching the card head.
  const existing = el.querySelector('.arb-upcoming');
  if (S.openArbitration && !existing) {
    const upcoming = document.createElement('div');
    upcoming.className = 'arb-upcoming';
    upcoming.innerHTML = arb.upcoming.map((s, i) =>
      arbSlotHtml(s, fmtHourRange(expiry + i * HOUR, expiry + (i + 1) * HOUR))
    ).join('');
    card!.appendChild(upcoming);
    card!.classList.add('open');
    arbSig = `${structSig}|true`;
  } else if (!S.openArbitration && existing) {
    existing.remove();
    card!.classList.remove('open');
    arbSig = `${structSig}|false`;
  }
}
