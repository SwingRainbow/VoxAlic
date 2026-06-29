import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';

interface FissureAlert {
  tier: string;         // "" = any
  mission_type: string; // "" = any
  difficulty: string;   // "normal"|"hard"|"storm"|"" = any
}

interface CycleAlert {
  location: string;
  state: string;
}

interface ArbitrationAlert {
  mission_type: string;  // "" = any
  planet: string;        // "" = any
}

interface AppConfig {
  close_to_tray: boolean;
  fissure_alerts: FissureAlert[];
  cycle_alerts: CycleAlert[];
  arbitration_alerts: ArbitrationAlert[];
  mission_timer: {
    mode: string;
    ocr_interval_secs: number;
    checkpoint_auto_focus: boolean;
    hp_alert_enabled: boolean;
    alert_method: string;
    checkpoint_alert_text: string;
    hp_alert_text: string;
    selected_hwnd: number;
    window_title: string;
    strip_frame: boolean;
    normal_roi: { x: number; y: number; w: number; h: number };
    fissure_roi: { x: number; y: number; w: number; h: number };
    life_support_roi: { x: number; y: number; w: number; h: number };
    fissure_hp_roi: { x: number; y: number; w: number; h: number };
  };
}

// ── Type definitions matching Rust AppStatePayload ──
interface Fissure {
  node_key: string;
  node_name: string;
  planet: string;
  mission_type: string;
  tier_key: string;
  tier_label: string;
  expiry_ms: number;
  is_hard: boolean;
  is_storm: boolean;
  remain_ms: number;
  remain_str: string;
  is_expiring: boolean;
}

interface CycleInfo {
  name: string;
  state: string;
  state_icon: string;
  remain_ms: number;
  is_day: boolean;
}

interface MissionTimerPayload {
  elapsed_secs: number;
  elapsed_str: string;
  state: string;
  mode: string;
  life_support_pct: number;
  life_support_level: string;
  status_text: string;
  detection_rate: number;
  ocr_raw?: string;
  window_status?: string;
}

interface BaroItem {
  name: string;
  ducats: number;
  credits: number;
}

interface BaroInfo {
  active: boolean;
  location: string;
  start_ms: number;
  end_ms: number;
  remain_ms: number;
  remain_str: string;
  items: BaroItem[];
}

interface RewardItem {
  name: string;
  rarity: string;
  chance: number;
}

interface RewardRotation {
  label: string;
  items: RewardItem[];
}

interface BountyJob {
  title: string;
  name: string;
  min_level: number;
  max_level: number;
  mastery_req: number;
  stages: number;
  standing: number;
  tier: string;
  rotations: RewardRotation[];
}

interface BountyInfo {
  syndicate: string;
  card: string;        // 点开本面板的周期卡名（多数=syndicate；解剖圣所=魔胎之境）
  expiry_ms: number;
  remain_ms: number;
  remain_str: string;
  active_rotation: string;
  jobs: BountyJob[];
}

interface CircuitInfo {
  normal: string[];   // 普通回廊·战甲
  hard: string[];     // 钢铁之路回廊·Incarnon 武器
  expiry_ms: number;
  remain_ms: number;
  remain_str: string;
}

interface ArbitrationSlot {
  node: string;
  planet: string;
  mission: string;
  faction: string;
  min_level: number;
  max_level: number;
  archwing: boolean;
}

interface ArbitrationInfo {
  current: ArbitrationSlot;
  upcoming: ArbitrationSlot[];
  expiry_ms: number;
  remain_ms: number;
  remain_str: string;
}

interface AppStatePayload {
  normal_fissures: Fissure[];
  hard_fissures: Fissure[];
  storm_fissures: Fissure[];
  cycles: CycleInfo[];
  last_update: string;
  countdown_secs: number;
  mission_timer: MissionTimerPayload;
  baro: BaroInfo | null;
  bounties: BountyInfo[];
  circuit: CircuitInfo | null;
  arbitration: ArbitrationInfo | null;
}

// ── Tier colors (match Python original) ──
const TIER_BG: Record<string, string> = {
  VoidT1: '#564b43', VoidT2: '#3e4140', VoidT3: '#383839',
  VoidT4: '#56523f', VoidT5: '#443037', VoidT6: '#384757',
};
const TIER_FG = '#ddd5c5';

let currentData: AppStatePayload | null = null;
let currentSubTab = 'normal';
let currentConfig: AppConfig | null = null;
function updateTimerConfig(partial: Record<string, any>) {
  if (!currentConfig) return;
  const newCfg = {
    ...currentConfig,
    mission_timer: { ...currentConfig.mission_timer, ...partial }
  };
  currentConfig = newCfg;
  invoke('set_config', { config: newCfg });
}

