import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type ProviderStatus =
  | "ok"
  | "needs_auth"
  | "parse_error"
  | "network_error"
  | "unsupported";

type LatencyStatus = "ok" | "timeout" | "error";

interface UsageSnapshot {
  provider: string;
  displayName: string;
  membership?: string | null;
  periodStart?: string | null;
  periodEnd?: string | null;
  unit: string;
  used: number;
  limit?: number | null;
  remaining?: number | null;
  percentUsed?: number | null;
  autoPercentUsed?: number | null;
  apiPercentUsed?: number | null;
  onDemandUsed?: number | null;
  status: ProviderStatus;
  message?: string | null;
  fetchedAt: string;
}

interface SystemSnapshot {
  cpuPercent: number;
  cpuTempC?: number | null;
  gpuPercent?: number | null;
  gpuTempC?: number | null;
  memUsedBytes: number;
  memTotalBytes: number;
  fetchedAt: string;
}

interface LatencySnapshot {
  target: string;
  latencyMs?: number | null;
  status: LatencyStatus;
  fetchedAt: string;
  regionLabel?: string | null;
}

interface PanelState {
  usages: UsageSnapshot[];
  system: SystemSnapshot;
  latency: LatencySnapshot;
  autoRefreshSec: number;
  systemRefreshSec: number;
  highLatencyMs: number;
  hasCursorToken: boolean;
  hasDeepseekKey: boolean;
}

interface AppSettings {
  cursorRefreshSec: number;
  systemRefreshSec: number;
  latencyTarget: string;
  highLatencyMs: number;
}

interface LocalSessionProbe {
  homesTried: string[];
  dbPathsFound: number;
  dbPathsOpenable: number;
  tokenLen?: number | null;
  failure?: string | null;
}

const $ = <T extends HTMLElement>(id: string) =>
  document.getElementById(id) as T;

let panelState: PanelState | null = null;
let cursorTimer: number | undefined;
let systemTimer: number | undefined;
let refreshing = false;
/** 面板刚显示后的失焦保护窗口（毫秒时间戳） */
let ignoreBlurUntil = 0;
const BLUR_GRACE_MS = 350;
const LATENCY_HISTORY_SIZE = 20;
const COMPACT_STORAGE_KEY = "usages-compact-collapsed";
const SETTINGS_PROVIDER_COLLAPSED_KEY = "usages-settings-provider-collapsed";

type UsageTone = "ok" | "warn" | "danger" | "neutral";

/** 延迟采样环形缓冲（毫秒；null 表示超时/错误） */
const latencyHistory: (number | null)[] = [];
let latencyHistoryHasReal = false;
let lastLatencyFetchedAt: string | null = null;

function centsToDollars(cents: number): string {
  return `$${(cents / 100).toFixed(2)}`;
}

function formatPercent(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return "—";
  return `${Math.max(0, Math.min(100, n)).toFixed(0)}%`;
}

function clampPercent(n: number): number {
  return Math.max(0, Math.min(100, n));
}

/** 用量百分比 → 色阶：≥90 危险，≥70 警告，否则正常 */
function usageTone(percent: number | null | undefined): UsageTone {
  if (percent == null || Number.isNaN(percent)) return "neutral";
  const used = clampPercent(percent);
  if (used >= 90) return "danger";
  if (used >= 70) return "warn";
  return "ok";
}

function applyTone(el: HTMLElement, tone: UsageTone): void {
  el.classList.remove("tone-ok", "tone-warn", "tone-danger");
  if (tone === "ok" || tone === "warn" || tone === "danger") {
    el.classList.add(`tone-${tone}`);
  }
}

function setFillTone(fill: HTMLElement, tone: UsageTone): void {
  fill.classList.toggle("ok", tone === "ok");
  fill.classList.toggle("warn", tone === "warn");
  fill.classList.toggle("danger", tone === "danger");
}

