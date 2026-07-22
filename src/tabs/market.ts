import { invoke } from '@tauri-apps/api/core';
import {
  type MarketItemSummary, type MarketItemFull,
  type MarketOrder, type MarketCacheStatus,
  type MarketAuthStatus, type ProfileOrder,
  type CreateOrderRequest, type MarketCommandError,
  S,
  marketName, updateAuthDropdown,
  statusLabel, repClass, copyWhisper,
} from '../state';

// ── Lazy order rendering ──
export const ORDER_BATCH = 20;
type SortKey = 'price' | 'status';
interface SortState { key: SortKey; dir: 'asc' | 'desc'; }
let _lazyItemName = '';
let _lazyMaxRank: number | null | undefined = null;
let _lazySellOrders: MarketOrder[] = [];
let _lazyBuyOrders: MarketOrder[] = [];
let _lazySellCount = 0;
let _lazyBuyCount = 0;
let _lazySellSort: SortState = { key: 'price', dir: 'asc' };
let _lazyBuySort: SortState = { key: 'price', dir: 'desc' };
let _lazyObservers: IntersectionObserver[] = [];

export function orderRowHTML(o: MarketOrder, i: number): string {
  const rankCol = _lazyMaxRank ? `<td class="rank-cell">${o.mod_rank != null ? `${o.mod_rank} of ${_lazyMaxRank}` : '--'}</td>` : '';
  return `<tr style="--i:${i}">
    <td class="player" title="${o.player_name}">${o.player_name}</td>
    <td class="status-cell">${statusLabel(o.status)}</td>
    <td class="rep-cell"><span class="rep-badge ${repClass(o.reputation)}">${o.reputation >= 0 ? '+' : ''}${o.reputation}</span></td>${rankCol}
    <td class="price">${o.platinum}p</td>
    <td class="qty-cell">${o.quantity > 1 ? `<span class="qty-badge">×${o.quantity}</span>` : '<span class="qty-one">×1'}</td>
    <td class="whisper-cell"><button class="btn-whisper" onclick="var b=this;b.textContent='✓';setTimeout(function(){b.textContent='📋'},800);window._copyWhisper('${o.player_name}', '${_lazyItemName.replace(/'/g, "\\'")}', ${o.platinum}, '${o.order_type}')" title="复制私信">📋</button></td>
  </tr>`;
}

export function applySort(orders: MarketOrder[], sort: SortState): MarketOrder[] {
  const sorted = [...orders];
  if (sort.key === 'price') {
    sorted.sort((a, b) => sort.dir === 'asc' ? a.platinum - b.platinum : b.platinum - a.platinum);
  } else {
    const rank: Record<string, number> = { ingame: 0, online: 1, offline: 2 };
    sorted.sort((a, b) => {
      const ra = rank[a.status] ?? 3;
      const rb = rank[b.status] ?? 3;
      if (ra !== rb) return sort.dir === 'asc' ? ra - rb : rb - ra;
      return a.platinum - b.platinum;
    });
  }
  return sorted;
}

export function reloadOrderSide(side: 'sell' | 'buy') {
  // Disconnect existing observers for this side
  _lazyObservers.forEach(o => o.disconnect());
  _lazyObservers = [];

  const orders = side === 'sell' ? _lazySellOrders : _lazyBuyOrders;
  const sort = side === 'sell' ? _lazySellSort : _lazyBuySort;
  const tbodyId = side === 'sell' ? 'sell-tbody' : 'buy-tbody';

  const sorted = applySort(orders, sort);
  if (side === 'sell') { _lazySellOrders = sorted; _lazySellCount = 0; }
  else { _lazyBuyOrders = sorted; _lazyBuyCount = 0; }

  const tbody = document.getElementById(tbodyId);
  if (tbody) tbody.innerHTML = '';

  loadMoreOrders(side);
}

export function sortCtrlHTML(side: 'sell' | 'buy', key: SortKey, label: string): string {
  const sort = side === 'sell' ? _lazySellSort : _lazyBuySort;
  const active = sort.key === key;
  const arrow = active ? (sort.dir === 'asc' ? ' ↑' : ' ↓') : '';
  return `<span class="sort-ctrl${active ? ' active' : ''}" onclick="window._sortMarket('${side}','${key}')">${label}${arrow}</span>`;
}

