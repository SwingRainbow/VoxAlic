import {
  type FissureAlert, type CycleAlert, type ArbitrationAlert,
  S, selOpts,
} from '../state';
import { invoke } from '@tauri-apps/api/core';
import { renderFissures } from './fissures';

// ── Subscription (alert rules) management ──────────────────────────────────

export const FISSURE_TIERS = ['', '古纪', '前纪', '中纪', '后纪', '安魂', '全能'];
export const FISSURE_TIER_LABELS: Record<string, string> = {
  '': '任意遗物',
};
export const FISSURE_DIFFICULTIES = ['', 'normal', 'hard', 'storm'];
export const FISSURE_DIFF_LABELS: Record<string, string> = {
  '': '任意难度', 'normal': '普通', 'hard': '钢铁之路', 'storm': '虚空风暴',
};

export const CYCLE_LOCATIONS: Record<string, string[]> = {
  '夜灵平野': ['白天', '黑夜'],
  '奥布山谷': ['温暖', '寒冷'],
  '魔胎之境': ['Fass', 'Vome'],
  '扎里曼': ['Corpus', 'Grineer'],
  '双衍王境': ['悲伤', '恐惧', '喜悦', '愤怒', '嫉妒'],
};

// Derive available mission types from current worldstate fissure data.
export function availableMissionTypes(): string[] {
  if (!S.currentData) return [''];
  const all = [
    ...S.currentData.normal_fissures,
    ...S.currentData.hard_fissures,
    ...S.currentData.storm_fissures,
  ];
  const types = [...new Set(all.map(f => f.mission_type).filter(t => t && t !== '--'))].sort();
  return ['', ...types];
}

// Derive available arbitration mission types and nodes from current + upcoming slots.
// Use the full arbitration node catalogue (all_missions / all_planets from Rust)
// so the alert-rule dropdown shows every possible type, not just the 4 slots
// currently visible.
export function availableArbMissions(): string[] {
  if (!S.currentData?.arbitration?.all_missions?.length) return [''];
  return ['', ...S.currentData.arbitration.all_missions];
}

export function availableArbPlanets(): string[] {
  if (!S.currentData?.arbitration?.all_planets?.length) return [''];
  return ['', ...S.currentData.arbitration.all_planets];
}