function setUsageBar(
  rowId: string,
  fillId: string,
  pctId: string,
  percent: number | null | undefined,
): void {
  const row = $(rowId);
  const fill = $(fillId);
  const pctEl = $(pctId);
  if (percent == null || Number.isNaN(percent)) {
    row.classList.add("hidden");
    fill.style.width = "0%";
    setFillTone(fill, "neutral");
    applyTone(pctEl, "neutral");
    pctEl.textContent = "—";
    return;
  }
  const used = clampPercent(percent);
  const tone = usageTone(used);
  row.classList.remove("hidden");
  fill.style.width = `${used}%`;
  setFillTone(fill, tone);
  applyTone(pctEl, tone);
  pctEl.textContent = `${formatPercent(used)} used`;
}

function setStatBar(
  barId: string,
  percent: number | null | undefined,
): void {
  const bar = $(barId);
  if (percent == null || Number.isNaN(percent)) {
    bar.style.width = "0%";
    return;
  }
  bar.style.width = `${clampPercent(percent)}%`;
}

function formatPeriodEnd(iso: string | null | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const weekday = d.toLocaleDateString(undefined, { weekday: "short" });
  const datePart = d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
  const timePart = d.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  return `${weekday} · ${datePart}, ${timePart}`;
}

function formatUpdated(iso: string | null | undefined): string {
  if (!iso) return "Updated —";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "Updated —";
  return `Updated ${d.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  })}`;
}

function latestFetchedAt(state: PanelState): string | null {
  const times = [
    ...state.usages.map((u) => u.fetchedAt),
    state.system.fetchedAt,
    state.latency.fetchedAt,
  ].filter((t) => !!t);
  if (times.length === 0) return null;
  times.sort();
  return times[times.length - 1] ?? null;
}

function cursorUsage(state: PanelState): UsageSnapshot | undefined {
  return state.usages.find((u) => u.provider === "cursor");
}

function deepseekUsage(state: PanelState): UsageSnapshot | undefined {
  return state.usages.find((u) => u.provider === "deepseek");
}

function formatBalanceAmount(currency: string | null | undefined, amount: number): string {
  const c = (currency ?? "").toUpperCase();
  if (c === "CNY" || c === "RMB") return `¥${amount.toFixed(2)}`;
  if (c === "USD") return `$${amount.toFixed(2)}`;
  if (c) return `${c} ${amount.toFixed(2)}`;
  return amount.toFixed(2);
}

function renderCursor(state: PanelState): void {
  const snap = cursorUsage(state);
  const membership = $("cursor-membership");
  const amountEl = $("cursor-amount");
  const periodEl = $("cursor-period");
  const alertEl = $("cursor-alert");

  if (!snap) {
    membership.textContent = "";
    setUsageBar("cursor-auto-row", "cursor-auto-progress", "cursor-auto-pct", null);
    setUsageBar("cursor-api-row", "cursor-api-progress", "cursor-api-pct", null);
    amountEl.textContent = "—";
    applyTone(amountEl, "neutral");
    periodEl.textContent = "";
    alertEl.classList.add("hidden");
    renderCursorCompactSummary(undefined);
    return;
  }

  membership.textContent = snap.membership ?? "";

  setUsageBar(
    "cursor-auto-row",
    "cursor-auto-progress",
    "cursor-auto-pct",
    snap.autoPercentUsed,
  );
  setUsageBar(
    "cursor-api-row",
    "cursor-api-progress",
    "cursor-api-pct",
    snap.apiPercentUsed,
  );

  if (snap.unit === "cents") {
    const limit = snap.limit != null ? centsToDollars(snap.limit) : "—";
    amountEl.textContent = `Included ${centsToDollars(snap.used)} / ${limit}`;
  } else {
    amountEl.textContent = `${snap.used}${snap.limit != null ? ` / ${snap.limit}` : ""}`;
  }

  const amountTone = cursorAmountTone(snap);
  applyTone(amountEl, amountTone);

  periodEl.textContent = formatPeriodEnd(snap.periodEnd);

  if (snap.status === "needs_auth") {
    alertEl.textContent =
      snap.message || "请确保本机已登录 Cursor，或在设置中粘贴 Cookie";
    alertEl.classList.remove("hidden");
  } else if (snap.status !== "ok" && snap.message) {
    alertEl.textContent = snap.message;
    alertEl.classList.remove("hidden");
  } else {
    alertEl.textContent = "";
    alertEl.classList.add("hidden");
  }

  renderCursorCompactSummary(snap);
}

