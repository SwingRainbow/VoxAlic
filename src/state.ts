// ── Type definitions ──────────────────────────────────────────────────────

export interface FissureAlert {
  tier: string;         // "" = any
  mission_type: string; // "" = any
  difficulty: string;   // "normal"|"hard"|"storm"|"" = any
}

export interface CycleAlert {
  location: string;
  state: string;
  advance_minutes: number;  // 0 = on transition, 5/10/15 = advance notice
}

export interface ArbitrationAlert {
  mission_type: string;  // "" = any
  planet: string;        // "" = any
}

export interface AppConfig {
  close_to_tray: boolean;
  worldstate_source: string;
  notify_bark_url: string;
  update_source: string;
  market_language: string;
  hotkey: string | null;
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
export interface Fissure {
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

export interface CycleInfo {
  name: string;
  state: string;
  state_icon: string;
  remain_ms: number;
  is_day: boolean;
  remain_str: string;
}

export interface MissionTimerPayload {
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

export interface BaroItem {
  name: string;
  ducats: number;
  credits: number;
}

export interface BaroInfo {
  active: boolean;
  location: string;
  tag: string;       // "" = regular, "TennoCon" = TennoCon special
  start_ms: number;
  end_ms: number;
  remain_ms: number;
  remain_str: string;
  items: BaroItem[];
}

export interface RewardItem {
  name: string;
  rarity: string;
  chance: number;
}

export interface RewardRotation {
  label: string;
  items: RewardItem[];
}

export interface BountyJob {
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

export interface BountyInfo {
  syndicate: string;
  card: string;        // 点开本面板的周期卡名（多数=syndicate；解剖圣所=魔胎之境）
  expiry_ms: number;
  remain_ms: number;
  remain_str: string;
  active_rotation: string;
  jobs: BountyJob[];
}

export interface CircuitInfo {
  normal: string[];   // 普通回廊·战甲
  hard: string[];     // 钢铁之路回廊·Incarnon 武器
  expiry_ms: number;
  remain_ms: number;
  remain_str: string;
}

export interface ArbitrationSlot {
  node: string;
  planet: string;
  mission: string;
  faction: string;
  min_level: number;
  max_level: number;
  archwing: boolean;
}

export interface ArbitrationInfo {
  current: ArbitrationSlot;
  upcoming: ArbitrationSlot[];
  expiry_ms: number;
  remain_ms: number;
  remain_str: string;
  all_missions: string[];
  all_planets: string[];
}

export interface AppStatePayload {
  normal_fissures: Fissure[];
  hard_fissures: Fissure[];
  storm_fissures: Fissure[];
  cycles: CycleInfo[];
  last_update: string;
  countdown_secs: number;
  mission_timer: MissionTimerPayload;
  baro: BaroInfo[];
  bounties: BountyInfo[];
  circuit: CircuitInfo | null;
  arbitration: ArbitrationInfo | null;
}

// ── Warframe.Market types ──
export interface MarketItemSummary {
  slug: string;
  name: string;
  name_zh?: string;
  icon_url: string;
  mr: number | null;
  max_rank?: number | null;
  tags: string[];
}
export interface MarketOrder {
  order_type: string;
  platinum: number;
  quantity: number;
  player_name: string;
  reputation: number;
  status: string;
  mod_rank?: number | null;
}
export interface MarketItemFull {
  item: MarketItemSummary;
  ducats: number | null;
  trading_tax: number | null;
  set_root: boolean;
  set_parts: MarketItemSummary[];
  sell_orders: MarketOrder[];
  buy_orders: MarketOrder[];
}
export interface MarketCacheStatus {
  count: number;
  last_updated: string;
}

// ── Market Auth & Orders types ──

export interface MarketAuthStatus {
  logged_in: boolean;
  ingame_name: string | null;
  avatar: string | null;
  reputation: number | null;
  current_status: string | null;
}

export interface ProfileOrder {
  id: string;
  order_type: string;   // "sell" | "buy"
  item_id: string;
  item_slug: string;
  item_name: string;
  platinum: number;
  quantity: number;
  rank: number;
  visible: boolean;
  platform: string;
  creation_date: string;
}

export interface CreateOrderRequest {
  item_id: string;
  order_type: string;
  platinum: number;
  quantity: number;
  rank: number;
  visible: boolean;
}

export interface MarketCommandError {
  code: string;
  message: string;
}

export type Box = { x: number; y: number; w: number; h: number };

// ── Tier colors (match Python original) ──
export const TIER_BG: Record<string, string> = {
  VoidT1: '#564b43', VoidT2: '#3e4140', VoidT3: '#383839',
  VoidT4: '#56523f', VoidT5: '#443037', VoidT6: '#384757',
};
export const TIER_FG = '#ddd5c5';

// ── Mutable global state (wrapped in object for cross-module mutation) ─────

export const S = {
  currentData: null as AppStatePayload | null,
  currentSubTab: 'normal' as string,
  currentConfig: null as AppConfig | null,

  // Which cycle location currently has its bounty panel open (null = none).
  openBounty: null as string | null,
  // Whether the Duviri Circuit panel is open.
  openCircuit: false as boolean,
  // Whether the Baro items table is expanded.
  openBaro: null as string | null,  // location of the currently expanded Baro card (null = none)
  // Whether the Arbitration upcoming slots panel is expanded.
  openArbitration: false as boolean,

  // Market state
  marketOpenSlug: null as string | null,  // currently expanded item slug
  marketReqId: 0 as number,              // race-condition guard
  marketSearchTimer: null as ReturnType<typeof setTimeout> | null,
  marketLang: 'en' as string,            // 'en' | 'zh'
  _lastMarketDetail: null as MarketItemFull | null,  // cached for language-switch re-render
  _lastMarketResults: [] as MarketItemSummary[],     // last search results (for hot-switch)

  // Market auth & orders state
  marketAuthName: null as string | null,       // logged-in ingame_name; null = not logged in
  marketAuthAvatar: null as string | null,     // avatar URL
  marketAuthStatus: null as string | null,     // current online status (online/ingame/invisible)
  marketAuthRep: null as number | null,        // reputation score
  myOrders: [] as ProfileOrder[],              // own profile orders
  myOrdersBySlug: new Map<string, number>(),   // slug → order count (fast lookup)
  orderFormSlug: null as string | null,        // slug whose order form is open
  orderFormSide: 'sell' as 'sell' | 'buy',     // which side the form is for
  pendingOrderIds: new Set<string>(),           // per-row mutation lock
  editingOrderId: null as string | null,        // singleton inline edit form

  // Subscription alert signature tracking
  _lastAlertSig: '' as string,
  _refreshAlertsCb: null as (() => void) | null,
};

// ── Constants ──
export const CIRCUIT_CYCLE = '双衍王境';

// ── Format remaining time ──
export function fmtRemain(ms: number): string {
  if (ms <= 0) return '切换中';
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${String(m).padStart(2, '0')}m ${String(sec).padStart(2, '0')}s`;
  if (m > 0) return `${m}m ${String(sec).padStart(2, '0')}s`;
  return `${sec}s`;
}

/** Return the display name for an item based on current market language. */
export function marketName(item: { name: string; name_zh?: string }): string {
  if (S.marketLang === 'zh' && item.name_zh) return item.name_zh;
  return item.name;
}

/** Update dropdown content from current auth state. */
export function updateAuthDropdown() {
  const dd = document.getElementById('auth-dropdown')!;
  if (!S.marketAuthName) { dd.classList.add('hidden'); return; }
  const avatarEl = document.getElementById('auth-avatar') as HTMLImageElement;
  if (S.marketAuthAvatar) {
    avatarEl.src = S.marketAuthAvatar;
    avatarEl.style.display = '';
  } else {
    avatarEl.style.display = 'none';
  }
  document.getElementById('auth-name')!.textContent = S.marketAuthName;
  const repEl = document.getElementById('auth-rep')!;
  if (S.marketAuthRep != null) {
    repEl.textContent = `声望 ${S.marketAuthRep >= 0 ? '+' : ''}${S.marketAuthRep}`;
    repEl.style.display = '';
  } else {
    repEl.style.display = 'none';
  }
  // Pre-select the current status radio.
  if (S.marketAuthStatus) {
    const radio = document.querySelector(`input[name="wm-status"][value="${S.marketAuthStatus}"]`) as HTMLInputElement | null;
    if (radio) radio.checked = true;
  }
}

export function closeAuthDropdown() {
  document.getElementById('auth-dropdown')!.classList.add('hidden');
}

export function rarityCls(r: string): string {
  switch (r) {
    case 'Common': return 'rk-common';
    case 'Uncommon': return 'rk-uncommon';
    case 'Rare': return 'rk-rare';
    case 'Legendary': return 'rk-legendary';
    default: return 'rk-common';
  }
}

export function statusLabel(s: string): string {
  switch (s) {
    case 'ingame': return '<span class="status-text ingame">游戏中</span>';
    case 'online': return '<span class="status-text online">在线</span>';
    default: return '<span class="status-text offline">离线</span>';
  }
}

export function repClass(rep: number): string {
  if (rep >= 20) return 'rep-high';
  if (rep >= 10) return 'rep-mid';
  if (rep >= 0) return 'rep-low';
  return 'rep-neg';
}

export function copyWhisper(playerName: string, itemName: string, platinum: number, orderType: string): void {
  const verb = orderType === 'sell' ? 'buy' : 'sell';
  const msg = `/w ${playerName} Hi! I want to ${verb}: "${itemName}" for ${platinum} platinum. (warframe.market)`;
  navigator.clipboard.writeText(msg).then(() => {
    // brief visual feedback
  }).catch(() => {
    // fallback: select the text so user can Ctrl+C
    const ta = document.createElement('textarea');
    ta.value = msg;
    ta.style.position = 'fixed';
    ta.style.left = '-9999px';
    document.body.appendChild(ta);
    ta.select();
    document.execCommand('copy');
    document.body.removeChild(ta);
  });
}

export function selOpts(options: string[], labels: Record<string, string>, current: string, cls: string): string {
  return `<select class="${cls}">${options.map(o =>
    `<option value="${o}"${o === current ? ' selected' : ''}>${labels[o] ?? (o || '任意任务')}</option>`
  ).join('')}</select>`;
}

export function safeBaroId(location: string): string {
  return location.replace(/[^a-zA-Z0-9一-鿿]/g, '_');
}

// Programmatic tab activation (shared by click handlers and popup click-through).
export function activateTab(tab: string) {
  const btn = document.querySelector(`.tab-btn[data-tab="${tab}"]`) as HTMLButtonElement | null;
  if (!btn || btn.classList.contains('locked') || btn.disabled) return;
  document.querySelectorAll('.tab-btn, .tab-content').forEach(e => e.classList.remove('active'));
  btn.classList.add('active');
  document.getElementById(`tab-${tab}`)!.classList.add('active');
}

export function activateSubTab(sub: string) {
  const btn = document.querySelector(`.sub-tab-btn[data-sub="${sub}"]`) as HTMLButtonElement | null;
  if (!btn) return;
  document.querySelectorAll('.sub-tab-btn').forEach(e => e.classList.remove('active'));
  btn.classList.add('active');
  S.currentSubTab = sub;
}

// Scroll the fissure row for `node` into view and pulse it briefly. Runs after
// the next paint so the freshly-rendered rows exist.
export function highlightFissureRow(node: string) {
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