// ── Format remaining time ──
function fmtRemain(ms: number): string {
  if (ms <= 0) return '切换中';
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${String(m).padStart(2, '0')}m ${String(sec).padStart(2, '0')}s`;
  if (m > 0) return `${m}m ${String(sec).padStart(2, '0')}s`;
  return `${sec}s`;
}

// Which cycle location currently has its bounty panel open (null = none).
let openBounty: string | null = null;
// Whether the Duviri Circuit panel is open.
let openCircuit = false;
// Whether the Baro items table is expanded.
let openBaro = false;
// Whether the Arbitration upcoming slots panel is expanded.
let openArbitration = false;
// The Duviri cycle card opens the Circuit panel instead of a bounty board.
const CIRCUIT_CYCLE = '双衍王境';

// ── Render functions ──
function renderCycles(cycles: CycleInfo[]) {
  const container = document.getElementById('cycle-cards')!;
  const withBounty = new Set((currentData?.bounties ?? []).map(b => b.card));
  const hasCircuit = !!currentData?.circuit;
  container.innerHTML = cycles.map(c => {
    const isCircuit = c.name === CIRCUIT_CYCLE && hasCircuit;
    const clickable = withBounty.has(c.name) || isCircuit;
    const isOpen = isCircuit ? openCircuit : openBounty === c.name;
    const tag = isCircuit ? '回廊' : '赏金';
    return `
    <div class="cycle-card ${c.is_day ? 'day' : 'night'}${clickable ? ' clickable' : ''}${isOpen ? ' open' : ''}"${clickable ? ` data-cycle="${c.name}"` : ''}>
      <div class="card-name">${c.name}${clickable ? ` <span class="bounty-tag">${tag}</span>` : ''}</div>
      <div class="card-state">${c.state}</div>
      <div class="card-time">剩余 ${fmtRemain(c.remain_ms)}</div>
    </div>`;
  }).join('');
}

const BOUNTY_COLS = 4;

function rarityCls(r: string): string {
  switch (r) {
    case 'Common': return 'rk-common';
    case 'Uncommon': return 'rk-uncommon';
    case 'Rare': return 'rk-rare';
    case 'Legendary': return 'rk-legendary';
    default: return 'rk-common';
  }
}

// Bounty board card, shown under the cycle cards when a 赏金 card is clicked.
// Branded orange header + rotation A/B/C selector + per-bounty blocks
// (number · title · 等级) each over a clean 4-column zebra reward grid, items
// colored by rarity (hover shows drop chance).
// Panel header shows the syndicate's proper name, while `syndicate` itself stays
// the open-world location (must match the cycle-card name for click-to-open).
const SYNDICATE_DISPLAY: Record<string, string> = {
  '夜灵平野': '希图斯',
  '奥布山谷': '索拉里斯联盟',
  '扎里曼': '坚守者',
  '魔胎之境': '英择谛',
  '霍瓦尼亚': '六人组',
};

function renderBountyPanel(bounties: BountyInfo[]) {
  const panel = document.getElementById('bounty-panel')!;
  // One card can host several boards (解剖圣所 hangs under 魔胎之境). Render a
  // section per BountyInfo whose `card` matches the open card.
  const list = openBounty ? bounties.filter(x => x.card === openBounty) : [];
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
    openBounty = null;
    renderBountyPanel(bounties);
    renderCycles(currentData?.cycles ?? []);
  });
}

// Duviri Circuit (无限回廊) panel — this week's selectable 战甲 (普通回廊) and
// Incarnon 武器 (钢铁之路回廊), with the weekly (Monday) refresh countdown.
// Shown under the cycle cards when the 双衍王境 card is clicked.
function renderCircuitPanel(circuit: CircuitInfo | null) {
  const panel = document.getElementById('circuit-panel')!;
  if (!circuit || !openCircuit) {
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
    </div>
    <div class="bounty-foot">以上为无限回廊每周一刷新的<b>奖励</b>（非可用战甲/武器）：普通回廊可获这些战甲，钢铁之路可获这些武器的灵化之源。战甲名国服保留英文。</div>`;
  document.getElementById('circuit-close')!.addEventListener('click', () => {
    openCircuit = false;
    renderCircuitPanel(circuit);
    renderCycles(currentData?.cycles ?? []);
  });
}

// Signature of the structural (non-countdown) parts of the Baro panel. The
// per-second tick only changes the countdown, so we rebuild the panel DOM only
// when this signature changes — otherwise we just patch the countdown text.
// Rebuilding every tick would destroy the货物 table's scroll container and snap
// the user's scrollbar back to the top.
let baroSig = '';

function renderBaro(baro: BaroInfo | null) {
  const container = document.getElementById('baro-card')!;
  if (!baro) { container.innerHTML = ''; baroSig = ''; return; }

  const sig = `${baro.active}|${baro.location}|${baro.items.length}|${openBaro}`;
  const cdLabel = baro.active ? '离开倒计时' : '到达倒计时';

  // Fast path: structure unchanged → only update the live countdown.
  if (sig === baroSig) {
    const cd = container.querySelector('.baro-countdown');
    if (cd) cd.textContent = `${cdLabel} ${baro.remain_str}`;
    return;
  }
  baroSig = sig;

  if (baro.active) {
    const rows = baro.items.map(it => `
      <tr>
        <td>${it.name}</td>
        <td class="baro-ducats">${it.ducats > 0 ? it.ducats : '—'}</td>
        <td class="baro-credits">${it.credits.toLocaleString()}</td>
      </tr>
    `).join('');
    const tableHtml = openBaro ? `
      <div class="baro-table-wrap">
        <table class="baro-table">
          <thead><tr><th>物品</th><th>杜卡德</th><th>现金</th></tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>` : '';
    container.innerHTML = `
      <div class="baro-panel active clickable${openBaro ? ' open' : ''}">
        <div class="baro-head">
          <span class="baro-title">🛒 虚空商人 Baro Ki'Teer</span>
          <span class="baro-loc">${baro.location}</span>
          <span class="baro-countdown">${cdLabel} ${baro.remain_str}</span>
        </div>
        ${tableHtml}
      </div>`;
  } else {
    container.innerHTML = `
      <div class="baro-panel waiting">
        <div class="baro-head">
          <span class="baro-title">🛒 虚空商人 Baro Ki'Teer</span>
          <span class="baro-loc">${baro.location}</span>
          <span class="baro-countdown">${cdLabel} ${baro.remain_str}</span>
        </div>
        <div class="baro-wait-note">尚未到达，到达后可点击展开货物清单</div>
      </div>`;
  }
}

// Shared fissure filter: drop expired, then apply tier/type selections.
function filterFissures(list: Fissure[], tier: string, type: string): Fissure[] {
  return list.filter(f => {
    if (f.remain_ms <= 0) return false;
    if (tier && f.tier_label !== tier) return false;
    if (type && f.mission_type !== type) return false;
    return true;
  });
}

// Does this fissure match any active 裂缝 subscription rule? Mirrors the backend
// `check_fissure_alerts` matching so the list highlight reflects what's notified.
function fissureSubscribed(f: Fissure): boolean {
  const alerts = currentConfig?.fissure_alerts ?? [];
  if (!alerts.length) return false;
  const diff = f.is_hard ? 'hard' : f.is_storm ? 'storm' : 'normal';
  return alerts.some(a =>
    (!a.tier || a.tier === f.tier_label) &&
    (!a.mission_type || a.mission_type === f.mission_type) &&
    (!a.difficulty || a.difficulty === diff));
}

function getFilteredFissures(): Fissure[] {
  if (!currentData) return [];
  const tier = (document.getElementById('tier-filter') as HTMLSelectElement).value;
  const type = (document.getElementById('type-filter') as HTMLSelectElement).value;
  let list: Fissure[];
  if (currentSubTab === 'normal') list = currentData.normal_fissures;
  else if (currentSubTab === 'hard') list = currentData.hard_fissures;
  else list = currentData.storm_fissures;
  return filterFissures(list, tier, type);
}