function cursorAmountTone(snap: UsageSnapshot): UsageTone {
  const candidates = [
    snap.autoPercentUsed,
    snap.apiPercentUsed,
    snap.percentUsed,
  ].filter((n): n is number => n != null && !Number.isNaN(n));
  if (candidates.length === 0) return "neutral";
  return usageTone(Math.max(...candidates));
}

function renderCursorCompactSummary(snap: UsageSnapshot | undefined): void {
  const el = $("cursor-compact-text");
  if (!snap) {
    el.textContent = "—";
    applyTone(el, "neutral");
    return;
  }
  const parts: string[] = [];
  if (snap.autoPercentUsed != null) {
    parts.push(`Auto ${formatPercent(snap.autoPercentUsed)}`);
  }
  if (snap.apiPercentUsed != null) {
    parts.push(`API ${formatPercent(snap.apiPercentUsed)}`);
  }
  if (parts.length === 0 && snap.percentUsed != null) {
    parts.push(formatPercent(snap.percentUsed));
  }
  el.textContent = parts.length > 0 ? parts.join(" · ") : "—";
  applyTone(el, cursorAmountTone(snap));
}

function renderDeepSeek(state: PanelState): void {
  const card = $("card-deepseek");
  const snap = deepseekUsage(state);
  const show =
    state.hasDeepseekKey ||
    (snap != null && snap.status !== "needs_auth");

  card.classList.toggle("hidden", !show);
  if (!show) return;

  const currencyEl = $("deepseek-currency");
  const balanceEl = $("deepseek-balance");
  const detailEl = $("deepseek-detail");
  const alertEl = $("deepseek-alert");

  if (!snap) {
    currencyEl.textContent = "";
    balanceEl.textContent = "—";
    applyTone(balanceEl, "neutral");
    detailEl.textContent = "加载中…";
    alertEl.classList.add("hidden");
    return;
  }

  currencyEl.textContent = snap.membership ?? "";

  if (snap.status === "needs_auth") {
    balanceEl.textContent = "—";
    applyTone(balanceEl, "warn");
    detailEl.textContent = "需要 API Key";
    alertEl.textContent = snap.message || "请在设置中配置 DeepSeek API Key";
    alertEl.classList.remove("hidden");
    return;
  }

  if (snap.status !== "ok") {
    balanceEl.textContent = "—";
    applyTone(balanceEl, "warn");
    detailEl.textContent = snap.message || "查询失败";
    alertEl.textContent = snap.message || "DeepSeek 余额查询失败";
    alertEl.classList.remove("hidden");
    return;
  }

  const total = snap.remaining ?? 0;
  balanceEl.textContent = formatBalanceAmount(snap.membership, total);
  applyTone(balanceEl, total <= 0 ? "warn" : "ok");

  const parts: string[] = [];
  if (snap.used > 0) {
    parts.push(`充值 ${formatBalanceAmount(snap.membership, snap.used)}`);
  }
  if (snap.onDemandUsed != null && snap.onDemandUsed > 0) {
    parts.push(`赠送 ${formatBalanceAmount(snap.membership, snap.onDemandUsed)}`);
  }
  detailEl.textContent = parts.length > 0 ? parts.join(" · ") : "官方余额接口";

  if (snap.message) {
    alertEl.textContent = snap.message;
    alertEl.classList.remove("hidden");
  } else {
    alertEl.textContent = "";
    alertEl.classList.add("hidden");
  }
}

function formatMemGb(bytes: number): string {
  return (bytes / (1024 * 1024 * 1024)).toFixed(1);
}

function formatTemp(temp: number | null | undefined): string | null {
  if (temp == null || Number.isNaN(temp)) return null;
  return `${Math.round(temp)}°C`;
}