export function loadMoreOrders(side: 'sell' | 'buy') {
  const orders = side === 'sell' ? _lazySellOrders : _lazyBuyOrders;
  const rendered = side === 'sell' ? _lazySellCount : _lazyBuyCount;
  const tbodyId = side === 'sell' ? 'sell-tbody' : 'buy-tbody';
  const sentinelId = side === 'sell' ? 'sell-sentinel' : 'buy-sentinel';

  const tbody = document.getElementById(tbodyId);
  if (!tbody || rendered >= orders.length) return;

  const batch = orders.slice(rendered, rendered + ORDER_BATCH);
  const oldSentinel = document.getElementById(sentinelId);
  if (oldSentinel) oldSentinel.remove();

  tbody.insertAdjacentHTML('beforeend', batch.map((o, i) => orderRowHTML(o, i + rendered)).join(''));

  if (side === 'sell') _lazySellCount += batch.length;
  else _lazyBuyCount += batch.length;

  const newRendered = side === 'sell' ? _lazySellCount : _lazyBuyCount;
  if (newRendered < orders.length) {
    const sentinel = document.createElement('tr');
    sentinel.id = sentinelId;
    sentinel.innerHTML = `<td colspan="${_lazyMaxRank ? 7 : 6}" style="padding:0;height:1px"></td>`;
    tbody.appendChild(sentinel);

    const observer = new IntersectionObserver((entries) => {
      if (entries[0].isIntersecting) {
        observer.disconnect();
        loadMoreOrders(side);
      }
    }, { rootMargin: '300px' });
    observer.observe(sentinel);
    _lazyObservers.push(observer);
  }
}

export function renderMarketResults(items: MarketItemSummary[]) {
  S._lastMarketResults = items;
  const container = document.getElementById('market-results')!;
  if (!items.length) {
    container.innerHTML = '<div class="market-empty">未找到物品</div>';
    return;
  }
  container.innerHTML = items.map(item => {
    const activeCls = S.marketOpenSlug === item.slug ? ' active' : '';
    return `
    <div class="market-result-row${activeCls}" data-slug="${item.slug}">
      <span class="market-result-name">${marketName(item)}</span>
    </div>`;
  }).join('');
}

export function renderMarketDetail(data: MarketItemFull) {
  _lazyObservers.forEach(o => o.disconnect());
  _lazyObservers = [];
  S._lastMarketDetail = data;  // cached for slug-match re-render without re-fetch
  S._lastMarketDetailTs = Date.now();

  const container = document.getElementById('market-detail')!;
  container.classList.remove('hidden');
  const item = data.item;
  const mrText = (item.mr != null && item.mr > 0) ? `MR ${item.mr}` : 'MR --';
  const taxText = data.trading_tax != null ? `交易税 ${data.trading_tax.toLocaleString()}` : '';
  const ducatsText = data.ducats != null ? `杜卡德 ${data.ducats}` : '';

  _lazyItemName = item.name;
  _lazyMaxRank = item.max_rank;
  _lazySellOrders = data.sell_orders;
  _lazyBuyOrders = data.buy_orders;
  _lazySellCount = 0;
  _lazyBuyCount = 0;
  _lazySellSort = { key: 'price', dir: 'asc' };
  _lazyBuySort = { key: 'price', dir: 'desc' };

  const makeOrderSide = (side: 'sell' | 'buy', orders: MarketOrder[], title: string): string => {
    const tbodyId = side === 'sell' ? 'sell-tbody' : 'buy-tbody';
    if (!orders.length) {
      return `<div class="market-order-side">
        <div class="market-order-title">${title}</div>
        <div class="market-order-empty">暂无${title.includes('卖') ? '卖单' : '买单'}</div>
      </div>`;
    }
    return `<div class="market-order-side">
      <div class="market-order-title">
        ${title} (${orders.length})
        <span class="sort-controls">${sortCtrlHTML(side, 'price', '价格')} ${sortCtrlHTML(side, 'status', '状态')}</span>
      </div>
      <table class="market-order-table"><tbody id="${tbodyId}"></tbody></table>
    </div>`;
  };

  container.innerHTML = `
    <div class="market-detail-head" id="market-detail-head">
      <div class="market-detail-info">
        <div class="market-detail-name">${marketName(item)}</div>
        <div class="market-detail-meta">
          <span>${mrText}</span>
          ${taxText ? `<span>${taxText}</span>` : ''}
          ${ducatsText ? `<span class="market-ducats">${ducatsText}</span>` : ''}
        </div>
      </div>
    </div>
    ${S.marketAuthName ? `
    <div class="market-detail-actions" id="market-detail-actions">
      <button id="btn-create-sell" onclick="window._showOrderForm('${item.slug}','sell')">📝 挂单出售</button>
      <button id="btn-create-buy" onclick="window._showOrderForm('${item.slug}','buy')">📝 挂单求购</button>
    </div>` : ''}
    ${S.orderFormSlug === item.slug ? orderFormHTML(item.slug, S.orderFormSide) : ''}
    ${data.set_parts.length ? `
    <div class="market-set-parts">
      ${(() => {
        const base = item.name.replace(/ Set$/, '');
        return data.set_parts.map(p => {
          let label = S.marketLang === 'zh' ? marketName(p) : p.name;
          if (S.marketLang !== 'zh' && label.startsWith(base + ' ')) label = label.substring(base.length + 1);
          if (S.marketLang !== 'zh') label = label.replace(/ Blueprint$/, '');
          return `<span class="set-part-link" onclick="window._openSetPart('${p.slug}')" title="${marketName(p)}">${label}</span>`;
        }).join('');
      })()}
    </div>` : ''}
    <div class="market-orders">
      ${makeOrderSide('sell', data.sell_orders, '卖家（最低价）')}
      ${makeOrderSide('buy', data.buy_orders, '买家（最高价）')}
    </div>
    <button class="btn-backtop" id="btn-market-backtop" onclick="document.getElementById('tab-market').scrollTop=0" title="回到顶部">⬆</button>`;

  // Bind order-form buttons if form is present.
  const submitBtn = document.getElementById('btn-order-submit');
  const cancelBtn = document.getElementById('btn-order-cancel');
  if (submitBtn) submitBtn.addEventListener('click', handleCreateOrder);
  if (cancelBtn) cancelBtn.addEventListener('click', hideOrderForm);

  loadMoreOrders('sell');
  loadMoreOrders('buy');
}

