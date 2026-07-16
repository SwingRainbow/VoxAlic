import {
  type Fissure,
  S,
  TIER_BG, TIER_FG,
} from '../state';

// Shared fissure filter: drop expired, then apply tier/type selections.
export function filterFissures(list: Fissure[], tier: string, type: string): Fissure[] {
  return list.filter(f => {
    if (f.remain_ms <= 0) return false;
    if (tier && f.tier_label !== tier) return false;
    if (type && f.mission_type !== type) return false;
    return true;
  });
}

// Does this fissure match any active 裂缝 subscription rule? Mirrors the backend
// `check_fissure_alerts` matching so the list highlight reflects what's notified.
export function fissureSubscribed(f: Fissure): boolean {
  const alerts = S.currentConfig?.fissure_alerts ?? [];
  if (!alerts.length) return false;
  const diff = f.is_hard ? 'hard' : f.is_storm ? 'storm' : 'normal';
  return alerts.some(a =>
    (!a.tier || a.tier === f.tier_label) &&
    (!a.mission_type || a.mission_type === f.mission_type) &&
    (!a.difficulty || a.difficulty === diff));
}

export function getFilteredFissures(): Fissure[] {
  if (!S.currentData) return [];
  const tier = (document.getElementById('tier-filter') as HTMLSelectElement).value;
  const type = (document.getElementById('type-filter') as HTMLSelectElement).value;
  let list: Fissure[];
  if (S.currentSubTab === 'normal') list = S.currentData.normal_fissures;
  else if (S.currentSubTab === 'hard') list = S.currentData.hard_fissures;
  else list = S.currentData.storm_fissures;
  return filterFissures(list, tier, type);
}

export function renderFissures() {
  if (!S.currentData) return;
  const filtered = getFilteredFissures();
  const tbody = document.querySelector('#fissure-table tbody')!;
  tbody.innerHTML = filtered.map(f => {
    const sub = fissureSubscribed(f);
    const cls = `${f.is_expiring ? 'expiring' : ''}${sub ? ' subscribed' : ''}`.trim();
    return `
    <tr class="${cls}" data-node="${f.node_name}" data-node-key="${f.node_key}" style="background:${TIER_BG[f.tier_key] || '#252525'};color:${TIER_FG};">
      <td>${sub ? '<span class="sub-bell">🔔</span>' : ''}<img src="/relics/${f.tier_key}.png" class="relic-icon" alt=""> ${f.tier_label}</td>
      <td>${f.node_name}</td>
      <td>${f.planet}</td>
      <td>${f.mission_type}</td>
      <td class="fissure-remain">${f.remain_str}</td>
    </tr>`;
  }).join('');

  // Update counts with current filters applied
  const tier = (document.getElementById('tier-filter') as HTMLSelectElement).value;
  const type = (document.getElementById('type-filter') as HTMLSelectElement).value;
  document.getElementById('count-normal')!.textContent = String(filterFissures(S.currentData.normal_fissures, tier, type).length);
  document.getElementById('count-hard')!.textContent = String(filterFissures(S.currentData.hard_fissures, tier, type).length);
  document.getElementById('count-storm')!.textContent = String(filterFissures(S.currentData.storm_fissures, tier, type).length);
}

export function updateFilters() {
  if (!S.currentData) return;
  const allFissures = [...S.currentData.normal_fissures, ...S.currentData.hard_fissures];
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