function renderTimer(t: MissionTimerPayload) {
  // Sync time
  document.getElementById('timer-digits')!.textContent = t.elapsed_str;

  // OCR raw digits (from backend, if available)
  const ocrEl = document.getElementById('timer-ocr-digits');
  if (ocrEl && t.ocr_raw) {
    ocrEl.textContent = t.ocr_raw;
  } else if (ocrEl && t.state === 'idle') {
    ocrEl.textContent = '--:--';
  }

  // Status
  const statusEl = document.getElementById('timer-status')!;
  statusEl.textContent = t.status_text;
  statusEl.className = 'timer-status';
  if (t.state === 'checkpoint') statusEl.classList.add('checkpoint');

  // Detection rate
  const rateEl = document.getElementById('timer-rate')!;
  if (t.state === 'running') {
    rateEl.textContent = `识别率 ${t.detection_rate.toFixed(0)}%`;
  } else if (t.state === 'idle') {
    rateEl.textContent = '';
  }

  // Window status
  const wsEl = document.getElementById('window-status');
  if (wsEl && t.window_status) {
    wsEl.textContent = t.window_status;
    wsEl.className = 'window-status ' + (t.window_status.includes('检测到') ? 'found' : 'not-found');
  }

  // Life support indicator
  const lsDot = document.getElementById('ls-dot')!;
  const lsStatus = document.getElementById('ls-status')!;
  if (t.state === 'idle') {
    lsDot.className = 'ls-dot';
    lsStatus.className = 'ls-status';
    lsStatus.textContent = '--';
  } else if (t.life_support_level === 'danger') {
    lsDot.className = 'ls-dot danger';
    lsStatus.className = 'ls-status danger';
    lsStatus.textContent = '≤20%';
  } else {
    lsDot.className = 'ls-dot normal';
    lsStatus.className = 'ls-status';
    lsStatus.textContent = '正常';
  }

  // Next checkpoint countdown
  if (t.state === 'running') {
    const elapsed = t.elapsed_secs;
    const next5 = Math.ceil(elapsed / 300) * 300;
    const remain = next5 - elapsed;
    const m5 = Math.floor(next5 / 60);
    const s5 = next5 % 60;
    document.getElementById('cp-target')!.textContent = `${m5}:${String(s5).padStart(2, '0')}`;
    const rm5 = Math.floor(remain / 60);
    const rs5 = remain % 60;
    document.getElementById('cp-remain')!.textContent = `${rm5}:${String(rs5).padStart(2, '0')}`;
    document.getElementById('next-checkpoint')!.style.display = '';
  } else {
    document.getElementById('next-checkpoint')!.style.display = 'none';
  }
}

// Programmatic tab activation (shared by click handlers and popup click-through).
function activateTab(tab: string) {
  const btn = document.querySelector(`.tab-btn[data-tab="${tab}"]`) as HTMLButtonElement | null;
  if (!btn || btn.classList.contains('locked') || btn.disabled) return;
  document.querySelectorAll('.tab-btn, .tab-content').forEach(e => e.classList.remove('active'));
  btn.classList.add('active');
  document.getElementById(`tab-${tab}`)!.classList.add('active');
}

function activateSubTab(sub: string) {
  const btn = document.querySelector(`.sub-tab-btn[data-sub="${sub}"]`) as HTMLButtonElement | null;
  if (!btn) return;
  document.querySelectorAll('.sub-tab-btn').forEach(e => e.classList.remove('active'));
  btn.classList.add('active');
  currentSubTab = sub;
}

// Scroll the fissure row for `node` into view and pulse it briefly. Runs after
// the next paint so the freshly-rendered rows exist.
function highlightFissureRow(node: string) {
  if (!node) return;
  requestAnimationFrame(() => {
    const rows = document.querySelectorAll('#fissure-table tbody tr');
    const row = Array.from(rows).find(
      r => (r as HTMLElement).dataset.node === node) as HTMLElement | undefined;
    if (!row) return;
    row.scrollIntoView({ block: 'center', behavior: 'smooth' });
    row.classList.add('row-flash');
    setTimeout(() => row.classList.remove('row-flash'), 2400);
  });
}

function renderFissures() {
  if (!currentData) return;
  const filtered = getFilteredFissures();
  const tbody = document.querySelector('#fissure-table tbody')!;
  tbody.innerHTML = filtered.map(f => {
    const sub = fissureSubscribed(f);
    const cls = `${f.is_expiring ? 'expiring' : ''}${sub ? ' subscribed' : ''}`.trim();
    return `
    <tr class="${cls}" data-node="${f.node_name}" style="background:${TIER_BG[f.tier_key] || '#252525'};color:${TIER_FG};">
      <td>${sub ? '<span class="sub-bell">🔔</span>' : ''}<img src="/relics/${f.tier_key}.png" class="relic-icon" alt=""> ${f.tier_label}</td>
      <td>${f.node_name}</td>
      <td>${f.planet}</td>
      <td>${f.mission_type}</td>
      <td>${f.remain_str}</td>
    </tr>`;
  }).join('');

  // Update counts with current filters applied
  const tier = (document.getElementById('tier-filter') as HTMLSelectElement).value;
  const type = (document.getElementById('type-filter') as HTMLSelectElement).value;
  document.getElementById('count-normal')!.textContent = String(filterFissures(currentData.normal_fissures, tier, type).length);
  document.getElementById('count-hard')!.textContent = String(filterFissures(currentData.hard_fissures, tier, type).length);
  document.getElementById('count-storm')!.textContent = String(filterFissures(currentData.storm_fissures, tier, type).length);
}

function updateFilters() {
  if (!currentData) return;
  const allFissures = [...currentData.normal_fissures, ...currentData.hard_fissures];
  const tiers = [...new Set(allFissures.map(f => f.tier_label))];
  const types = [...new Set(allFissures.map(f => f.mission_type).filter(t => t && t !== '--'))].sort();

  const tierSelect = document.getElementById('tier-filter') as HTMLSelectElement;
  const typeSelect = document.getElementById('type-filter') as HTMLSelectElement;
  const currentTier = tierSelect.value;
  const currentType = typeSelect.value;

  tierSelect.innerHTML = '<option value="">全部</option>' + tiers.map(t => `<option>${t}</option>`).join('');
  typeSelect.innerHTML = '<option value="">全部</option>' + types.map(t => `<option>${t}</option>`).join('');
  tierSelect.value = currentTier;
  typeSelect.value = currentType;
}