export function showMarketSkeleton() {
  const container = document.getElementById('market-detail')!;
  container.classList.remove('hidden');
  container.innerHTML = `
    <div class="market-skeleton">
      <div class="market-skel-line med"></div>
      <div class="market-skel-line short"></div>
      <div class="market-skel-line med"></div>
      <div class="market-skel-line short"></div>
    </div>`;
}

export function showMarketError(msg: string) {
  const container = document.getElementById('market-detail')!;
  container.classList.remove('hidden');
  container.innerHTML = `
    <div class="market-error">⚠️ ${msg}</div>
    <div style="text-align:center"><button class="timer-btn-sm market-retry-btn" id="btn-market-retry">重试</button></div>`;
}

export function updateMarketStatus(status: MarketCacheStatus) {
  const text = `${status.count} 条 · ${status.last_updated}`;
  document.getElementById('market-status-inline')!.textContent = text;
  const settingsEl = document.getElementById('market-status-settings');
  if (settingsEl) settingsEl.textContent = text;
}

// ── Market auth & orders ──────────────────────────────────────────────────

/** Generic invoke wrapper — catches MarketError and dispatches by code. */
export async function marketInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (err: any) {
    if (err?.code && err?.message) {
      throw err as MarketCommandError;
    }
    throw { code: 'unknown', message: String(err) } as MarketCommandError;
  }
}

/** Handle auth_expired globally — clear state, update chip, close forms. */
export function handleAuthExpired() {
  S.marketAuthName = null;
  S.marketAuthAvatar = null;
  S.marketAuthStatus = null;
  S.marketAuthRep = null;
  S.myOrders = [];
  S.myOrdersBySlug.clear();
  S.orderFormSlug = null;
  updateAuthChip();
  renderMyOrders();
  if (S._lastMarketDetail) renderMarketDetail(S._lastMarketDetail);
}

export function updateAuthChip() {
  const chip = document.getElementById('market-auth-chip')!;
  if (S.marketAuthName) {
    chip.textContent = `✓ ${S.marketAuthName}`;
    chip.className = 'market-auth-chip logged-in';
    chip.title = `已登录: ${S.marketAuthName} — 点击管理账号`;
  } else {
    chip.textContent = '🔒 未登录';
    chip.className = 'market-auth-chip';
    chip.title = '点击登录 Warframe.Market';
  }
  // Show/hide my-orders section
  const panel = document.getElementById('market-my-orders') as HTMLDetailsElement;
  panel.hidden = !S.marketAuthName;
  if (!S.marketAuthName) {
    panel.open = false;
  }
  // Update dropdown content
  updateAuthDropdown();
}

export async function initMarketAuth() {
  try {
    const status = await marketInvoke<MarketAuthStatus>('market_auth_status');
    if (status.logged_in && status.ingame_name) {
      S.marketAuthName = status.ingame_name;
      S.marketAuthAvatar = status.avatar ?? null;
      S.marketAuthStatus = status.current_status ?? null;
      S.marketAuthRep = status.reputation ?? null;
      updateAuthChip();
      refreshMyOrders();
    }
  } catch (_) {
    // Not logged in — chip already shows default.
  }
}