export function renderArbitrationAlerts(list: ArbitrationAlert[], container: HTMLElement) {
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

export function renderFissureAlerts(list: FissureAlert[], container: HTMLElement) {
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

export function renderCycleAlerts(list: CycleAlert[], container: HTMLElement) {
  if (!list.length) {
    container.innerHTML = '<div class="alert-rules-empty">暂无规则，点击「+ 添加」开始订阅</div>';
    return;
  }
  const locations = Object.keys(CYCLE_LOCATIONS);
  const advanceOpts = [
    { v: 0, label: '切换时' },
    { v: 5, label: '提前 5 分钟' },
    { v: 10, label: '提前 10 分钟' },
    { v: 15, label: '提前 15 分钟' },
  ];
  container.innerHTML = list.map((a, i) => {
    const states = CYCLE_LOCATIONS[a.location] ?? CYCLE_LOCATIONS[locations[0]];
    const locSel = `<select class="rule-sel loc-sel">${locations.map(l =>
      `<option value="${l}"${l === a.location ? ' selected' : ''}>${l}</option>`
    ).join('')}</select>`;
    const stateSel = `<select class="rule-sel state-sel">${states.map(s =>
      `<option value="${s}"${s === a.state ? ' selected' : ''}>${s}</option>`
    ).join('')}</select>`;
    // Advance-notice dropdown — only meaningful for 夜灵平野 for now.
    const advanceSel = a.location === '夜灵平野'
      ? `<select class="rule-sel advance-sel">${advanceOpts.map(o =>
          `<option value="${o.v}"${o.v === (a.advance_minutes || 0) ? ' selected' : ''}>${o.label}</option>`
        ).join('')}</select>`
      : '';
    return `<div class="alert-rule-row" data-index="${i}">${locSel}进入${stateSel}${advanceSel}<button class="rule-del timer-btn-sm" data-index="${i}">删除</button></div>`;
  }).join('');
}

export function saveAlerts() {
  if (!S.currentConfig) return;
  invoke('set_config', { config: S.currentConfig });
  // Refresh the fissure list so subscription highlights track the rule change.
  if (S.currentData) renderFissures();
}

export function setupAlerts(): () => void {
  const fissureList = document.getElementById('fissure-alerts-list')!;
  const cycleList = document.getElementById('cycle-alerts-list')!;
  const arbList = document.getElementById('arb-alerts-list')!;

  function refresh() {
    if (!S.currentConfig) return;
    renderFissureAlerts(S.currentConfig.fissure_alerts, fissureList);
    renderCycleAlerts(S.currentConfig.cycle_alerts, cycleList);
    renderArbitrationAlerts(S.currentConfig.arbitration_alerts, arbList);
  }

  document.getElementById('btn-add-fissure-alert')!.addEventListener('click', () => {
    if (!S.currentConfig) return;
    const missions = availableMissionTypes();
    const defaultMission = missions.find(m => m !== '') ?? '';
    S.currentConfig.fissure_alerts = [...S.currentConfig.fissure_alerts, { tier: '', mission_type: defaultMission, difficulty: '' }];
    refresh();
    saveAlerts();
  });

  document.getElementById('btn-add-cycle-alert')!.addEventListener('click', () => {
    if (!S.currentConfig) return;
    const loc = '夜灵平野';
    S.currentConfig.cycle_alerts = [...S.currentConfig.cycle_alerts, { location: loc, state: CYCLE_LOCATIONS[loc][0], advance_minutes: 0 }];
    refresh();
    saveAlerts();
  });

  document.getElementById('btn-add-arb-alert')!.addEventListener('click', () => {
    if (!S.currentConfig) return;
    S.currentConfig.arbitration_alerts = [...S.currentConfig.arbitration_alerts, { mission_type: '', planet: '' }];
    refresh();
    saveAlerts();
  });

  fissureList.addEventListener('change', (e) => {
    if (!S.currentConfig) return;
    const row = (e.target as HTMLElement).closest('.alert-rule-row') as HTMLElement | null;
    if (!row) return;
    const i = parseInt(row.dataset.index!);
    const rule = S.currentConfig.fissure_alerts[i];
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
    if (!S.currentConfig || !(e.target as HTMLElement).classList.contains('rule-del')) return;
    const i = parseInt((e.target as HTMLElement).dataset.index!);
    S.currentConfig.fissure_alerts = S.currentConfig.fissure_alerts.filter((_, idx) => idx !== i);
    refresh();
    saveAlerts();
  });

  cycleList.addEventListener('change', (e) => {
    if (!S.currentConfig) return;
    const row = (e.target as HTMLElement).closest('.alert-rule-row') as HTMLElement | null;
    if (!row) return;
    const i = parseInt(row.dataset.index!);
    const rule = S.currentConfig.cycle_alerts[i];
    if ((e.target as HTMLElement).classList.contains('loc-sel')) {
      rule.location = (e.target as HTMLSelectElement).value;
      rule.state = CYCLE_LOCATIONS[rule.location][0];
      // Reset advance when switching away from 夜灵平野 (only Plains supports it).
      if (rule.location !== '夜灵平野') rule.advance_minutes = 0;
      refresh();
    } else if ((e.target as HTMLElement).classList.contains('state-sel')) {
      rule.state = (e.target as HTMLSelectElement).value;
    } else if ((e.target as HTMLElement).classList.contains('advance-sel')) {
      rule.advance_minutes = parseInt((e.target as HTMLSelectElement).value);
    }
    saveAlerts();
  });

  cycleList.addEventListener('click', (e) => {
    if (!S.currentConfig || !(e.target as HTMLElement).classList.contains('rule-del')) return;
    const i = parseInt((e.target as HTMLElement).dataset.index!);
    S.currentConfig.cycle_alerts = S.currentConfig.cycle_alerts.filter((_, idx) => idx !== i);
    refresh();
    saveAlerts();
  });

  arbList.addEventListener('change', (e) => {
    if (!S.currentConfig) return;
    const row = (e.target as HTMLElement).closest('.alert-rule-row') as HTMLElement | null;
    if (!row) return;
    const i = parseInt(row.dataset.index!);
    const rule = S.currentConfig.arbitration_alerts[i];
    if ((e.target as HTMLElement).classList.contains('arb-mission-sel'))
      rule.mission_type = (e.target as HTMLSelectElement).value;
    else if ((e.target as HTMLElement).classList.contains('arb-planet-sel'))
      rule.planet = (e.target as HTMLSelectElement).value;
    saveAlerts();
  });

  arbList.addEventListener('click', (e) => {
    if (!S.currentConfig || !(e.target as HTMLElement).classList.contains('rule-del')) return;
    const i = parseInt((e.target as HTMLElement).dataset.index!);
    S.currentConfig.arbitration_alerts = S.currentConfig.arbitration_alerts.filter((_, idx) => idx !== i);
    refresh();
    saveAlerts();
  });

  return refresh;
}