let arbSig = '';

function fmtHourRange(startMs: number, endMs: number): string {
  const fmt = (ms: number) => {
    const d = new Date(ms);
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };
  return `${fmt(startMs)}–${fmt(endMs)}`;
}

function arbSlotHtml(slot: ArbitrationSlot, label: string): string {
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

function renderArbitration(arb: ArbitrationInfo | null) {
  const el = document.getElementById('arbitration-card')!;
  if (!arb) { el.innerHTML = ''; arbSig = ''; return; }

  const sig = `${arb.current.node}|${arb.remain_str}|${openArbitration}`;
  if (sig === arbSig) return;
  arbSig = sig;

  const HOUR = 3600 * 1000;
  const expiry = arb.expiry_ms;
  const currentLabel = fmtHourRange(expiry - HOUR, expiry);
  const upcomingHtml = openArbitration
    ? `<div class="arb-upcoming">${arb.upcoming.map((s, i) =>
        arbSlotHtml(s, fmtHourRange(expiry + i * HOUR, expiry + (i + 1) * HOUR))
      ).join('')}</div>`
    : '';

  el.innerHTML = `
    <div class="arb-card clickable${openArbitration ? ' open' : ''}">
      <div class="arb-head">
        <span class="arb-title">⚔ 仲裁</span>
        <span class="arb-countdown">${arb.remain_str}</span>
      </div>
      ${arbSlotHtml(arb.current, currentLabel)}
      ${upcomingHtml}
    </div>`;
}

// Signature of subscription-relevant data: only rebuild alert UIs when
// fissure mission types or arbitration node actually change (not every tick).
let _refreshAlertsCb: (() => void) | null = null;
let _lastAlertSig = '';

function handleUpdate(payload: AppStatePayload) {
  currentData = payload;
  // Subtle "last updated" watermark at the bottom of the 世界时间 tab. The
  // "下次刷新 Ns" countdown is intentionally hidden for now.
  document.getElementById('cycles-updated')!.textContent =
    `更新于 ${payload.last_update}`;
  renderCycles(payload.cycles);
  renderBountyPanel(payload.bounties);
  renderCircuitPanel(payload.circuit);
  renderBaro(payload.baro);
  renderArbitration(payload.arbitration);
  updateFilters();
  renderFissures();
  renderTimer(payload.mission_timer);
  // Only re-render alert rule lists when underlying data changes structurally,
  // not on every per-second tick (which would close open dropdowns).
  const sig = availableMissionTypes().join(',') + '|' + (payload.arbitration?.current.node ?? '');
  if (sig !== _lastAlertSig) {
    _lastAlertSig = sig;
    _refreshAlertsCb?.();
  }
}

// ── Event listeners ──
window.addEventListener('DOMContentLoaded', () => {
  // Lock the 任务计时 tab in production builds (the shipped installer) but keep
  // it usable during local development (`tauri dev`). Vite sets PROD only for
  // `tauri build` output. A self-use "unlocked" installer can be produced by
  // building with VITE_UNLOCK_TIMER=1 (keeps the timer enabled in a prod build).
  if ((import.meta as any).env?.PROD && (import.meta as any).env?.VITE_UNLOCK_TIMER !== '1') {
    const timerTab = document.querySelector('.tab-btn[data-tab="timer"]') as HTMLButtonElement | null;
    if (timerTab) {
      timerTab.classList.add('locked');
      timerTab.disabled = true;
      timerTab.title = '该功能暂未开放';
      timerTab.textContent = '任务计时 🔒';
    }
  }

  // Tab switching
  document.querySelectorAll('.tab-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      activateTab((btn as HTMLElement).dataset.tab!);
    });
  });

  // Click a 赏金 cycle card to toggle its inline bounty panel
  document.getElementById('cycle-cards')!.addEventListener('click', (e) => {
    const card = (e.target as HTMLElement).closest('.cycle-card.clickable') as HTMLElement | null;
    if (!card) return;
    const name = card.dataset.cycle!;
    if (name === CIRCUIT_CYCLE) {
      openCircuit = !openCircuit;
      openBounty = null;  // keep only one panel open
    } else {
      openBounty = openBounty === name ? null : name;
      openCircuit = false;
    }
    renderBountyPanel(currentData?.bounties ?? []);
    renderCircuitPanel(currentData?.circuit ?? null);
    renderCycles(currentData?.cycles ?? []);
  });

  // Sub-tab switching
  document.querySelectorAll('.sub-tab-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      activateSubTab((btn as HTMLElement).dataset.sub!);
      renderFissures();
    });
  });

  // Refresh button
  document.getElementById('btn-refresh')!.addEventListener('click', () => {
    invoke('refresh_now');
  });

  // Filters
  document.getElementById('tier-filter')!.addEventListener('change', renderFissures);
  document.getElementById('type-filter')!.addEventListener('change', renderFissures);

  // Settings: load config
  const closeToggle = document.getElementById('setting-close-to-tray') as HTMLInputElement;
  invoke<AppConfig>('get_config').then(cfg => {
    currentConfig = cfg;
    closeToggle.checked = cfg.close_to_tray;
    // Init timer settings
    const mt = cfg.mission_timer;
    document.getElementById('ocr-interval')!.setAttribute('value', String(mt.ocr_interval_secs));
    (document.getElementById('toggle-checkpoint-focus') as HTMLInputElement).checked = mt.checkpoint_auto_focus;
    (document.getElementById('toggle-hp-alert') as HTMLInputElement).checked = mt.hp_alert_enabled;
    // Init alert-method radio (default "focus" if unset)
    document.querySelectorAll<HTMLInputElement>('input[name="alert-method"]').forEach(r => {
      r.checked = r.value === (mt.alert_method || 'focus');
    });
    // Init custom reminder text inputs
    (document.getElementById('checkpoint-text') as HTMLInputElement).value = mt.checkpoint_alert_text ?? '';
    (document.getElementById('hp-text') as HTMLInputElement).value = mt.hp_alert_text ?? '';
    // Init mode radio from saved config
    document.querySelectorAll<HTMLInputElement>('input[name="timer-mode"]').forEach(r => {
      r.checked = r.value === mt.mode;
    });
    // Init subscription rules
    refreshAlerts();
  }).catch((err: unknown) => {
    console.error('get_config 失败:', err);
  });

  const refreshAlerts = setupAlerts();
  _refreshAlertsCb = refreshAlerts;

  // Autostart toggle: read current registry state on init, write on change
  const autostartToggle = document.getElementById('setting-autostart') as HTMLInputElement;
  invoke<boolean>('get_autostart').then(v => { autostartToggle.checked = v; });
  autostartToggle.addEventListener('change', () => {
    invoke('set_autostart', { enabled: autostartToggle.checked });
  });

  // Settings: save on change
  closeToggle.addEventListener('change', () => {
    if (!currentConfig) return;
    const newCfg = { ...currentConfig, close_to_tray: closeToggle.checked };
    currentConfig = newCfg;
    invoke('set_config', { config: newCfg });
  });

  // Timer: start/stop/reset buttons
  document.getElementById('btn-timer-start')!.addEventListener('click', () => {
    invoke('timer_command', { command: 'start' });
  });
  document.getElementById('btn-timer-stop')!.addEventListener('click', () => {
    invoke('timer_command', { command: 'stop' });
  });
  document.getElementById('btn-timer-reset')!.addEventListener('click', () => {
    invoke('timer_command', { command: 'reset' });
  });

  // Timer: mode radio
  document.querySelectorAll('input[name="timer-mode"]').forEach(radio => {
    radio.addEventListener('change', () => {
      if ((radio as HTMLInputElement).checked) {
        invoke('timer_command', { command: 'set_mode', mode: (radio as HTMLInputElement).value });
      }
    });
  });

  // Single capture button
  document.getElementById('btn-single-capture')!.addEventListener('click', () => {
    invoke('single_capture');
  });

  // OCR interval
  const ocrInterval = document.getElementById('ocr-interval') as HTMLInputElement;
  ocrInterval.addEventListener('change', () => {
    const val = Math.max(1, Math.min(30, parseInt(ocrInterval.value) || 2));
    ocrInterval.value = String(val);
    if (!currentConfig) return;
    const newCfg = {
      ...currentConfig,
      mission_timer: { ...currentConfig.mission_timer, ocr_interval_secs: val }
    };
    currentConfig = newCfg;
    invoke('set_config', { config: newCfg });
  });

  // Toggle: checkpoint auto-focus
  document.getElementById('toggle-checkpoint-focus')!.addEventListener('change', function(this: HTMLInputElement) {
    updateTimerConfig({ checkpoint_auto_focus: this.checked });
  });
  // Toggle: HP alert
  document.getElementById('toggle-hp-alert')!.addEventListener('change', function(this: HTMLInputElement) {
    updateTimerConfig({ hp_alert_enabled: this.checked });
  });

  // Alert method: force-focus vs Windows toast
  document.querySelectorAll('input[name="alert-method"]').forEach(radio => {
    radio.addEventListener('change', function(this: HTMLInputElement) {
      if (this.checked) updateTimerConfig({ alert_method: this.value });
    });
  });

  // Custom reminder text (saved on blur/change; empty falls back to default in Rust)
  document.getElementById('checkpoint-text')!.addEventListener('change', function(this: HTMLInputElement) {
    updateTimerConfig({ checkpoint_alert_text: this.value });
  });
  document.getElementById('hp-text')!.addEventListener('change', function(this: HTMLInputElement) {
    updateTimerConfig({ hp_alert_text: this.value });
  });

  // Custom reminder text popup: open / close
  const alertModal = document.getElementById('alert-text-modal')!;
  document.getElementById('btn-edit-alert-text')!.addEventListener('click', () => {
    alertModal.classList.remove('hidden');
  });
  document.getElementById('btn-close-alert-modal')!.addEventListener('click', () => {
    alertModal.classList.add('hidden');
  });
  // Click the dimmed backdrop (outside the box) to dismiss
  alertModal.addEventListener('click', (e) => {
    if (e.target === alertModal) alertModal.classList.add('hidden');
  });

  // Software update
  const updateStatus = document.getElementById('update-status')!;
  const checkUpdateBtn = document.getElementById('btn-check-update') as HTMLButtonElement;
  const updateModal = document.getElementById('update-modal')!;
  const updateModalStatus = document.getElementById('update-modal-status')!;
  const confirmUpdateBtn = document.getElementById('btn-confirm-update') as HTMLButtonElement;
  getVersion().then(v => {
    document.getElementById('update-cur-version')!.textContent = `当前版本 ${v}`;
  });
  const updateSource = (): string =>
    (document.querySelector('input[name="update-source"]:checked') as HTMLInputElement | null)?.value ?? 'gitee';
  checkUpdateBtn.addEventListener('click', () => {
    checkUpdateBtn.disabled = true;
    updateStatus.textContent = '检查中…';
    invoke<{ version: string; notes: string } | null>('check_for_update', { source: updateSource() })
      .then(info => {
        if (info) {
          updateStatus.textContent = '';
          document.getElementById('update-modal-version')!.textContent = `最新版本：${info.version}`;
          document.getElementById('update-modal-notes')!.textContent = info.notes;
          updateModalStatus.textContent = '';
          confirmUpdateBtn.disabled = false;
          updateModal.classList.remove('hidden');
        } else {
          updateStatus.textContent = '✅ 已是最新版本';
        }
      })
      .catch(err => { updateStatus.textContent = `❌ ${String(err)}`; })
      .finally(() => { checkUpdateBtn.disabled = false; });
  });
  document.getElementById('btn-cancel-update')!.addEventListener('click', () => {
    updateModal.classList.add('hidden');
  });
  confirmUpdateBtn.addEventListener('click', () => {
    confirmUpdateBtn.disabled = true;
    updateModalStatus.textContent = '下载中，请稍候…';
    invoke('install_update', { source: updateSource() })
      .catch(err => {
        updateModalStatus.textContent = `❌ ${String(err)}`;
        confirmUpdateBtn.disabled = false;
      });
  });

  // Uninstall flow
  const uninstallModal = document.getElementById('uninstall-modal')!;
  const uninstallStatus = document.getElementById('uninstall-status')!;
  document.getElementById('btn-uninstall')!.addEventListener('click', () => {
    uninstallStatus.textContent = '';
    uninstallModal.classList.remove('hidden');
  });
  document.getElementById('btn-cancel-uninstall')!.addEventListener('click', () => {
    uninstallModal.classList.add('hidden');
  });
  uninstallModal.addEventListener('click', (e) => {
    if (e.target === uninstallModal) uninstallModal.classList.add('hidden');
  });
  document.getElementById('btn-confirm-uninstall')!.addEventListener('click', () => {
    uninstallStatus.textContent = '正在清理数据…';
    (document.getElementById('btn-confirm-uninstall') as HTMLButtonElement).disabled = true;
    invoke('uninstall_clean')
      .catch(err => {
        uninstallStatus.textContent = String(err);
        (document.getElementById('btn-confirm-uninstall') as HTMLButtonElement).disabled = false;
      });
  });

  // Test the currently selected alert method
  const alertTestStatus = document.getElementById('alert-test-status')!;
  document.getElementById('btn-test-alert')!.addEventListener('click', () => {
    alertTestStatus.textContent = '测试中…';
    invoke('test_alert')
      .then(() => { alertTestStatus.textContent = '✅ 已触发'; })
      .catch(err => { alertTestStatus.textContent = String(err); });
  });

  // Baro card: click to expand/collapse items table (only when active)
  document.getElementById('baro-card')!.addEventListener('click', () => {
    if (currentData?.baro?.active) {
      openBaro = !openBaro;
      renderBaro(currentData.baro);
    }
  });

  // Arbitration card: click to expand/collapse upcoming slots
  document.getElementById('arbitration-card')!.addEventListener('click', () => {
    if (currentData?.arbitration) {
      openArbitration = !openArbitration;
      renderArbitration(currentData.arbitration);
    }
  });

  // Item-name 中文 table: the bundled table refreshes per release, so show the
  // game version it covers instead of an on-demand update button.
  const itemNamesStatus = document.getElementById('itemnames-status')!;
  invoke<string>('game_data_version')
    .then(v => { itemNamesStatus.textContent = v; })
    .catch(() => { itemNamesStatus.textContent = ''; });

  // Log listener
  let logLines: string[] = [];
  const MAX_LOG = 200;
  const logContent = document.getElementById('log-content')!;
  listen<string>('timer-log', (event) => {
    logLines.push(event.payload);
    if (logLines.length > MAX_LOG) logLines = logLines.slice(-MAX_LOG);
    logContent.innerHTML = logLines.map(line => {
      let cls = 'log-info';
      if (line.includes('⚠')) cls = 'log-warn';
      else if (line.includes('同步') || line.includes('OCR')) cls = 'log-ok';
      return `<div class="${cls}">${line}</div>`;
    }).join('');
    logContent.scrollTop = logContent.scrollHeight;
  });

  document.getElementById('btn-clear-log')!.addEventListener('click', () => {
    logLines = [];
    logContent.innerHTML = '';
  });

  // Tauri events
  listen<AppStatePayload>('worldstate-update', (event) => {
    handleUpdate(event.payload);
  });

  listen<AppStatePayload>('tick-update', (event) => {
    handleUpdate(event.payload);
  });

  // Click-through from a tray-popup notification → jump to the matching entry.
  listen<{ kind: string; node: string; sub: string }>('navigate', (event) => {
    const { kind, node, sub } = event.payload;
    if (kind === 'fissure') {
      activateTab('fissures');
      if (sub) activateSubTab(sub);
      // Clear filters so the target row can't be hidden by an active filter.
      (document.getElementById('tier-filter') as HTMLSelectElement).value = '';
      (document.getElementById('type-filter') as HTMLSelectElement).value = '';
      renderFissures();
      highlightFissureRow(node);
    } else {
      // cycle / arbitration live on the 世界时间 tab.
      activateTab('cycles');
    }
  });

  // ── ROI calibration ──
  setupCalibration();
});