export async function refreshMyOrders() {
  if (!S.marketAuthName) return;
  try {
    S.myOrders = await marketInvoke<ProfileOrder[]>('market_list_orders');
  } catch (err: any) {
    if (err?.code === 'auth_expired') { handleAuthExpired(); return; }
    console.warn('refreshMyOrders failed:', err?.message || err);
    return; // keep existing data — never wipe on transient errors
  }
  // Rebuild slug→count map
  S.myOrdersBySlug.clear();
  for (const o of S.myOrders) {
    const slug = o.item_slug || o.item_id;
    S.myOrdersBySlug.set(slug, (S.myOrdersBySlug.get(slug) || 0) + 1);
  }
  renderMyOrders();
}

// ── Order mutation helpers ─────────────────────────────────────────────────

/** Flash a red border on an order row to signal a failed mutation. */
export function flashErrorRow(orderId: string) {
  const rows = document.querySelectorAll<HTMLElement>('.my-order-row');
  const row = Array.from(rows).find(r => r.dataset.orderId === orderId);
  if (!row) return;
  row.classList.add('flash-error');
  setTimeout(() => row.classList.remove('flash-error'), 900);
}

/** Show an inline error message below an order row for 3s. */
export function showOrderError(orderId: string, msg: string) {
  const prev = document.getElementById(`order-err-${orderId}`);
  if (prev) prev.remove();
  const rows = document.querySelectorAll<HTMLElement>('.my-order-row');
  const row = Array.from(rows).find(r => r.dataset.orderId === orderId);
  if (!row) return;
  const err = document.createElement('div');
  err.id = `order-err-${orderId}`;
  err.className = 'my-order-error';
  err.textContent = msg;
  row.insertAdjacentElement('afterend', err);
  setTimeout(() => { err.remove(); }, 3000);
}

/** Close the inline edit form (if any) and clear singleton state. */
export function cancelEdit() {
  if (S.editingOrderId) {
    S.editingOrderId = null;
    renderMyOrders();
  }
}

// ESC key closes the edit form.
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && S.editingOrderId) {
    cancelEdit();
  }
});

/** Core mutation wrapper: optimistic apply → API call → canonical update / rollback. */
export async function orderMutate(
  orderId: string,
  idx: number,
  apply: () => void,
  invokeFn: () => Promise<ProfileOrder | void>,
  rollback: () => void,
) {
  if (S.pendingOrderIds.has(orderId)) return;
  S.pendingOrderIds.add(orderId);
  apply();
  renderMyOrders();
  try {
    const result = await invokeFn();
    // Write the server response back — PATCH returns the full updated order.
    // DELETE returns void, so result is falsy and we skip (optimistic removal stands).
    if (result) {
      S.myOrders[idx] = result;
      // Rebuild slug map in case an order was replaced (relist changes the id).
      S.myOrdersBySlug.clear();
      for (const o of S.myOrders) {
        const s = o.item_slug || o.item_id;
        S.myOrdersBySlug.set(s, (S.myOrdersBySlug.get(s) || 0) + 1);
      }
    }
  } catch (err: any) {
    if (err?.code === 'auth_expired') { handleAuthExpired(); return; }
    rollback();
    flashErrorRow(orderId);
    showOrderError(orderId, err?.message || '操作失败');
  } finally {
    S.pendingOrderIds.delete(orderId);
    renderMyOrders(); // always re-render — the lock was just released
  }
}

/** +1 / Sold (-1): increment quantity, delete when reaching 0. */
export async function handleIncrement(orderId: string, delta: number) {
  if (S.pendingOrderIds.has(orderId) || S.editingOrderId) return;
  const idx = S.myOrders.findIndex(o => o.id === orderId);
  if (idx === -1) return;
  const prev = { ...S.myOrders[idx] };
  const newQty = prev.quantity + delta;

  if (newQty < 1) {
    if (!confirm(`数量归零，删除此订单？\n${prev.item_name} — ${prev.platinum}p`)) return;
    await orderMutate(orderId, idx,
      () => {
        S.myOrders.splice(idx, 1);
        const slug = prev.item_slug;
        S.myOrdersBySlug.set(slug, Math.max(0, (S.myOrdersBySlug.get(slug) || 1) - 1));
      },
      () => marketInvoke('market_delete_order', { orderId }),
      () => {
        S.myOrders.splice(idx, 0, prev);
        const slug = prev.item_slug;
        S.myOrdersBySlug.set(slug, (S.myOrdersBySlug.get(slug) || 0) + 1);
      },
    );
    return;
  }
  if (newQty > 100) return;

  await orderMutate(orderId, idx,
    () => { S.myOrders[idx].quantity = newQty; },
    () => marketInvoke<ProfileOrder>('market_update_order', { req: { order_id: orderId, quantity: newQty } }),
    () => { S.myOrders[idx] = prev; },
  );
}

/** Re-list a hidden order: DELETE old → POST new with visible:true.
 * Used when PATCH {visible:true} is rejected (WM may treat visible=false as close). */