function setStatSub(id: string, text: string | null): void {
  const el = $(id);
  if (text == null) {
    el.textContent = "";
    el.classList.add("hidden");
  } else {
    el.textContent = text;
    el.classList.remove("hidden");
  }
}

function renderSystem(state: PanelState): void {
  const sys = state.system;
  const hasData = !!sys.fetchedAt;

  const cpuPct = hasData ? sys.cpuPercent : null;
  $("sys-cpu-pct").textContent = formatPercent(cpuPct);
  setStatBar("sys-cpu-bar", cpuPct);
  setStatSub("sys-cpu-sub", formatTemp(sys.cpuTempC));

  if (hasData && sys.gpuPercent != null) {
    $("sys-gpu-pct").textContent = formatPercent(sys.gpuPercent);
    setStatBar("sys-gpu-bar", sys.gpuPercent);
  } else {
    $("sys-gpu-pct").textContent = hasData ? "N/A" : "—";
    setStatBar("sys-gpu-bar", null);
  }
  setStatSub("sys-gpu-sub", formatTemp(sys.gpuTempC));

  if (hasData && sys.memTotalBytes > 0) {
    const memPct = (sys.memUsedBytes / sys.memTotalBytes) * 100;
    $("sys-mem-pct").textContent = formatPercent(memPct);
    setStatBar("sys-mem-bar", memPct);
    $("sys-mem-sub").textContent = `${formatMemGb(sys.memUsedBytes)}/${formatMemGb(sys.memTotalBytes)} GB`;
    $("sys-mem-sub").classList.remove("hidden");
  } else {
    $("sys-mem-pct").textContent = "—";
    setStatBar("sys-mem-bar", null);
    $("sys-mem-sub").textContent = "—";
    $("sys-mem-sub").classList.remove("hidden");
  }
}

function pushLatencySample(lat: LatencySnapshot): void {
  if (!lat.fetchedAt || lat.fetchedAt === lastLatencyFetchedAt) return;
  lastLatencyFetchedAt = lat.fetchedAt;

  if (lat.status === "ok" && lat.latencyMs != null) {
    latencyHistory.push(lat.latencyMs);
  } else {
    latencyHistory.push(null);
  }
  latencyHistoryHasReal = true;
  while (latencyHistory.length > LATENCY_HISTORY_SIZE) {
    latencyHistory.shift();
  }
}

function placeholderSparkValues(count: number): number[] {
  return Array.from({ length: count }, (_, i) => {
    const wave = Math.sin(i * 0.65 + 1.2) * 0.22 + 0.38;
    return Math.round(wave * 100);
  });
}

function renderLatencySpark(highThreshold: number): void {
  const container = $("latency-spark");
  container.replaceChildren();

  const slots = LATENCY_HISTORY_SIZE;
  let values: (number | null)[];
  let isPlaceholder = false;

  if (latencyHistoryHasReal && latencyHistory.length > 0) {
    const pad = slots - latencyHistory.length;
    values = [...Array(Math.max(0, pad)).fill(null), ...latencyHistory];
  } else {
    values = placeholderSparkValues(slots);
    isPlaceholder = true;
  }

  const numeric = values.filter((v): v is number => v != null);
  const maxMs = Math.max(highThreshold, ...numeric, 100);

  values.forEach((ms) => {
    const bar = document.createElement("div");
    bar.className = "spark-bar";
    if (ms == null) {
      bar.classList.add("empty");
      bar.style.height = "12%";
    } else {
      const pct = Math.max(10, Math.min(100, (ms / maxMs) * 100));
      bar.style.height = `${pct}%`;
      if (!isPlaceholder && ms > highThreshold) {
        bar.classList.add("high");
      }
      if (isPlaceholder) {
        bar.classList.add("placeholder");
      }
    }
    container.appendChild(bar);
  });
}