// ── Subscription (alert rules) management ──────────────────────────────────

const FISSURE_TIERS = ['', '古纪', '前纪', '中纪', '后纪', '全能', '恐惧'];
const FISSURE_TIER_LABELS: Record<string, string> = {
  '': '任意遗物', '恐惧': '恐惧（钢铁）',
};
const FISSURE_DIFFICULTIES = ['', 'normal', 'hard', 'storm'];
const FISSURE_DIFF_LABELS: Record<string, string> = {
  '': '任意难度', 'normal': '普通', 'hard': '钢铁之路', 'storm': '虚空风暴',
};

const CYCLE_LOCATIONS: Record<string, string[]> = {
  '夜灵平野': ['白天', '黑夜'],
  '奥布山谷': ['温暖', '寒冷'],
  '魔胎之境': ['Fass', 'Vome'],
  '扎里曼': ['Corpus', 'Grineer'],
  '双衍王境': ['悲伤', '恐惧', '喜悦', '愤怒', '嫉妒'],
};

// Derive available mission types from current worldstate fissure data.
function availableMissionTypes(): string[] {
  if (!currentData) return [''];
  const all = [
    ...currentData.normal_fissures,
    ...currentData.hard_fissures,
    ...currentData.storm_fissures,
  ];
  const types = [...new Set(all.map(f => f.mission_type).filter(t => t && t !== '--'))].sort();
  return ['', ...types];
}

