import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

// Mirrors the Rust `SubNotify` struct (lib.rs).
interface SubNotify {
  kind: string;    // "fissure" | "cycle" | "arbitration"
  icon: string;
  title: string;
  detail: string;
  ts: number;      // fired-at (ms)
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
  el.innerHTML = list.map(n => `
    <div class="np-item np-${esc(n.kind)}">
      <div class="np-icon">${esc(n.icon)}</div>
      <div class="np-body">
        <div class="np-title">${esc(n.title)}</div>
        <div class="np-detail">${esc(n.detail)}</div>
      </div>
      <div class="np-time">${relTime(n.ts)}</div>
    </div>`).join('');
}

async function init() {
  render(await invoke<SubNotify[]>('get_notifications'));
  await listen<SubNotify[]>('sub-notify', e => render(e.payload));
  document.getElementById('np-clear')!.addEventListener('click', async () => {
    await invoke('clear_notifications');
    render([]);
  });
}

init();