function renderLatency(state: PanelState): void {
  const lat = state.latency;
  const el = $("latency-ms");
  const regionEl = $("latency-region");
  el.classList.remove("high");

  if (lat.fetchedAt) {
    pushLatencySample(lat);
  }

  if (!lat.fetchedAt) {
    el.textContent = "—";
    regionEl.textContent = "—";
    regionEl.title = "";
    regionEl.classList.remove("muted-hint");
    renderLatencySpark(state.highLatencyMs);
    return;
  }

  const regionText = lat.regionLabel?.trim() || "出口暂不可用";
  regionEl.textContent = regionText;
  regionEl.title = regionText;
  regionEl.classList.toggle(
    "muted-hint",
    regionText === "出口暂不可用" || regionText === "—",
  );

  if (lat.status !== "ok" || lat.latencyMs == null) {
    el.textContent = lat.status === "timeout" ? "超时" : "错误";
    el.classList.add("high");
    renderLatencySpark(state.highLatencyMs);
    return;
  }

  el.textContent = `${lat.latencyMs} ms`;
  if (lat.latencyMs > state.highLatencyMs) {
    el.classList.add("high");
  }
  renderLatencySpark(state.highLatencyMs);
}

function renderFooter(state: PanelState): void {
  $("footer-auto").textContent = "Auto refresh";
  $("footer-updated").textContent = formatUpdated(latestFetchedAt(state));
}

function renderPanel(state: PanelState): void {
  panelState = state;
  renderCursor(state);
  renderDeepSeek(state);
  renderSystem(state);
  renderLatency(state);
  renderFooter(state);
  scheduleAutoRefresh(state);
}

function clearTimers(): void {
  if (cursorTimer != null) {
    window.clearInterval(cursorTimer);
    cursorTimer = undefined;
  }
  if (systemTimer != null) {
    window.clearInterval(systemTimer);
    systemTimer = undefined;
  }
}

function scheduleAutoRefresh(state: PanelState): void {
  clearTimers();
  const cursorMs = Math.max(60, state.autoRefreshSec) * 1000;
  const systemMs = Math.min(30, Math.max(10, state.systemRefreshSec)) * 1000;

  cursorTimer = window.setInterval(() => {
    void refreshProvidersOnly();
  }, cursorMs);

  systemTimer = window.setInterval(() => {
    void refreshSystemAndLatency();
  }, systemMs);
}

async function refreshAll(): Promise<void> {
  if (refreshing) return;
  refreshing = true;
  const btn = $("btn-refresh");
  btn.classList.add("spin");
  btn.setAttribute("disabled", "true");
  try {
    const state = await invoke<PanelState>("refresh_all");
    renderPanel(state);
  } catch (err) {
    console.error("refresh_all failed", err);
  } finally {
    refreshing = false;
    btn.classList.remove("spin");
    btn.removeAttribute("disabled");
  }
}

async function refreshProvidersOnly(): Promise<void> {
  try {
    await invoke<UsageSnapshot>("refresh_cursor");
    if (panelState?.hasDeepseekKey) {
      await invoke<UsageSnapshot>("refresh_deepseek");
    }
    const state = await invoke<PanelState>("get_panel_state");
    renderPanel(state);
  } catch (err) {
    console.error("provider refresh failed", err);
  }
}

async function refreshCursorOnly(): Promise<void> {
  try {
    await invoke<UsageSnapshot>("refresh_cursor");
    const state = await invoke<PanelState>("get_panel_state");
    renderPanel(state);
  } catch (err) {
    console.error("refresh_cursor failed", err);
  }
}

async function refreshDeepSeekOnly(): Promise<void> {
  try {
    await invoke<UsageSnapshot>("refresh_deepseek");
    const state = await invoke<PanelState>("get_panel_state");
    renderPanel(state);
  } catch (err) {
    console.error("refresh_deepseek failed", err);
  }
}

async function refreshSystemAndLatency(): Promise<void> {
  try {
    await Promise.all([
      invoke<SystemSnapshot>("refresh_system"),
      invoke<LatencySnapshot>("refresh_latency"),
    ]);
    const state = await invoke<PanelState>("get_panel_state");
    renderPanel(state);
  } catch (err) {
    console.error("system/latency refresh failed", err);
  }
}