// Derive available arbitration mission types and nodes from current + upcoming slots.
function availableArbMissions(): string[] {
  if (!currentData?.arbitration) return [''];
  const arb = currentData.arbitration;
  const slots = [arb.current, ...arb.upcoming];
  const types = [...new Set(slots.map(s => s.mission).filter(Boolean))].sort();
  return ['', ...types];
}

function availableArbPlanets(): string[] {
  if (!currentData?.arbitration) return [''];
  const arb = currentData.arbitration;
  const slots = [arb.current, ...arb.upcoming];
  const planets = [...new Set(slots.map(s => s.planet).filter(Boolean))].sort();
  return ['', ...planets];
}

function renderArbitrationAlerts(list: ArbitrationAlert[], container: HTMLElement) {
  if (!list.length) {
    container.innerHTML = '<div class="alert-rules-empty">暂无规则，点击「+ 添加」开始订阅</div>';
    return;
  }
  const missions = availableArbMissions();
  const planets = availableArbPlanets();
  container.innerHTML = list.map((a, i) => `
    <div class="alert-rule-row" data-index="${i}">
      ${selOpts(missions, { '': '任意任务' }, a.mission_type, 'rule-sel arb-mission-sel')}
      ${selOpts(planets, { '': '任意星球' }, a.planet, 'rule-sel arb-planet-sel')}
      <button class="rule-del timer-btn-sm" data-index="${i}">删除</button>
    </div>`).join('');
}