export async function relistOrder(order: ProfileOrder, idx: number) {
  const orderId = order.id;
  if (S.pendingOrderIds.has(orderId)) return;
  S.pendingOrderIds.add(orderId);

  // Optimistic: remove old order, rebuild slug map.
  const slug = order.item_slug || order.item_id;
  S.myOrders.splice(idx, 1);
  S.myOrdersBySlug.set(slug, Math.max(0, (S.myOrdersBySlug.get(slug) || 1) - 1));
  renderMyOrders();

  try {
    // 1) Delete the hidden order.
    await marketInvoke<void>('market_delete_order', { orderId });

    // 2) Create a fresh one with visible: true (new id assigned by server).
    const created = await marketInvoke<ProfileOrder>('market_create_order', {
      req: {
        item_id: order.item_slug || order.item_id,
        order_type: order.order_type,
        platinum: order.platinum,
        quantity: order.quantity,
        rank: order.rank,
        visible: true,
      },
    });

    // 3) Insert the new order at the same position.
    S.myOrders.splice(idx, 0, created);
    S.myOrdersBySlug.set(slug, (S.myOrdersBySlug.get(slug) || 0) + 1);
  } catch (err: any) {
    if (err?.code === 'auth_expired') { handleAuthExpired(); return; }
    // Rollback: restore old order at original position.
    S.myOrders.splice(idx, 0, order);
    S.myOrdersBySlug.set(slug, (S.myOrdersBySlug.get(slug) || 0) + 1);
    flashErrorRow(orderId);
    showOrderError(orderId, err?.message || '恢复显示失败');
  } finally {
    S.pendingOrderIds.delete(orderId);
    renderMyOrders(); // always re-render — the lock was just released
  }
}

/** Toggle visible on/off. Hide uses PATCH; show uses DELETE+POST (relist)
 * because the WM API may treat visible=false as an irreversible close. */
export async function handleToggleVisible(orderId: string) {
  if (S.pendingOrderIds.has(orderId) || S.editingOrderId) return;
  const idx = S.myOrders.findIndex(o => o.id === orderId);
  if (idx === -1) return;
  const order = S.myOrders[idx];
  const newVis = !order.visible;

  if (newVis) {
    // Show: DELETE old + POST new (visible: true).
    await relistOrder(order, idx);
    return;
  }

  // Hide: PATCH {visible: false} (one round trip).
  const prev = { ...order };
  await orderMutate(orderId, idx,
    () => { S.myOrders[idx].visible = false; },
    () => marketInvoke<ProfileOrder>('market_update_order', { req: { order_id: orderId, visible: false } }),
    () => { S.myOrders[idx] = prev; },
  );
}

/** Submit the inline edit form. */
export async function handleSubmitEdit(orderId: string) {
  if (S.pendingOrderIds.has(orderId)) return;
  const idx = S.myOrders.findIndex(o => o.id === orderId);
  if (idx === -1) return;
  const prev = { ...S.myOrders[idx] };

  const priceInput = document.getElementById('edit-form-price') as HTMLInputElement;
  const qtyInput = document.getElementById('edit-form-qty') as HTMLInputElement;
  const visibleCb = document.getElementById('edit-form-visible') as HTMLInputElement;
  const rankEl = document.getElementById('edit-form-rank') as HTMLInputElement | null;

  const platinum = parseInt(priceInput?.value || '0', 10);
  const quantity = parseInt(qtyInput?.value || '0', 10);
  const visible = visibleCb?.checked ?? true;
  const rank = rankEl ? parseInt(rankEl.value, 10) : 0;

  if (platinum < 1 || platinum > 999999) {
    showOrderError(orderId, '价格必须在 1 ～ 999,999 之间');
    return;
  }
  if (quantity < 1 || quantity > 100) {
    showOrderError(orderId, '数量必须在 1 ～ 100 之间');
    return;
  }

  await orderMutate(orderId, idx,
    () => {
      Object.assign(S.myOrders[idx], { platinum, quantity, visible, rank });
      S.editingOrderId = null;
    },
    () => marketInvoke<ProfileOrder>('market_update_order', {
      req: { order_id: orderId, platinum, quantity, visible, rank },
    }),
    () => {
      Object.assign(S.myOrders[idx], prev);
      S.editingOrderId = null;
    },
  );
}

/** Toggle the inline edit form for an order (singleton: closes any other). */
export function handleEditOrder(orderId: string) {
  if (S.pendingOrderIds.has(orderId)) return;
  if (S.editingOrderId === orderId) {
    S.editingOrderId = null;
  } else {
    S.editingOrderId = orderId;
  }
  renderMyOrders();
}