function showMain(): void {
  const main = $("view-main");
  const settings = $("view-settings");
  settings.classList.add("view-hidden");
  settings.setAttribute("aria-hidden", "true");
  main.classList.remove("view-hidden");
  main.removeAttribute("aria-hidden");
}

function showSettings(): void {
  const main = $("view-main");
  const settings = $("view-settings");
  main.classList.add("view-hidden");
  main.setAttribute("aria-hidden", "true");
  settings.classList.remove("view-hidden");
  settings.removeAttribute("aria-hidden");
  void loadSettingsForm();
}

async function loadSettingsForm(): Promise<void> {
  const msg = $("settings-msg");
  msg.textContent = "";
  msg.className = "settings-msg";
  try {
    const settings = await invoke<AppSettings>("get_settings");
    ($("cursor-refresh-input") as HTMLInputElement).value = String(
      settings.cursorRefreshSec,
    );
    ($("system-refresh-input") as HTMLInputElement).value = String(
      settings.systemRefreshSec,
    );
    ($("latency-target-input") as HTMLInputElement).value =
      settings.latencyTarget;
    ($("high-latency-input") as HTMLInputElement).value = String(
      settings.highLatencyMs,
    );

    const state =
      panelState ?? (await invoke<PanelState>("get_panel_state"));
    updateTokenSavedUi(state.hasCursorToken);
    updateDeepseekKeyUi(state.hasDeepseekKey);
    ($("token-input") as HTMLTextAreaElement).value = "";
    ($("deepseek-key-input") as HTMLInputElement).value = "";
    $("diagnose-result").textContent = "";
  } catch (err) {
    msg.textContent = `读取设置失败：${String(err)}`;
    msg.classList.add("error");
  }
}

function updateTokenSavedUi(hasToken: boolean): void {
  const hint = $("token-hint");
  const status = $("token-status");
  if (hasToken) {
    hint.textContent =
      "已保存 Cookie 兜底。保存后不会回显完整 Cookie，属正常；粘贴新值可覆盖。";
    status.textContent = "Cookie 已保存";
    status.classList.add("saved");
  } else {
    hint.textContent =
      "未保存 Cookie。本机已登录 Cursor 时通常无需粘贴；需要时可在此保存兜底 Cookie。";
    status.textContent = "未保存 Cookie";
    status.classList.remove("saved");
  }
  ($("token-input") as HTMLTextAreaElement).placeholder = hasToken
    ? "••••••••（已保存，粘贴可覆盖）"
    : "通常无需填写；登录 Cursor 即可";
}

function updateDeepseekKeyUi(hasKey: boolean): void {
  const hint = $("deepseek-key-hint");
  const status = $("deepseek-key-status");
  if (hasKey) {
    hint.textContent = "已保存 API Key。保存后不会回显，属正常；输入新值可覆盖。";
    status.textContent = "Key 已保存";
    status.classList.add("saved");
  } else {
    hint.textContent = "未保存。配置后主面板将显示 DeepSeek 余额卡片。";
    status.textContent = "未保存";
    status.classList.remove("saved");
  }
  ($("deepseek-key-input") as HTMLInputElement).placeholder = hasKey
    ? "••••••••（已保存，输入可覆盖）"
    : "sk-…";
}

function setCompactCollapsed(collapsed: boolean): void {
  const section = $("cursor-section");
  const btn = $("btn-compact-toggle");
  const compact = $("cursor-compact");
  const full = $("cursor-full");

  section.classList.toggle("collapsed", collapsed);
  btn.setAttribute("aria-expanded", String(!collapsed));
  compact.classList.toggle("hidden", !collapsed);
  full.classList.toggle("hidden", collapsed);

  try {
    localStorage.setItem(COMPACT_STORAGE_KEY, collapsed ? "1" : "0");
  } catch {
    /* 忽略 localStorage 不可用 */
  }
}