function selOpts(options: string[], labels: Record<string, string>, current: string, cls: string): string {
  return `<select class="${cls}">${options.map(o =>
    `<option value="${o}"${o === current ? ' selected' : ''}>${labels[o] ?? (o || '任意任务')}</option>`
  ).join('')}</select>`;
}

function renderFissureAlerts(list: FissureAlert[], container: HTMLElement) {
  if (!list.length) {
    container.innerHTML = '<div class="alert-rules-empty">暂无规则，点击「+ 添加」开始订阅</div>';
    return;
  }
  const missions = availableMissionTypes();
  container.innerHTML = list.map((a, i) => `
    <div class="alert-rule-row" data-index="${i}">
      ${selOpts(FISSURE_TIERS, FISSURE_TIER_LABELS, a.tier, 'rule-sel tier-sel')}
      ${selOpts(missions, { '': '任意任务' }, a.mission_type, 'rule-sel mission-sel')}
      ${selOpts(FISSURE_DIFFICULTIES, FISSURE_DIFF_LABELS, a.difficulty, 'rule-sel diff-sel')}
      <button class="rule-del timer-btn-sm" data-index="${i}">删除</button>
    </div>`).join('');
}

function renderCycleAlerts(list: CycleAlert[], container: HTMLElement) {
  if (!list.length) {
    container.innerHTML = '<div class="alert-rules-empty">暂无规则，点击「+ 添加」开始订阅</div>';
    return;
  }
  const locations = Object.keys(CYCLE_LOCATIONS);
  container.innerHTML = list.map((a, i) => {
    const states = CYCLE_LOCATIONS[a.location] ?? CYCLE_LOCATIONS[locations[0]];
    const locSel = `<select class="rule-sel loc-sel">${locations.map(l =>
      `<option value="${l}"${l === a.location ? ' selected' : ''}>${l}</option>`
    ).join('')}</select>`;
    const stateSel = `<select class="rule-sel state-sel">${states.map(s =>
      `<option value="${s}"${s === a.state ? ' selected' : ''}>${s}</option>`
    ).join('')}</select>`;
    return `<div class="alert-rule-row" data-index="${i}">${locSel}进入${stateSel}<button class="rule-del timer-btn-sm" data-index="${i}">删除</button></div>`;
  }).join('');
}

function saveAlerts() {
  if (!currentConfig) return;
  invoke('set_config', { config: currentConfig });
  // Refresh the fissure list so subscription highlights track the rule change.
  if (currentData) renderFissures();
}

function setupAlerts(): () => void {
  const fissureList = document.getElementById('fissure-alerts-list')!;
  const cycleList = document.getElementById('cycle-alerts-list')!;
  const arbList = document.getElementById('arb-alerts-list')!;

  function refresh() {
    if (!currentConfig) return;
    renderFissureAlerts(currentConfig.fissure_alerts, fissureList);
    renderCycleAlerts(currentConfig.cycle_alerts, cycleList);
    renderArbitrationAlerts(currentConfig.arbitration_alerts, arbList);
  }

  document.getElementById('btn-add-fissure-alert')!.addEventListener('click', () => {
    if (!currentConfig) return;
    const missions = availableMissionTypes();
    const defaultMission = missions.find(m => m !== '') ?? '';
    currentConfig.fissure_alerts = [...currentConfig.fissure_alerts, { tier: '', mission_type: defaultMission, difficulty: '' }];
    refresh();
    saveAlerts();
  });

  document.getElementById('btn-add-cycle-alert')!.addEventListener('click', () => {
    if (!currentConfig) return;
    const loc = '夜灵平野';
    currentConfig.cycle_alerts = [...currentConfig.cycle_alerts, { location: loc, state: CYCLE_LOCATIONS[loc][0] }];
    refresh();
    saveAlerts();
  });

  document.getElementById('btn-add-arb-alert')!.addEventListener('click', () => {
    if (!currentConfig) return;
    currentConfig.arbitration_alerts = [...currentConfig.arbitration_alerts, { mission_type: '', planet: '' }];
    refresh();
    saveAlerts();
  });

  fissureList.addEventListener('change', (e) => {
    if (!currentConfig) return;
    const row = (e.target as HTMLElement).closest('.alert-rule-row') as HTMLElement | null;
    if (!row) return;
    const i = parseInt(row.dataset.index!);
    const rule = currentConfig.fissure_alerts[i];
    const cl = (e.target as HTMLElement).classList;
    if (cl.contains('tier-sel'))
      rule.tier = (e.target as HTMLSelectElement).value;
    else if (cl.contains('mission-sel'))
      rule.mission_type = (e.target as HTMLSelectElement).value;
    else if (cl.contains('diff-sel'))
      rule.difficulty = (e.target as HTMLSelectElement).value;
    saveAlerts();
  });

  fissureList.addEventListener('click', (e) => {
    if (!currentConfig || !(e.target as HTMLElement).classList.contains('rule-del')) return;
    const i = parseInt((e.target as HTMLElement).dataset.index!);
    currentConfig.fissure_alerts = currentConfig.fissure_alerts.filter((_, idx) => idx !== i);
    refresh();
    saveAlerts();
  });

  cycleList.addEventListener('change', (e) => {
    if (!currentConfig) return;
    const row = (e.target as HTMLElement).closest('.alert-rule-row') as HTMLElement | null;
    if (!row) return;
    const i = parseInt(row.dataset.index!);
    const rule = currentConfig.cycle_alerts[i];
    if ((e.target as HTMLElement).classList.contains('loc-sel')) {
      rule.location = (e.target as HTMLSelectElement).value;
      rule.state = CYCLE_LOCATIONS[rule.location][0];
      refresh();
    } else if ((e.target as HTMLElement).classList.contains('state-sel')) {
      rule.state = (e.target as HTMLSelectElement).value;
    }
    saveAlerts();
  });

  cycleList.addEventListener('click', (e) => {
    if (!currentConfig || !(e.target as HTMLElement).classList.contains('rule-del')) return;
    const i = parseInt((e.target as HTMLElement).dataset.index!);
    currentConfig.cycle_alerts = currentConfig.cycle_alerts.filter((_, idx) => idx !== i);
    refresh();
    saveAlerts();
  });

  arbList.addEventListener('change', (e) => {
    if (!currentConfig) return;
    const row = (e.target as HTMLElement).closest('.alert-rule-row') as HTMLElement | null;
    if (!row) return;
    const i = parseInt(row.dataset.index!);
    const rule = currentConfig.arbitration_alerts[i];
    if ((e.target as HTMLElement).classList.contains('arb-mission-sel'))
      rule.mission_type = (e.target as HTMLSelectElement).value;
    else if ((e.target as HTMLElement).classList.contains('arb-planet-sel'))
      rule.planet = (e.target as HTMLSelectElement).value;
    saveAlerts();
  });

  arbList.addEventListener('click', (e) => {
    if (!currentConfig || !(e.target as HTMLElement).classList.contains('rule-del')) return;
    const i = parseInt((e.target as HTMLElement).dataset.index!);
    currentConfig.arbitration_alerts = currentConfig.arbitration_alerts.filter((_, idx) => idx !== i);
    refresh();
    saveAlerts();
  });

  return refresh;
}