export function renderMyOrders() {
  const listEl = document.getElementById('my-orders-list')!;
  const countEl = document.getElementById('my-orders-count')!;
  countEl.textContent = String(S.myOrders.length);

  if (S.myOrders.length === 0) {
    listEl.innerHTML = '<div class="my-orders-empty">暂无订单</div>';
    return;
  }

  listEl.innerHTML = S.myOrders.map(o => {
    const sideCls = o.order_type === 'sell' ? 'sell' : 'buy';
    const sideLabel = o.order_type === 'sell' ? '卖' : '买';
    const hidden = !o.visible;
    const pending = S.pendingOrderIds.has(o.id);
    const atMax = o.quantity >= 100;
    const atMin = o.quantity <= 1;
    const slug = o.item_slug || o.item_id;
    const eyeIcon = o.visible ? '👁' : '👁‍🗨';
    const eyeTitle = o.visible ? '点击隐藏' : '点击显示';
    const editFormHtml = S.editingOrderId === o.id ? editOrderFormHTML(o) : '';
    return `
    <div class="my-order-row${hidden ? ' hidden' : ''}${pending ? ' pending' : ''}" data-order-id="${o.id}">
      <span class="my-order-type ${sideCls}">${sideLabel}</span>
      <span class="my-order-name" onclick="window._openMarketItem('${slug}')" title="${o.item_name}">${o.item_name}</span>
      <span class="my-order-price">${o.platinum}p</span>
      <button class="btn-qty" ${pending || atMin ? 'disabled' : ''} onclick="window._incQty('${o.id}', -1)" title="Sold / 卖出一个">−</button>
      <span class="my-order-meta">×${o.quantity}</span>
      <button class="btn-qty" ${pending || atMax ? 'disabled' : ''} onclick="window._incQty('${o.id}', 1)" title="+1 数量">+</button>
      <span class="my-order-actions">
        <button ${pending ? 'disabled' : ''} onclick="window._toggleVisible('${o.id}')" title="${eyeTitle}">${eyeIcon}</button>
        <button ${pending ? 'disabled' : ''} onclick="window._editMyOrder('${o.id}')" title="编辑">✎</button>
        <button class="btn-order-delete" ${pending ? 'disabled' : ''} onclick="window._deleteMyOrder('${o.id}')" title="删除">✕</button>
      </span>
    </div>${editFormHtml}`;
  }).join('');
}

/** Compact inline edit form rendered below an order row in the my-orders list. */
export function editOrderFormHTML(order: ProfileOrder): string {
  const sideLabel = order.order_type === 'sell' ? '卖' : '买';
  return `
    <div class="my-order-edit-form" id="order-edit-${order.id}">
      <div class="market-order-form-title">编辑${sideLabel}单: ${order.item_name}</div>
      <div class="market-order-form-row">
        <label>价格 <input type="number" id="edit-form-price" min="1" max="999999" value="${order.platinum}" /></label>
        <label>数量 <input type="number" id="edit-form-qty" min="1" max="100" value="${order.quantity}" /></label>
        <label>等级 <input type="number" id="edit-form-rank" min="0" max="99" value="${order.rank}" style="width:50px" /></label>
        <label class="market-order-form-toggle">
          <input type="checkbox" id="edit-form-visible" ${order.visible ? 'checked' : ''} /> 公开可见
        </label>
      </div>
      <div class="market-order-form-actions">
        <button class="timer-btn-sm" onclick="window._submitEdit('${order.id}')">确认修改</button>
        <button class="timer-btn-sm" onclick="window._cancelEdit()">取消</button>
        <span class="market-order-form-status" id="edit-form-status"></span>
      </div>
    </div>`;
}

export function renderOrderForm(slug: string, side: 'sell' | 'buy') {
  S.orderFormSlug = slug;
  S.orderFormSide = side;
  if (!S._lastMarketDetail || S._lastMarketDetail.item.slug !== slug) return;
  // Re-render detail to show form.
  renderMarketDetail(S._lastMarketDetail);
}

export function hideOrderForm() {
  S.orderFormSlug = null;
  if (S._lastMarketDetail) renderMarketDetail(S._lastMarketDetail);
}