function loadCompactState(): boolean {
  try {
    return localStorage.getItem(COMPACT_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function loadProviderCollapsedMap(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(SETTINGS_PROVIDER_COLLAPSED_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }
    const out: Record<string, boolean> = {};
    for (const [key, value] of Object.entries(parsed)) {
      if (typeof value === "boolean") out[key] = value;
    }
    return out;
  } catch {
    return {};
  }
}

function saveProviderCollapsedMap(map: Record<string, boolean>): void {
  try {
    localStorage.setItem(SETTINGS_PROVIDER_COLLAPSED_KEY, JSON.stringify(map));
  } catch {
    /* 忽略 localStorage 不可用 */
  }
}

function setProviderCollapsed(
  section: HTMLElement,
  collapsed: boolean,
  persist = true,
): void {
  const provider = section.dataset.provider;
  if (!provider) return;

  const toggle = section.querySelector<HTMLButtonElement>(
    ".settings-group-toggle",
  );
  section.classList.toggle("collapsed", collapsed);
  if (toggle) {
    toggle.setAttribute("aria-expanded", String(!collapsed));
  }

  if (!persist) return;
  const map = loadProviderCollapsedMap();
  map[provider] = collapsed;
  saveProviderCollapsedMap(map);
}

function bindSettingsProviderCollapse(): void {
  const map = loadProviderCollapsedMap();
  document
    .querySelectorAll<HTMLElement>(".settings-group.settings-provider")
    .forEach((section) => {
      const provider = section.dataset.provider;
      if (!provider) return;
      setProviderCollapsed(section, map[provider] === true, false);

      const toggle = section.querySelector<HTMLButtonElement>(
        ".settings-group-toggle",
      );
      toggle?.addEventListener("click", () => {
        setProviderCollapsed(
          section,
          !section.classList.contains("collapsed"),
        );
      });
    });
}

function formatDiagnoseResult(p: LocalSessionProbe): string {
  const homes = p.homesTried?.length ?? 0;
  const tokenPart =
    p.tokenLen != null ? `tokenLen=${p.tokenLen}` : "tokenLen=无";
  const fail = p.failure ? `failure=${p.failure}` : "failure=无";
  return `homes=${homes} · dbFound=${p.dbPathsFound} · dbOpen=${p.dbPathsOpenable} · ${tokenPart} · ${fail}`;
}

function bindUi(): void {
  setCompactCollapsed(loadCompactState());
  bindSettingsProviderCollapse();

  $("btn-compact-toggle").addEventListener("click", () => {
    const section = $("cursor-section");
    setCompactCollapsed(!section.classList.contains("collapsed"));
  });

  $("btn-refresh").addEventListener("click", () => {
    void refreshAll();
  });
  $("btn-settings").addEventListener("click", () => showSettings());
  $("btn-back").addEventListener("click", () => {
    showMain();
    void refreshAll();
  });

  $("btn-save-token").addEventListener("click", async () => {
    const msg = $("settings-msg");
    const token = ($("token-input") as HTMLTextAreaElement).value.trim();
    if (!token) {
      msg.textContent = "请先粘贴 Cookie";
      msg.className = "settings-msg error";
      return;
    }
    try {
      await invoke("set_cursor_session_token", { token });
      const state = await invoke<PanelState>("get_panel_state");
      if (!state.hasCursorToken) {
        msg.textContent = "保存后回读失败，Cookie 可能未写入";
        msg.className = "settings-msg error";
        updateTokenSavedUi(false);
        return;
      }
      if (panelState) {
        panelState.hasCursorToken = true;
      } else {
        panelState = state;
      }
      ($("token-input") as HTMLTextAreaElement).value = "";
      updateTokenSavedUi(true);
      msg.textContent = "已保存（不会回显完整 Cookie，属正常）";
      msg.className = "settings-msg ok";
      await refreshCursorOnly();
    } catch (err) {
      msg.textContent = `保存失败：${String(err)}`;
      msg.className = "settings-msg error";
    }
  });

  $("btn-clear-token").addEventListener("click", async () => {
    const msg = $("settings-msg");
    try {
      await invoke("clear_cursor_session_token");
      ($("token-input") as HTMLTextAreaElement).value = "";
      if (panelState) {
        panelState.hasCursorToken = false;
      }
      updateTokenSavedUi(false);
      msg.textContent = "已清除 Cookie";
      msg.className = "settings-msg ok";
    } catch (err) {
      msg.textContent = `清除失败：${String(err)}`;
      msg.className = "settings-msg error";
    }
  });

  $("btn-diagnose-session").addEventListener("click", async () => {
    const out = $("diagnose-result");
    out.textContent = "诊断中…";
    try {
      const probe = await invoke<LocalSessionProbe>("diagnose_local_session");
      out.textContent = formatDiagnoseResult(probe);
    } catch (err) {
      out.textContent = `诊断失败：${String(err)}`;
    }
  });

  $("btn-save-deepseek").addEventListener("click", async () => {
    const msg = $("settings-msg");
    const key = ($("deepseek-key-input") as HTMLInputElement).value.trim();
    if (!key) {
      msg.textContent = "请先输入 DeepSeek API Key";
      msg.className = "settings-msg error";
      return;
    }
    try {
      await invoke("set_deepseek_api_key", { key });
      const state = await invoke<PanelState>("get_panel_state");
      if (!state.hasDeepseekKey) {
        msg.textContent = "保存后回读失败，API Key 可能未写入";
        msg.className = "settings-msg error";
        updateDeepseekKeyUi(false);
        return;
      }
      if (panelState) {
        panelState.hasDeepseekKey = true;
      } else {
        panelState = state;
      }
      ($("deepseek-key-input") as HTMLInputElement).value = "";
      updateDeepseekKeyUi(true);
      msg.textContent = "DeepSeek API Key 已保存";
      msg.className = "settings-msg ok";
      await refreshDeepSeekOnly();
    } catch (err) {
      msg.textContent = `保存失败：${String(err)}`;
      msg.className = "settings-msg error";
    }
  });

  $("btn-clear-deepseek").addEventListener("click", async () => {
    const msg = $("settings-msg");
    try {
      await invoke("clear_deepseek_api_key");
      ($("deepseek-key-input") as HTMLInputElement).value = "";
      if (panelState) {
        panelState.hasDeepseekKey = false;
      }
      updateDeepseekKeyUi(false);
      msg.textContent = "已清除 DeepSeek API Key";
      msg.className = "settings-msg ok";
      await refreshAll();
    } catch (err) {
      msg.textContent = `清除失败：${String(err)}`;
      msg.className = "settings-msg error";
    }
  });

  $("settings-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const msg = $("settings-msg");
    const patch = {
      cursorRefreshSec: Number(
        ($("cursor-refresh-input") as HTMLInputElement).value,
      ),
      systemRefreshSec: Number(
        ($("system-refresh-input") as HTMLInputElement).value,
      ),
      latencyTarget: ($("latency-target-input") as HTMLInputElement).value,
      highLatencyMs: Number(($("high-latency-input") as HTMLInputElement).value),
    };
    try {
      await invoke<AppSettings>("update_settings", { patch });
      msg.textContent = "设置已保存";
      msg.className = "settings-msg ok";
      const state = await invoke<PanelState>("get_panel_state");
      renderPanel(state);
    } catch (err) {
      msg.textContent = `保存失败：${String(err)}`;
      msg.className = "settings-msg error";
    }
  });
}

function markPanelShown(): void {
  ignoreBlurUntil = Date.now() + BLUR_GRACE_MS;
}

window.addEventListener("DOMContentLoaded", () => {
  document.documentElement.dataset.app = "usages";
  bindUi();
  void refreshAll();

  const win = getCurrentWindow();
  markPanelShown();

  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      markPanelShown();
    }
  });

  void win.onFocusChanged(({ payload: focused }) => {
    if (focused) {
      markPanelShown();
      void refreshAll();
      return;
    }
    if (Date.now() < ignoreBlurUntil) return;
    void win.hide();
  });
});