// ── End subscription management ────────────────────────────────────────────

type Box = { x: number; y: number; w: number; h: number };

function getTimerMode(): 'normal' | 'fissure' {
  const r = document.querySelector<HTMLInputElement>('input[name="timer-mode"]:checked');
  return (r?.value === 'fissure') ? 'fissure' : 'normal';
}

function setupCalibration() {
  const canvas = document.getElementById('calib-canvas') as HTMLCanvasElement;
  const ctx = canvas.getContext('2d')!;
  const resultEl = document.getElementById('calib-result')!;
  const btnTime = document.getElementById('btn-calib-time')!;
  const btnLs = document.getElementById('btn-calib-ls')!;

  let img: HTMLImageElement | null = null;
  const boxes: { time: Box | null; ls: Box | null } = { time: null, ls: null };
  let activeTool: 'time' | 'ls' | null = null;
  let dragging = false;
  let startX = 0, startY = 0;

  function roiToBox(roi: { x: number; y: number; w: number; h: number }): Box {
    return { x: roi.x * canvas.width, y: roi.y * canvas.height, w: roi.w * canvas.width, h: roi.h * canvas.height };
  }

  function redraw() {
    if (!img) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
    const draw = (b: Box | null, color: string, label: string) => {
      if (!b) return;
      ctx.lineWidth = 2;
      ctx.strokeStyle = color;
      ctx.strokeRect(b.x, b.y, b.w, b.h);
      ctx.fillStyle = color;
      ctx.font = '14px sans-serif';
      ctx.fillText(label, b.x + 2, Math.max(14, b.y - 4));
    };
    draw(boxes.time, '#4CAF50', '时间');
    draw(boxes.ls, '#FF9800', '维生');
  }

  // Map a pointer event to canvas backing-store pixel coordinates.
  function toCanvasXY(e: MouseEvent): [number, number] {
    const rect = canvas.getBoundingClientRect();
    const sx = canvas.width / rect.width;
    const sy = canvas.height / rect.height;
    return [(e.clientX - rect.left) * sx, (e.clientY - rect.top) * sy];
  }

  function setTool(tool: 'time' | 'ls' | null) {
    activeTool = tool;
    btnTime.classList.toggle('active', tool === 'time');
    btnLs.classList.toggle('active', tool === 'ls');
  }

  btnTime.addEventListener('click', () => setTool(activeTool === 'time' ? null : 'time'));
  btnLs.addEventListener('click', () => setTool(activeTool === 'ls' ? null : 'ls'));

  canvas.addEventListener('mousedown', (e) => {
    if (!img || !activeTool) return;
    dragging = true;
    [startX, startY] = toCanvasXY(e);
  });
  canvas.addEventListener('mousemove', (e) => {
    if (!dragging || !activeTool) return;
    const [cx, cy] = toCanvasXY(e);
    boxes[activeTool] = {
      x: Math.min(startX, cx), y: Math.min(startY, cy),
      w: Math.abs(cx - startX), h: Math.abs(cy - startY),
    };
    redraw();
  });
  const endDrag = () => { dragging = false; };
  canvas.addEventListener('mouseup', endDrag);
  canvas.addEventListener('mouseleave', endDrag);

  // Capture current game frame into the canvas.
  document.getElementById('btn-calib-capture')!.addEventListener('click', () => {
    resultEl.textContent = '截取中…';
    invoke<string>('capture_preview').then(dataUrl => {
      const im = new Image();
      im.onload = () => {
        img = im;
        canvas.width = im.naturalWidth;
        canvas.height = im.naturalHeight;
        // Seed boxes from current config so existing ROIs show as starting frames.
        if (currentConfig) {
          const mt = currentConfig.mission_timer;
          const mode = getTimerMode();
          boxes.time = roiToBox(mode === 'fissure' ? mt.fissure_roi : mt.normal_roi);
          boxes.ls = roiToBox(mode === 'fissure' ? mt.fissure_hp_roi : mt.life_support_roi);
        }
        redraw();
        resultEl.textContent = '已截图，拖框校准';
      };
      im.src = dataUrl;
    }).catch(err => { resultEl.textContent = String(err); });
  });

  // Test OCR on the current time box.
  document.getElementById('btn-calib-test')!.addEventListener('click', () => {
    if (!img || !boxes.time) { resultEl.textContent = '请先框选时间框'; return; }
    const b = boxes.time;
    const args = { x: b.x / canvas.width, y: b.y / canvas.height, w: b.w / canvas.width, h: b.h / canvas.height };
    resultEl.textContent = '识别中…';
    invoke<string>('test_recognize', args)
      .then(r => { resultEl.textContent = `识别: ${r}`; })
      .catch(err => { resultEl.textContent = String(err); });
  });

  // Save both boxes into the current mode's ROIs.
  document.getElementById('btn-calib-save')!.addEventListener('click', () => {
    if (!img || !boxes.time || !boxes.ls) { resultEl.textContent = '请先截图并框选两个区域'; return; }
    const frac = (b: Box) => ({ x: b.x / canvas.width, y: b.y / canvas.height, w: b.w / canvas.width, h: b.h / canvas.height });
    const mode = getTimerMode();
    if (mode === 'fissure') {
      updateTimerConfig({ fissure_roi: frac(boxes.time), fissure_hp_roi: frac(boxes.ls) });
    } else {
      updateTimerConfig({ normal_roi: frac(boxes.time), life_support_roi: frac(boxes.ls) });
    }
    resultEl.textContent = `已保存（${mode === 'fissure' ? '裂缝' : '普通'}模式）`;
  });
}