export function orderFormHTML(slug: string, side: 'sell' | 'buy'): string {
  const item = S._lastMarketDetail?.item;
  if (!item) return '';
  const maxRank = item.max_rank ?? 0;
  const existingCount = S.myOrdersBySlug.get(slug) || 0;
  const remaining = Math.max(0, 3 - existingCount);
  const sideLabel = side === 'sell' ? '出售' : '求购';

  // Build rank selector if applicable.
  let rankHtml = '';
  if (maxRank > 0) {
    const opts = Array.from({ length: maxRank + 1 }, (_, i) =>
      `<option value="${i}">${i}</option>`
    ).join('');
    rankHtml = `<label>等级 <select id="order-form-rank">${opts}</select></label>`;
  }

  // Reference prices from current orders.
  const sellOrders = S._lastMarketDetail?.sell_orders ?? [];
  const buyOrders = S._lastMarketDetail?.buy_orders ?? [];
  let refHtml = '';
  if (sellOrders.length > 0 && buyOrders.length > 0) {
    refHtml = `<div class="market-order-form-ref">当前行情: 最低卖价 <b class="market-ref-sell">${sellOrders[0].platinum}p</b> · 最高买价 <b class="market-ref-buy">${buyOrders[0].platinum}p</b></div>`;
  }

  let existingHtml = '';
  if (existingCount > 0) {
    existingHtml = `<div class="market-my-item-orders">你已有 <b>${existingCount}</b> 个${side === 'sell' ? '卖' : '买'}单（还可再挂 <b>${remaining}</b> 单）</div>`;
  }

  const disabled = remaining <= 0 ? 'disabled' : '';

  return `<div class="market-order-form">
    <div class="market-order-form-title">新建${sideLabel}单: ${marketName(item)}</div>
    ${refHtml}
    ${existingHtml}
    <div class="market-order-form-row">
      <label>价格 <input type="number" id="order-form-price" min="1" max="999999" value="${side === 'sell' ? (sellOrders[0]?.platinum ?? 10) : (buyOrders[0]?.platinum ?? 10)}" ${disabled} /></label>
      <label>数量 <input type="number" id="order-form-qty" min="1" max="100" value="1" ${disabled} /></label>
      ${rankHtml}
    </div>
    <div class="market-order-form-row">
      <label class="market-order-form-toggle">
        <input type="checkbox" id="order-form-visible" checked ${disabled} /> 公开可见
      </label>
    </div>
    <div class="market-order-form-actions">
      <button class="timer-btn-sm" id="btn-order-submit" ${disabled}>确认挂单</button>
      <button class="timer-btn-sm" id="btn-order-cancel">取消</button>
      <span class="market-order-form-status" id="order-form-status"></span>
    </div>
  </div>`;
}

async function handleCreateOrder() {
  if (!S.orderFormSlug) return;
  const item = S._lastMarketDetail?.item;
  if (!item) return;

  const priceInput = document.getElementById('order-form-price') as HTMLInputElement;
  const qtyInput = document.getElementById('order-form-qty') as HTMLInputElement;
  const visibleCb = document.getElementById('order-form-visible') as HTMLInputElement;
  const rankEl = document.getElementById('order-form-rank') as HTMLSelectElement | null;
  const statusEl = document.getElementById('order-form-status')!;
  const submitBtn = document.getElementById('btn-order-submit') as HTMLButtonElement;

  const platinum = parseInt(priceInput?.value || '0', 10);
  const quantity = parseInt(qtyInput?.value || '0', 10);
  const visible = visibleCb?.checked ?? true;
  const rank = rankEl ? parseInt(rankEl.value, 10) : 0;

  if (platinum < 1 || platinum > 999999) {
    statusEl.textContent = '价格必须在 1 ～ 999,999 之间';
    statusEl.className = 'market-order-form-status';
    return;
  }
  if (quantity < 1 || quantity > 100) {
    statusEl.textContent = '数量必须在 1 ～ 100 之间';
    statusEl.className = 'market-order-form-status';
    return;
  }

  statusEl.textContent = '挂单中…';
  statusEl.className = 'market-order-form-status';
  submitBtn.disabled = true;

  try {
    await marketInvoke<ProfileOrder>('market_create_order', {
      req: {
        item_id: item.slug, // Will be resolved by Rust side's MarketCache
        order_type: S.orderFormSide,
        platinum,
        quantity,
        rank,
        visible,
      } as CreateOrderRequest,
    });
    statusEl.textContent = '✅ 挂单成功';
    statusEl.className = 'market-order-form-status ok';
    S.orderFormSlug = null;
    await refreshMyOrders();
    // Refresh detail to show updated orders
    if (S.orderFormSlug === null) {
      // re-open the same item to refresh its order tables
      openMarketItem(item.slug);
    }
  } catch (err: any) {
    if (err?.code === 'auth_expired') {
      handleAuthExpired();
      return;
    }
    statusEl.textContent = err?.message || '挂单失败';
    statusEl.className = 'market-order-form-status';
  } finally {
    submitBtn.disabled = false;
  }
}

