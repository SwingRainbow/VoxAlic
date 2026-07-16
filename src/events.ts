import {
  type AppStatePayload,
  S,
} from './state';
import { renderCycles, renderBountyPanel, renderCircuitPanel, renderBaro, renderArbitration } from './tabs/cycles';
import { renderFissures, updateFilters } from './tabs/fissures';
import { renderTimer } from './tabs/timer';
import { availableMissionTypes } from './tabs/settings';

export function handleUpdate(payload: AppStatePayload) {
  S.currentData = payload;
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
  if (sig !== S._lastAlertSig) {
    S._lastAlertSig = sig;
    S._refreshAlertsCb?.();
  }
}

// ── Per-second tick handler: update only text nodes that carry live
//     countdowns (cycle cards, fissure remain cells, bounty/circuit
//     headers). Never rebuilds innerHTML — that only happens on
//     worldstate-update via handleUpdate.
export function handleTickUpdate(payload: AppStatePayload) {
  S.currentData = payload;

  // Cycle cards — patch .card-time text by data-cycle name.
  document.querySelectorAll<HTMLElement>('.cycle-card').forEach(card => {
    const name = card.dataset.cycle;
    if (!name) return;
    const cycle = payload.cycles.find(c => c.name === name);
    if (cycle) {
      const el = card.querySelector('.card-time');
      if (el) el.textContent = `剩余 ${cycle.remain_str}`;
    }
  });

  // Fissure table — patch .fissure-remain cells, toggle .expiring class.
  const allFissures = new Map(
    [...payload.normal_fissures, ...payload.hard_fissures, ...payload.storm_fissures]
      .map(f => [f.node_key, f])
  );
  document.querySelectorAll<HTMLElement>('#fissure-table tbody tr').forEach(row => {
    const key = row.dataset.nodeKey;
    if (!key) return;
    const f = allFissures.get(key);
    if (f) {
      const cell = row.querySelector('.fissure-remain');
      if (cell) cell.textContent = f.remain_str;
      row.classList.toggle('expiring', f.is_expiring);
    }
  });

  // Bounty panel countdown headers (if a board is open).
  if (S.openBounty) {
    const visible = payload.bounties.filter(x => x.card === S.openBounty);
    document.querySelectorAll<HTMLElement>('.bounty-section .bch-count').forEach((el, i) => {
      if (i < visible.length) el.textContent = visible[i].remain_str;
    });
  }

  // Circuit panel countdown header (if open).
  if (S.openCircuit && payload.circuit) {
    const cd = document.querySelector<HTMLElement>('.circuit-head .bch-count');
    if (cd) cd.textContent = `${payload.circuit.remain_str}后刷新`;
  }

  // Baro / Arbitration / Timer — these already have fast paths that only
  // touch textContent when the structural signature hasn't changed.
  renderBaro(payload.baro);
  renderArbitration(payload.arbitration);
  renderTimer(payload.mission_timer);

  // "更新于" watermark.
  const updatedEl = document.getElementById('cycles-updated');
  if (updatedEl) updatedEl.textContent = `更新于 ${payload.last_update}`;
}
