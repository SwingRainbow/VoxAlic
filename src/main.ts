import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
// import { openUrl } kept as dependency-free backup — no longer used; remove when confirmed

import {
  type AppStatePayload, type AppConfig, type MarketItemSummary,
  type MarketCacheStatus, type MarketAuthStatus,
  S,
  activateTab, activateSubTab, highlightFissureRow,
  CIRCUIT_CYCLE, updateAuthDropdown, closeAuthDropdown,
} from './state';
import { handleUpdate, handleTickUpdate } from './events';
import { renderCycles, renderBountyPanel, renderCircuitPanel, renderBaro, renderArbitration } from './tabs/cycles';
import { renderFissures } from './tabs/fissures';
import { updateTimerConfig, setupCalibration } from './tabs/timer';
import {
  renderMarketResults, renderMarketDetail,
  updateMarketStatus, updateAuthChip, handleAuthExpired,
  initMarketAuth, refreshMyOrders, closeMarketDetail,
  doMarketSearch, openMarketItem, marketInvoke,
} from './tabs/market';
import {
  setupAlerts,
} from './tabs/settings';

// ── Event listeners ──
window.addEventListener('DOMContentLoaded', () => {
  // Disable the webview right-click context menu.
  document.addEventListener('contextmenu', e => e.preventDefault());
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
      S.openCircuit = !S.openCircuit;
      S.openBounty = null;  // keep only one panel open
    } else {
      S.openBounty = S.openBounty === name ? null : name;
      S.openCircuit = false;
    }
    renderBountyPanel(S.currentData?.bounties ?? []);
    renderCircuitPanel(S.currentData?.circuit ?? null);
    renderCycles(S.currentData?.cycles ?? []);
  });

  // Sub-tab switching
  document.querySelectorAll('.sub-tab-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      activateSubTab((btn as HTMLElement).dataset.sub!);
      renderFissures();
    });
  });

  // Refresh button
  const refreshBtn = document.getElementById('btn-refresh') as HTMLButtonElement;
  refreshBtn.addEventListener('click', () => {
    refreshBtn.disabled = true;
    refreshBtn.textContent = '刷新中…';
    invoke('refresh_now')
      .then(() => {
        refreshBtn.textContent = '✅ 刷新成功';
        setTimeout(() => { refreshBtn.textContent = '刷新数据'; refreshBtn.disabled = false; }, 2000);
      })
      .catch(err => {
        refreshBtn.textContent = String(err).slice(0, 20);
        setTimeout(() => { refreshBtn.textContent = '刷新数据'; refreshBtn.disabled = false; }, 3000);
      });
  });

  // Filters
  document.getElementById('tier-filter')!.addEventListener('change', renderFissures);
  document.getElementById('type-filter')!.addEventListener('change', renderFissures);

  // Settings: load config
  const closeToggle = document.getElementById('setting-close-to-tray') as HTMLInputElement;
  invoke<AppConfig>('get_config').then(cfg => {
    S.currentConfig = cfg;
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
    // Init worldstate-source radio (default "official" if unset)
    document.querySelectorAll<HTMLInputElement>('input[name="worldstate-source"]').forEach(r => {
      r.checked = r.value === (cfg.worldstate_source || 'official');
    });
    // Init update-source radio (default "gitee" if unset)
    document.querySelectorAll<HTMLInputElement>('input[name="update-source"]').forEach(r => {
      r.checked = r.value === (cfg.update_source || 'gitee');
    });
    // Init market language radio (default "en" if unset)
    S.marketLang = cfg.market_language || 'en';
    document.querySelectorAll<HTMLInputElement>('input[name="market-language"]').forEach(r => {
      r.checked = r.value === S.marketLang;
    });
    // Init custom reminder text inputs
    (document.getElementById('checkpoint-text') as HTMLInputElement).value = mt.checkpoint_alert_text ?? '';
    (document.getElementById('hp-text') as HTMLInputElement).value = mt.hp_alert_text ?? '';
    // Init mode radio from saved config
    document.querySelectorAll<HTMLInputElement>('input[name="timer-mode"]').forEach(r => {
      r.checked = r.value === mt.mode;
    });
    // Init hotkey display
    const hotkeyInput = document.getElementById('hotkey-input') as HTMLInputElement;
    const hotkeyStatus = document.getElementById('hotkey-status') as HTMLSpanElement;
    hotkeyInput.value = cfg.hotkey ?? '';

    // Init subscription rules
    refreshAlerts();

    // ── Hotkey capture ──
    hotkeyInput.addEventListener('focus', () => {
      hotkeyInput.value = '';
      hotkeyStatus.textContent = '';
    });

    hotkeyInput.addEventListener('keydown', (e) => {
      e.preventDefault();
      const parts: string[] = [];
      if (e.ctrlKey) parts.push('Ctrl');
      if (e.altKey) parts.push('Alt');
      if (e.shiftKey) parts.push('Shift');
      if (e.metaKey) parts.push('Meta');
      // Map key name to display
      let key = e.key;
      if (key === ' ') key = 'Space';
      else if (key.length === 1) key = key.toUpperCase();
      else if (key.startsWith('F') && /^F\d+$/.test(key)) { /* keep as-is */ }
      else if (key === 'Escape') key = 'Esc';
      else if (key === 'Backslash') { /* keep — Rust parses both \ and Backslash */ }
      else if (key === 'Backspace') { /* ignore - just modifiers shown */ return; }
      // Skip standalone modifier keys
      if (['Control','Alt','Shift','Meta'].includes(key)) return;
      parts.push(key);
      hotkeyInput.value = parts.join('+');
      hotkeyStatus.textContent = '';
    });

    document.getElementById('btn-hotkey-save')!.addEventListener('click', async () => {
      const val = hotkeyInput.value.trim();
      if (!val) { hotkeyStatus.textContent = '请先输入组合键'; return; }
      try {
        await invoke('set_hotkey', { hotkey: val });
        if (!val.includes('+')) {
          hotkeyStatus.textContent = '✅ 已设置（单键热键极易冲突，注意检查）';
        } else {
          hotkeyStatus.textContent = '✅ 已设置';
        }
        if (S.currentConfig) S.currentConfig.hotkey = val;
      } catch (err: any) {
        hotkeyStatus.textContent = `⚠ ${String(err)}`;
        hotkeyInput.value = S.currentConfig?.hotkey ?? '';
      }
    });

    document.getElementById('btn-hotkey-clear')!.addEventListener('click', async () => {
      try {
        await invoke('set_hotkey', { hotkey: null });
        hotkeyInput.value = '';
        hotkeyStatus.textContent = '✅ 已清除';
        if (S.currentConfig) S.currentConfig.hotkey = null;
      } catch (err: any) {
        hotkeyStatus.textContent = `⚠ ${String(err)}`;
      }
    });
  }).catch((err: unknown) => {
    console.error('get_config 失败:', err);
  });

  const refreshAlerts = setupAlerts();
  S._refreshAlertsCb = refreshAlerts;

  // Autostart toggle: read current registry state on init, write on change
  const autostartToggle = document.getElementById('setting-autostart') as HTMLInputElement;
  invoke<boolean>('get_autostart').then(v => { autostartToggle.checked = v; });
  autostartToggle.addEventListener('change', () => {
    invoke('set_autostart', { enabled: autostartToggle.checked });
  });

  // ── Phone notification (Bark) ────────────────────────────────────────────

  const barkInput = document.getElementById('setting-bark-url') as HTMLInputElement;
  const barkStatus = document.getElementById('bark-status') as HTMLSpanElement;
  const barkModalStatus = document.getElementById('bark-modal-status') as HTMLSpanElement;
  const barkSaveBtn = document.getElementById('btn-save-bark-url') as HTMLButtonElement;
  const barkModal = document.getElementById('bark-config-modal')!;

  function updateBarkStatus() {
    const url = barkInput.value.trim();
    barkStatus.textContent = url ? '✅ 已配置' : '';
  }

  invoke<string>('get_bark_url').then(url => {
    if (url) {
      barkInput.value = url;
      updateBarkStatus();
    }
  });

  document.getElementById('btn-open-bark-modal')!.addEventListener('click', () => {
    barkModalStatus.textContent = '';
    barkModal.classList.remove('hidden');
  });

  document.getElementById('btn-close-bark-modal')!.addEventListener('click', () => {
    barkModal.classList.add('hidden');
  });

  barkSaveBtn.addEventListener('click', () => {
    if (!S.currentConfig) return;
    const url = barkInput.value.trim();
    const newCfg = { ...S.currentConfig, notify_bark_url: url };
    S.currentConfig = newCfg;
    invoke('set_config', { config: newCfg }).then(() => {
      barkModalStatus.textContent = url ? '✅ 已保存' : '已清空';
      updateBarkStatus();
      setTimeout(() => { barkModalStatus.textContent = ''; }, 2500);
    }).catch(err => {
      barkModalStatus.textContent = String(err).slice(0, 30);
    });
  });

  document.getElementById('btn-test-bark')!.addEventListener('click', () => {
    barkModalStatus.textContent = '发送中…';
    invoke<string>('test_phone_push').then(msg => {
      barkModalStatus.textContent = msg;
      setTimeout(() => { barkModalStatus.textContent = ''; }, 3000);
    }).catch(err => {
      barkModalStatus.textContent = String(err).slice(0, 30);
    });
  });

  // Settings: save on change
  closeToggle.addEventListener('change', () => {
    if (!S.currentConfig) return;
    const newCfg = { ...S.currentConfig, close_to_tray: closeToggle.checked };
    S.currentConfig = newCfg;
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
    if (!S.currentConfig) return;
    const newCfg = {
      ...S.currentConfig,
      mission_timer: { ...S.currentConfig.mission_timer, ocr_interval_secs: val }
    };
    S.currentConfig = newCfg;
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

  // Worldstate data source: official vs mirror — switch + refresh
  const wsSourceStatus = document.getElementById('worldstate-source-status')!;
  document.querySelectorAll('input[name="worldstate-source"]').forEach(radio => {
    radio.addEventListener('change', function(this: HTMLInputElement) {
      if (!this.checked || !S.currentConfig) return;
      const newCfg = { ...S.currentConfig, worldstate_source: this.value };
      S.currentConfig = newCfg;
      invoke('set_config', { config: newCfg }).then(() => {
        wsSourceStatus.textContent = '已切换 · 正在刷新…';
        return invoke('refresh_now');
      }).then(() => {
        wsSourceStatus.textContent = '✅ 刷新成功';
        setTimeout(() => { wsSourceStatus.textContent = ''; }, 3000);
      }).catch(err => {
        wsSourceStatus.textContent = String(err);
        setTimeout(() => { wsSourceStatus.textContent = ''; }, 5000);
      });
    });
  });

  // Update source: persist radio choice to config.
  document.querySelectorAll('input[name="update-source"]').forEach(radio => {
    radio.addEventListener('change', function(this: HTMLInputElement) {
      if (!this.checked || !S.currentConfig) return;
      S.currentConfig = { ...S.currentConfig, update_source: this.value };
      invoke('set_config', { config: S.currentConfig });
    });
  });

  // Market language: persist + re-render current view
  document.querySelectorAll('input[name="market-language"]').forEach(radio => {
    radio.addEventListener('change', function(this: HTMLInputElement) {
      if (!this.checked || !S.currentConfig) return;
      S.marketLang = this.value;
      S.currentConfig = { ...S.currentConfig, market_language: this.value };
      invoke('set_config', { config: S.currentConfig });
      // Re-render current view if any (no API re-fetch)
      if (S.marketOpenSlug && S._lastMarketDetail) {
        renderMarketDetail(S._lastMarketDetail);
      }
      // Re-render search results from cache
      if (S._lastMarketResults.length) {
        renderMarketResults(S._lastMarketResults);
      }
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
    document.getElementById('update-cur-version')!.textContent = `v${v}`;
  });
  const updateSource = (): string =>
    (document.querySelector('input[name="update-source"]:checked') as HTMLInputElement | null)?.value ?? 'gitee';

  // Shared handler: show the update modal with version + notes.
  function showUpdateModal(version: string, notes: string) {
    document.getElementById('update-modal-version')!.textContent = `最新版本：${version}`;
    document.getElementById('update-modal-notes')!.textContent = notes;
    updateModalStatus.textContent = '';
    confirmUpdateBtn.disabled = false;
    updateModal.classList.remove('hidden');
  }

  // Auto-update check fires on startup.
  listen<{ version: string; notes: string }>('update-available', (event) => {
    showUpdateModal(event.payload.version, event.payload.notes);
  });
  checkUpdateBtn.addEventListener('click', () => {
    checkUpdateBtn.disabled = true;
    updateStatus.textContent = '检查中…';
    invoke<{ version: string; notes: string } | null>('check_for_update', { source: updateSource() })
      .then(info => {
        if (info) {
          updateStatus.textContent = '';
          showUpdateModal(info.version, info.notes);
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
  let alertTestTimer: ReturnType<typeof setTimeout> | null = null;
  document.getElementById('btn-test-alert')!.addEventListener('click', () => {
    if (alertTestTimer) { clearTimeout(alertTestTimer); alertTestTimer = null; }
    alertTestStatus.textContent = '测试中…';
    invoke('test_alert')
      .then(() => {
        alertTestStatus.textContent = '✅ 已触发';
        alertTestTimer = setTimeout(() => { alertTestStatus.textContent = ''; alertTestTimer = null; }, 5000);
      })
      .catch(err => {
        alertTestStatus.textContent = String(err);
        alertTestTimer = setTimeout(() => { alertTestStatus.textContent = ''; alertTestTimer = null; }, 5000);
      });
  });

  // Baro cards: click to expand/collapse items table (only when active).
  document.getElementById('baro-card')!.addEventListener('click', (e) => {
    const panel = (e.target as HTMLElement).closest('.baro-panel') as HTMLElement | null;
    if (!panel) return;
    const loc = panel.dataset.baroLoc;
    if (!loc) return;
    const baro = (S.currentData?.baro ?? []).find(b => b.location === loc);
    if (!baro?.active) return;
    S.openBaro = S.openBaro === loc ? null : loc;
    renderBaro(S.currentData!.baro);
  });

  // Arbitration card: click to expand/collapse upcoming slots
  document.getElementById('arbitration-card')!.addEventListener('click', () => {
    if (S.currentData?.arbitration) {
      S.openArbitration = !S.openArbitration;
      renderArbitration(S.currentData.arbitration);
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

  // ── 启动过渡 ──────────────────────────────────────────────
  let _splashStart = Date.now(), _splashHidden = false;
  const SPLASH_MIN_MS = 2500;
  function _hideSplash() {
    if (_splashHidden) return;
    _splashHidden = true;
    const el = document.getElementById('splash-overlay');
    if (!el) return;
    const elapsed = Date.now() - _splashStart;
    const delay = Math.max(0, SPLASH_MIN_MS - elapsed);
    const fade = () => { el.style.opacity = '0'; setTimeout(() => el.remove(), 350); };
    delay > 0 ? setTimeout(fade, delay) : fade();
  }
  setTimeout(_hideSplash, 10_000);

  // Tauri events
  listen<AppStatePayload>('worldstate-update', (event) => {
    _hideSplash();
    handleUpdate(event.payload);
  });

  listen<AppStatePayload>('tick-update', (event) => {
    handleTickUpdate(event.payload);
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

  // ── Market event listeners ──────────────────────────────────────────
  listen<number>('market-cache-ready', (_event) => {
    invoke<MarketCacheStatus>('market_cache_status').then(updateMarketStatus).catch(() => {});
  });

  // Search input with debounce
  const marketSearch = document.getElementById('market-search') as HTMLInputElement;
  marketSearch.addEventListener('input', () => {
    if (S.marketSearchTimer) clearTimeout(S.marketSearchTimer);
    S.marketSearchTimer = setTimeout(() => {
      doMarketSearch(marketSearch.value);
    }, 200);
  });
  // Suppress search during IME composition
  marketSearch.addEventListener('compositionstart', () => {
    if (S.marketSearchTimer) clearTimeout(S.marketSearchTimer);
  });
  marketSearch.addEventListener('compositionend', () => {
    S.marketSearchTimer = setTimeout(() => {
      doMarketSearch(marketSearch.value);
    }, 200);
  });

  // Click on a search result row → expand detail
  document.getElementById('market-results')!.addEventListener('click', (e) => {
    const row = (e.target as HTMLElement).closest('.market-result-row') as HTMLElement | null;
    if (!row) return;
    const slug = row.dataset.slug!;
    if (S.marketOpenSlug === slug) {
      closeMarketDetail();
    } else {
      openMarketItem(slug);
    }
  });

  // Refresh cache button
  document.getElementById('btn-market-refresh')!.addEventListener('click', async function(this: HTMLButtonElement) {
    this.disabled = true;
    this.textContent = '⏳';
    const statusEl = document.getElementById('market-status-settings')!;
    statusEl.textContent = '更新中…';
    try {
      await invoke<number>('refresh_market_cache');
      const status = await invoke<MarketCacheStatus>('market_cache_status');
      updateMarketStatus(status);
    } catch (err) {
      statusEl.textContent = `⚠️ ${err}`;
    } finally {
      this.disabled = false;
      this.textContent = '🔄';
    }
  });

  // ── Market auth & orders event listeners ───────────────────────────

  // Register WS listeners BEFORE initMarketAuth so events from the
  // auto-started WebSocket task are not lost.
  listen<string>('market-ws-log', (_event) => {
    // Debug log suppressed in production.
  });

  listen<string>('market-status-change', (event) => {
    S.marketAuthStatus = event.payload;
    const radio = document.querySelector(`input[name="wm-status"][value="${event.payload}"]`) as HTMLInputElement | null;
    if (radio) radio.checked = true;
  });

  // WS connection state → degraded UI until initial status sync completes.
  let wsDisconnectedAt: number | null = null;
  listen<string>('market-ws-state', (event) => {
    const radios = document.getElementById('auth-status-radios');
    const hint = document.getElementById('auth-status-hint');
    if (event.payload === 'ready') {
      // Server confirmed our status — unlock radios.
      wsDisconnectedAt = null;
      if (radios) radios.classList.remove('degraded');
      if (hint) { hint.textContent = ''; hint.classList.remove('visible'); }
    } else if (event.payload === 'connected') {
      // Connected but status not yet confirmed — keep locked, show progress.
      wsDisconnectedAt = null;
      if (hint) { hint.textContent = '已连接，同步状态中…'; hint.classList.add('visible'); }
    } else {
      // Disconnected — degrade after 30s.
      wsDisconnectedAt = Date.now();
      if (hint) { hint.textContent = 'WebSocket 重连中…'; }
      setTimeout(() => {
        if (wsDisconnectedAt && Date.now() - wsDisconnectedAt >= 29_000) {
          if (radios) radios.classList.add('degraded');
          if (hint) hint.classList.add('visible');
        }
      }, 30_000);
    }
  });

  listen<string>('market-spy-result', (event) => {
    const statusEl = document.getElementById('market-login-status');
    if (statusEl) {
      statusEl.textContent = event.payload;
      statusEl.className = 'login-status';
    }
  });

  listen<string>('market-login-phase', (event) => {
    const phaseEl = document.getElementById('login-phase');
    if (phaseEl) phaseEl.textContent = event.payload;
  });

  // Init auth on startup (WS listeners are already registered above).

  initMarketAuth();

  // ── Auth event listeners ─────────────────────────────────────────

  // Close dropdown when clicking outside.
  document.addEventListener('click', (e) => {
    const dd = document.getElementById('auth-dropdown')!;
    const chip = document.getElementById('market-auth-chip')!;
    if (!dd.classList.contains('hidden') && !dd.contains(e.target as Node) && e.target !== chip) {
      closeAuthDropdown();
    }
  });

  // Auth chip → toggle dropdown (logged in) or open modal (not logged in).
  document.getElementById('market-auth-chip')!.addEventListener('click', (e) => {
    e.stopPropagation();
    if (S.marketAuthName) {
      // Toggle dropdown.
      const dd = document.getElementById('auth-dropdown')!;
      dd.classList.toggle('hidden');
      if (!dd.classList.contains('hidden')) updateAuthDropdown();
      return;
    }
    // Not logged in — show login form.
    const modal = document.getElementById('market-login-modal')!;
    modal.classList.remove('hidden');
    (document.getElementById('market-login-email') as HTMLInputElement).value = '';
    (document.getElementById('market-login-password') as HTMLInputElement).value = '';
    const statusEl = document.getElementById('market-login-status')!;
    statusEl.textContent = '';
    statusEl.className = 'login-status';
    document.getElementById('login-spinner')!.classList.add('hidden');
    document.getElementById('login-phase')!.textContent = '';
    document.getElementById('btn-market-logout')!.style.display = 'none';
    document.getElementById('btn-market-login')!.style.display = '';
    (document.getElementById('market-login-password') as HTMLInputElement).type = 'password';
    (document.getElementById('btn-pass-toggle')!).textContent = '👁';
  });

  // Password toggle.
  document.getElementById('btn-pass-toggle')!.addEventListener('click', () => {
    const pw = document.getElementById('market-login-password') as HTMLInputElement;
    const btn = document.getElementById('btn-pass-toggle')!;
    if (pw.type === 'password') { pw.type = 'text'; btn.textContent = '🙈'; }
    else { pw.type = 'password'; btn.textContent = '👁'; }
  });

  // Login modal — cancel
  document.getElementById('btn-market-login-cancel')!.addEventListener('click', () => {
    document.getElementById('market-login-modal')!.classList.add('hidden');
  });

  // Login modal — login (with spinner + phase text)
  document.getElementById('btn-market-login')!.addEventListener('click', async () => {
    const email = (document.getElementById('market-login-email') as HTMLInputElement).value.trim();
    const password = (document.getElementById('market-login-password') as HTMLInputElement).value;
    const statusEl = document.getElementById('market-login-status')!;
    const loginBtn = document.getElementById('btn-market-login') as HTMLButtonElement;
    const spinner = document.getElementById('login-spinner')!;
    const phaseEl = document.getElementById('login-phase')!;

    statusEl.textContent = '';
    statusEl.className = 'login-status';
    if (!email || !email.includes('@')) {
      statusEl.textContent = '请输入有效的邮箱地址';
      statusEl.className = 'login-status error';
      return;
    }
    if (!password) {
      statusEl.textContent = '请输入密码';
      statusEl.className = 'login-status error';
      return;
    }

    loginBtn.disabled = true;
    spinner.classList.remove('hidden');
    phaseEl.textContent = '获取安全令牌…';

    try {
      const result = await marketInvoke<MarketAuthStatus>('market_signin', { email, password });
      S.marketAuthName = result.ingame_name;
      S.marketAuthAvatar = result.avatar ?? null;
      S.marketAuthStatus = result.current_status ?? null;
      S.marketAuthRep = result.reputation ?? null;
      updateAuthChip();
      document.getElementById('market-login-modal')!.classList.add('hidden');
      await refreshMyOrders();
    } catch (err: any) {
      if (err?.code === 'auth_expired') { handleAuthExpired(); return; }
      statusEl.textContent = err?.message || '登录失败';
      statusEl.className = 'login-status error';
    } finally {
      loginBtn.disabled = false;
      spinner.classList.add('hidden');
      phaseEl.textContent = '';
    }
  });

  // Dropdown logout.
  document.getElementById('btn-auth-logout')!.addEventListener('click', async () => {
    try {
      await marketInvoke<void>('market_signout');
      handleAuthExpired();
      closeAuthDropdown();
    } catch (err: any) {
      // Silently clear local state even if API call fails.
      handleAuthExpired();
      closeAuthDropdown();
    }
  });

  // Status radio buttons in dropdown.
  document.getElementById('auth-status-radios')!.addEventListener('change', async (e) => {
    const target = e.target as HTMLInputElement;
    if (target.name !== 'wm-status') return;
    try {
      await marketInvoke<void>('market_set_status', { status: target.value });
      S.marketAuthStatus = target.value; // optimistically update local
    } catch (_) {}
  });

  // My orders panel — refresh on expand
  (document.getElementById('market-my-orders') as HTMLDetailsElement).addEventListener('toggle', function() {
    if (this.open && S.marketAuthName) refreshMyOrders();
  });

  // ── Translation (zh→en) ──────────────────────────────────────────
  let translateTimer: ReturnType<typeof setTimeout> | null = null;
  const translateZh = document.getElementById('translate-zh') as HTMLInputElement;
  const translateResults = document.getElementById('translate-results')!;

  translateZh.addEventListener('input', () => {
    if (translateTimer) clearTimeout(translateTimer);
    const q = translateZh.value.trim();
    if (!q) { translateResults.classList.add('hidden'); translateResults.innerHTML = ''; return; }
    translateTimer = setTimeout(async () => {
      try {
        const items = await invoke<MarketItemSummary[]>('translate_items', { query: q });
        if (!items.length) {
          translateResults.innerHTML = '<div class="translate-empty">无匹配</div>';
        } else {
          translateResults.innerHTML = items.map(i => `
            <div class="translate-row" data-en="${i.name}">
              <span class="translate-en-name">${i.name}</span>
              ${i.name_zh ? `<span class="translate-zh-name">${i.name_zh}</span>` : ''}
            </div>`).join('');
        }
        translateResults.classList.remove('hidden');
      } catch {
        translateResults.innerHTML = '';
        translateResults.classList.add('hidden');
      }
    }, 200);
  });

  // Click translate result → copy English name
  translateResults.addEventListener('click', (e) => {
    const row = (e.target as HTMLElement).closest('.translate-row') as HTMLElement | null;
    if (!row) return;
    const en = row.dataset.en || '';
    navigator.clipboard.writeText(en).then(() => {
      // Briefly highlight
      row.style.background = 'rgba(79,195,247,0.2)';
      setTimeout(() => { row.style.background = ''; }, 300);
    }).catch(() => {});
  });

  // Hide translate results when clicking outside
  document.addEventListener('click', (e) => {
    if (!(e.target as HTMLElement).closest('#translate-wrap')) {
      translateResults.classList.add('hidden');
    }
  });

  // Focus on translate input → re-show results if has text
  translateZh.addEventListener('focus', () => {
    if (translateZh.value.trim() && translateResults.children.length > 0) {
      translateResults.classList.remove('hidden');
    }
  });

  // ── Detail panel: retry button (delegated) ──
  document.getElementById('market-detail')!.addEventListener('click', (e) => {
    if ((e.target as HTMLElement).id === 'btn-market-retry' && S.marketOpenSlug) {
      openMarketItem(S.marketOpenSlug);
    }
  });

  // When switching to market tab, lazily check cache status on first search input focus.
  let marketStatusChecked = false;
  marketSearch.addEventListener('focus', () => {
    if (!marketStatusChecked) {
      marketStatusChecked = true;
      invoke<MarketCacheStatus>('market_cache_status').then(updateMarketStatus).catch(() => {});
    }
    // If detail is open, close it and restore search results
    if (S.marketOpenSlug) {
      closeMarketDetail();
    }
  });

  // ── Back-to-top button: show when scrolled away from top ──
  document.getElementById('tab-market')!.addEventListener('scroll', () => {
    const btn = document.getElementById('btn-market-backtop');
    if (btn) {
      const tab = document.getElementById('tab-market')!;
      btn.classList.toggle('visible', tab.scrollTop > 200);
    }
  });

  // ── First-launch skeletons: show placeholder cards + table rows immediately
  //     so the user knows the app is alive while waiting for worldstate-update.
  (function renderSkeletons() {
    document.getElementById('cycle-cards')!.innerHTML = Array.from({ length: 5 }, () =>
      `<div class="cycle-card-skel">
        <div class="skel-line w60"></div>
        <div class="skel-line w80"></div>
        <div class="skel-line w40"></div>
      </div>`).join('');
    document.querySelector('#fissure-table tbody')!.innerHTML = Array.from({ length: 8 }, () =>
      `<tr>${Array.from({ length: 5 }, () => '<td><div class="skel-line w100"></div></td>').join('')}</tr>`
    ).join('');
    document.getElementById('cycles-updated')!.textContent = '正在获取世界状态…';
  })();

  // ── ROI calibration ──
  setupCalibration();

  // ── Contact: open log folder ──
  (window as any)._openLogFolder = () => { invoke('open_log_folder').catch(() => {}); };

  // ── Feedback modal ──
  const feedbackModal = document.getElementById('feedback-modal')!;
  const btnSendFeedback = document.getElementById('btn-send-feedback') as HTMLButtonElement;
  const feedbackSendStatus = document.getElementById('feedback-send-status')!;

  // Open modal
  document.getElementById('btn-open-feedback')!.addEventListener('click', () => {
    feedbackModal.classList.remove('hidden');
    feedbackSendStatus.textContent = '';
    feedbackSendStatus.className = 'feedback-send-status';
  });

  document.getElementById('btn-close-feedback')!.addEventListener('click', () => {
    feedbackModal.classList.add('hidden');
  });
  feedbackModal.addEventListener('click', (e) => {
    if (e.target === feedbackModal) feedbackModal.classList.add('hidden');
  });

  // Copy QQ + email to clipboard
  btnSendFeedback.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText('QQ: 1098905880\n邮箱: ccvtdd@qq.com');
      feedbackSendStatus.textContent = '✅ 已复制联系方式，请贴入对话框发送';
      feedbackSendStatus.className = 'feedback-send-status success';
    } catch {
      feedbackSendStatus.textContent = '⚠ 复制失败，请手动添加：QQ 1098905880 / ccvtdd@qq.com';
      feedbackSendStatus.className = 'feedback-send-status error';
    }
  });
});