export async function handleDeleteOrder(orderId: string) {
  if (S.pendingOrderIds.has(orderId) || S.editingOrderId) return;
  const idx = S.myOrders.findIndex(o => o.id === orderId);
  if (idx === -1) return;
  const order = S.myOrders[idx];
  if (!confirm(`确定删除此订单？\n${order.item_name} — ${order.platinum}p ×${order.quantity}`)) return;
  const prev = { ...order };
  await orderMutate(orderId, idx,
    () => {
      S.myOrders.splice(idx, 1);
      const slug = prev.item_slug;
      S.myOrdersBySlug.set(slug, Math.max(0, (S.myOrdersBySlug.get(slug) || 1) - 1));
    },
    () => marketInvoke<void>('market_delete_order', { orderId }),
    () => {
      S.myOrders.splice(idx, 0, prev);
      const slug = prev.item_slug;
      S.myOrdersBySlug.set(slug, (S.myOrdersBySlug.get(slug) || 0) + 1);
    },
  );
}

// ── Market search & interaction ──

export async function doMarketSearch(query: string) {
  const q = query.trim();
  if (!q) {
    document.getElementById('market-results')!.innerHTML = '';
    S._lastMarketResults = [];
    return;
  }
  // Show skeleton rows immediately while the Rust backend searches.
  document.getElementById('market-results')!.innerHTML = Array.from({ length: 5 }, () =>
    `<div class="market-skel-row">
      <div class="market-skel-icon"></div>
      <div class="skel-line med" style="flex:1"></div>
    </div>`).join('');
  try {
    const items = await invoke<MarketItemSummary[]>('search_market_items', { query: q, lang: S.marketLang });
    renderMarketResults(items);
  } catch (err) {
    document.getElementById('market-results')!.innerHTML =
      `<div class="market-error">搜索失败: ${err}</div>`;
  }
}

const DETAIL_TTL_MS = 60_000;

export async function openMarketItem(slug: string) {
  S.marketOpenSlug = slug;
  // Cache hit: same slug within TTL → instant render.
  if (S._lastMarketDetail?.item.slug === slug
      && Date.now() - S._lastMarketDetailTs < DETAIL_TTL_MS) {
    renderMarketDetail(S._lastMarketDetail);
    return;
  }
  const reqId = ++S.marketReqId;
  showMarketSkeleton();
  // Hide search results while viewing detail
  document.getElementById('market-results')!.innerHTML = '';

  try {
    const data = await invoke<MarketItemFull>('get_market_item', { slug });
    if (reqId !== S.marketReqId) return;
    renderMarketDetail(data);
  } catch (err) {
    if (reqId !== S.marketReqId) return;
    showMarketError(String(err));
  }
}

export function closeMarketDetail() {
  S.marketOpenSlug = null;
  ++S.marketReqId;
  document.getElementById('market-detail')!.classList.add('hidden');
  // Restore search results
  const searchInput = document.getElementById('market-search') as HTMLInputElement;
  if (searchInput.value.trim()) {
    invoke<MarketItemSummary[]>('search_market_items', { query: searchInput.value.trim() })
      .then(renderMarketResults)
      .catch(() => {});
  }
}

// ── Register window._ handlers (module top level) ─────────────────────────
(window as any)._copyWhisper = copyWhisper;
(window as any)._openSetPart = (slug: string) => openMarketItem(slug);
(window as any)._showOrderForm = (slug: string, side: 'sell' | 'buy') => renderOrderForm(slug, side);
(window as any)._editMyOrder = (orderId: string) => handleEditOrder(orderId);
(window as any)._deleteMyOrder = (orderId: string) => handleDeleteOrder(orderId);
(window as any)._openMarketItem = (slug: string) => openMarketItem(slug);
(window as any)._incQty = (orderId: string, delta: number) => handleIncrement(orderId, delta);
(window as any)._toggleVisible = (orderId: string) => handleToggleVisible(orderId);
(window as any)._submitEdit = (orderId: string) => handleSubmitEdit(orderId);
(window as any)._cancelEdit = () => cancelEdit();
(window as any)._sortMarket = (side: 'sell' | 'buy', key: SortKey) => {
  const sort = side === 'sell' ? _lazySellSort : _lazyBuySort;
  if (sort.key === key) {
    sort.dir = sort.dir === 'asc' ? 'desc' : 'asc';
  } else {
    sort.key = key;
    sort.dir = key === 'price' ? (side === 'sell' ? 'asc' : 'desc') : 'asc';
  }
  // Update sort control labels.
  const sideEl = document.getElementById(side === 'sell' ? 'sell-tbody' : 'buy-tbody')?.closest('.market-order-side');
  if (sideEl) {
    const ctrls = sideEl.querySelector('.sort-controls');
    if (ctrls) {
      ctrls.innerHTML = `${sortCtrlHTML(side, 'price', '价格')} ${sortCtrlHTML(side, 'status', '状态')}`;
    }
  }
  reloadOrderSide(side);
};
