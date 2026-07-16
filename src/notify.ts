import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

// Mirrors the Rust `SubNotify` struct (lib.rs).
interface SubNotify {
  kind: string;    // "fissure" | "cycle" | "arbitration"
  icon: string;
  title: string;
  detail: string;
  ts: number;      // fired-at (ms)
  node: string;    // locate key for click-through
  sub: string;     // fissure sub-tab hint
}

const listEl = () => document.getElementById('np-list')!;

function esc(s: string): string {
  return s.replace(/[&<>"]/g, c =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]!));
}

function relTime(ts: number): string {
  const diff = Math.max(0, Date.now() - ts);
  const m = Math.floor(diff / 60000);
  if (m < 1) return '刚刚';
  if (m < 60) return `${m} 分钟前`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} 小时前`;
  return `${Math.floor(h / 24)} 天前`;
}

function render(list: SubNotify[]) {
  const el = listEl();
  if (!list.length) {
    el.innerHTML = '<div class="np-empty">暂无提醒</div>';
    return;
  }
  el.innerHTML = list.map((n, i) => `
    <div class="np-item np-${esc(n.kind)}" style="--ni:${i}" data-kind="${esc(n.kind)}" data-node="${esc(n.node)}" data-sub="${esc(n.sub)}">
      ${n.icon ? `<div class="np-icon">${esc(n.icon)}</div>` : ''}
      <div class="np-body">
        <div class="np-title">${esc(n.title)}</div>
        <div class="np-detail">${esc(n.detail)}</div>
      </div>
      <div class="np-time">${relTime(n.ts)}</div>
    </div>`).join('');
}

async function init() {
  // Register the push listener FIRST. The notify window's webview can execute
  // this script before `app.manage(notify_list)` runs at the end of the Rust
  // `setup` closure; if the initial `get_notifications` invoke loses that race
  // it rejects, and doing it before `listen` would abort init() and leave the
  // window permanently without a `sub-notify` listener (empty popup forever,
  // even though the tray flashes and re-emits on hover). Listener-first makes
  // the tray-hover re-emit a reliable safety net regardless of startup timing.
  await listen<SubNotify[]>('sub-notify', e => render(e.payload));
  document.getElementById('np-clear')!.addEventListener('click', async () => {
    await invoke('clear_notifications');
    render([]);
  });
  // Auto-hide is owned by the Rust cursor-poll watcher (start_popup_watch); the
  // popup no longer needs to report hover via DOM events.
  // Click an item → raise the main window and navigate to that entry.
  listEl().addEventListener('click', e => {
    const item = (e.target as HTMLElement).closest('.np-item') as HTMLElement | null;
    if (!item) return;
    invoke('open_main_navigate', {
      kind: item.dataset.kind || '',
      node: item.dataset.node || '',
      sub: item.dataset.sub || '',
    });
  });
  try {
    render(await invoke<SubNotify[]>('get_notifications'));
  } catch {
    // State not managed yet (startup race) — show the empty placeholder; the
    // next push / tray-hover re-emit will populate the list.
    render([]);
  }
}

init();
