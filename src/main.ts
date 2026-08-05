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
  /** 公网出口 IP；探测失败时为 null/缺省，勿展示假数据。 */
  egressIp?: string | null;
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

type ProviderId = "cursor" | "codex" | "deepseek";
type VisibilityMode = "auto" | "always" | "hidden";

interface ProviderVisibility {
  cursor: VisibilityMode;
  codex: VisibilityMode;
  deepseek: VisibilityMode;
}

interface AppSettings {
  cursorRefreshSec: number;
  systemRefreshSec: number;
  latencyTarget: string;
  highLatencyMs: number;
  providerVisibility: ProviderVisibility;
  providerOrder: ProviderId[];
  showSystemSection: boolean;
}

interface LocalSessionProbe {
  homesTried: string[];
  dbPathsFound: number;
  dbPathsOpenable: number;
  tokenLen?: number | null;
  failure?: string | null;
}

const PROVIDER_IDS: ProviderId[] = ["cursor", "codex", "deepseek"];
const DEFAULT_PROVIDER_ORDER: ProviderId[] = ["cursor", "codex", "deepseek"];
const DEFAULT_VISIBILITY: ProviderVisibility = {
  cursor: "auto",
  codex: "auto",
  deepseek: "auto",
};

const $ = <T extends HTMLElement>(id: string) =>
  document.getElementById(id) as T;

let panelState: PanelState | null = null;
/** 看板显隐/排序用的设置缓存；与后端 AppSettings 同步。 */
let currentSettings: AppSettings = {
  cursorRefreshSec: 300,
  systemRefreshSec: 15,
  latencyTarget: "https://cursor.com",
  highLatencyMs: 500,
  providerVisibility: { ...DEFAULT_VISIBILITY },
  providerOrder: [...DEFAULT_PROVIDER_ORDER],
  showSystemSection: true,
};
/** 主面板正在拖拽排序时为 true，避免 refresh 打断 DOM 顺序。 */
let providerDragActive = false;
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
  const next =
    tone === "ok" || tone === "warn" || tone === "danger" ? `tone-${tone}` : null;
  for (const cls of ["tone-ok", "tone-warn", "tone-danger"] as const) {
    el.classList.toggle(cls, cls === next);
  }
}

function setFillTone(fill: HTMLElement, tone: UsageTone): void {
  fill.classList.toggle("ok", tone === "ok");
  fill.classList.toggle("warn", tone === "warn");
  fill.classList.toggle("danger", tone === "danger");
}

function setTextIfChanged(el: HTMLElement, text: string): void {
  if (el.textContent !== text) el.textContent = text;
}

function setWidthIfChanged(el: HTMLElement, width: string): void {
  if (el.style.width !== width) el.style.width = width;
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
    setHiddenIfChanged(row, true);
    setWidthIfChanged(fill, "0%");
    setFillTone(fill, "neutral");
    applyTone(pctEl, "neutral");
    setTextIfChanged(pctEl, "—");
    return;
  }
  const used = clampPercent(percent);
  const tone = usageTone(used);
  setHiddenIfChanged(row, false);
  setWidthIfChanged(fill, `${used}%`);
  setFillTone(fill, tone);
  applyTone(pctEl, tone);
  setTextIfChanged(pctEl, `${formatPercent(used)} used`);
}

function setStatBar(
  barId: string,
  percent: number | null | undefined,
): void {
  const bar = $(barId);
  if (percent == null || Number.isNaN(percent)) {
    setWidthIfChanged(bar, "0%");
    return;
  }
  setWidthIfChanged(bar, `${clampPercent(percent)}%`);
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

function codexUsage(state: PanelState): UsageSnapshot | undefined {
  return state.usages.find((u) => u.provider === "codex");
}

function deepseekUsage(state: PanelState): UsageSnapshot | undefined {
  return state.usages.find((u) => u.provider === "deepseek");
}

function parseVisibilityMode(raw: unknown): VisibilityMode {
  if (raw === "always" || raw === "hidden" || raw === "auto") return raw;
  return "auto";
}

function normalizeProviderOrder(order: unknown): ProviderId[] {
  const result: ProviderId[] = [];
  if (Array.isArray(order)) {
    for (const item of order) {
      if (typeof item !== "string") continue;
      const id = item.trim().toLowerCase();
      if (
        (id === "cursor" || id === "codex" || id === "deepseek") &&
        !result.includes(id)
      ) {
        result.push(id);
      }
    }
  }
  for (const id of DEFAULT_PROVIDER_ORDER) {
    if (!result.includes(id)) result.push(id);
  }
  return result;
}

function normalizeSettings(raw: AppSettings): AppSettings {
  const vis = raw.providerVisibility ?? DEFAULT_VISIBILITY;
  return {
    cursorRefreshSec: raw.cursorRefreshSec,
    systemRefreshSec: raw.systemRefreshSec,
    latencyTarget: raw.latencyTarget,
    highLatencyMs: raw.highLatencyMs,
    providerVisibility: {
      cursor: parseVisibilityMode(vis.cursor),
      codex: parseVisibilityMode(vis.codex),
      deepseek: parseVisibilityMode(vis.deepseek),
    },
    providerOrder: normalizeProviderOrder(raw.providerOrder),
    showSystemSection: raw.showSystemSection !== false,
  };
}

/** Cursor：本机 session 或 Cookie（`hasCursorToken` 已聚合凭证存在性）。 */
function isConfigured(provider: ProviderId, state: PanelState): boolean {
  if (provider === "deepseek") return state.hasDeepseekKey;
  if (provider === "cursor") return state.hasCursorToken;
  // Codex：可读额度且非 needs_auth / 未安装
  const snap = codexUsage(state);
  if (!snap) return false;
  return snap.status === "ok";
}

function shouldShowProvider(mode: VisibilityMode, configured: boolean): boolean {
  if (mode === "hidden") return false;
  if (mode === "always") return true;
  return configured;
}

function providerDomNode(provider: ProviderId): HTMLElement | null {
  if (provider === "cursor") return document.getElementById("cursor-section");
  if (provider === "codex") return document.getElementById("card-codex");
  return document.getElementById("card-deepseek");
}

function setHiddenIfChanged(el: HTMLElement, hidden: boolean): void {
  if (el.classList.contains("hidden") === hidden) return;
  el.classList.toggle("hidden", hidden);
}

/** 当前 overview 内供应商节点顺序（含隐藏项，不含 System/Latency）。 */
function currentProviderDomOrder(): ProviderId[] {
  const overview = $("panel-overview");
  const ids: ProviderId[] = [];
  for (const child of Array.from(overview.children)) {
    const el = child as HTMLElement;
    const id = el.dataset.provider;
    if (isProviderId(id)) ids.push(id);
  }
  return ids;
}

function providerOrderMatchesDom(order: ProviderId[]): boolean {
  const domOrder = currentProviderDomOrder();
  if (domOrder.length !== order.length) return false;
  return order.every((id, i) => id === domOrder[i]);
}

function applyBoardLayout(state: PanelState): void {
  const overview = $("panel-overview");
  const systemCard = $("card-system");
  const latencyCard = $("card-latency");
  const order = normalizeProviderOrder(currentSettings.providerOrder);
  const showSystem = currentSettings.showSystemSection !== false;

  // 显隐：仅当结果相对上次变化时才改 class，避免 refresh 时先藏再显闪一下。
  for (const id of order) {
    const node = providerDomNode(id);
    if (!node) continue;
    const mode = parseVisibilityMode(currentSettings.providerVisibility[id]);
    const show = shouldShowProvider(mode, isConfigured(id, state));
    setHiddenIfChanged(node, !show);
  }
  setHiddenIfChanged(systemCard, !showSystem);
  setHiddenIfChanged(latencyCard, !showSystem);

  // 拖拽中不重排；排序仅在 order 或 System/Latency 相对位置变化时动手。
  if (!providerDragActive) {
    const needReorder =
      !providerOrderMatchesDom(order) ||
      overview.children[overview.children.length - 2] !== systemCard ||
      overview.children[overview.children.length - 1] !== latencyCard;

    if (needReorder) {
      for (const id of order) {
        const node = providerDomNode(id);
        if (!node) continue;
        overview.insertBefore(node, systemCard);
      }
      overview.appendChild(systemCard);
      overview.appendChild(latencyCard);
    }
  }

  const cursorVisible = shouldShowProvider(
    parseVisibilityMode(currentSettings.providerVisibility.cursor),
    isConfigured("cursor", state),
  );
  setHiddenIfChanged($("btn-compact-toggle"), !cursorVisible);
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

function renderCodex(state: PanelState): void {
  const snap = codexUsage(state);
  const membership = $("codex-membership");
  const detailEl = $("codex-detail");
  const periodEl = $("codex-period");
  const alertEl = $("codex-alert");

  if (!snap) {
    membership.textContent = "";
    setUsageBar("codex-usage-row", "codex-usage-progress", "codex-usage-pct", null);
    detailEl.textContent = "—";
    applyTone(detailEl, "neutral");
    periodEl.textContent = "";
    alertEl.classList.add("hidden");
    return;
  }

  membership.textContent = snap.membership ?? "";

  // 非 ok：明确失败/needs_auth，不渲染假成功进度条。
  if (snap.status !== "ok") {
    setUsageBar("codex-usage-row", "codex-usage-progress", "codex-usage-pct", null);
    if (snap.status === "needs_auth") {
      detailEl.textContent = "需要登录";
      applyTone(detailEl, "warn");
      alertEl.textContent =
        snap.message || "请先在本机 Codex / CLI 登录 ChatGPT";
    } else {
      detailEl.textContent = "本地不可用";
      applyTone(detailEl, "danger");
      alertEl.textContent = snap.message || "本地 Codex 不可用";
    }
    periodEl.textContent = "";
    alertEl.classList.remove("hidden");
    return;
  }

  setUsageBar(
    "codex-usage-row",
    "codex-usage-progress",
    "codex-usage-pct",
    snap.percentUsed,
  );
  const tone = usageTone(snap.percentUsed);
  detailEl.textContent = snap.message || formatPercent(snap.percentUsed);
  applyTone(detailEl, tone);
  periodEl.textContent = formatPeriodEnd(snap.periodEnd);
  alertEl.textContent = "";
  alertEl.classList.add("hidden");
}

function renderDeepSeek(state: PanelState): void {
  const snap = deepseekUsage(state);
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

  // 增量更新柱子，避免每次 refresh 销毁重建造成闪烁。
  while (container.children.length < slots) {
    const bar = document.createElement("div");
    bar.className = "spark-bar";
    container.appendChild(bar);
  }
  while (container.children.length > slots) {
    container.lastElementChild?.remove();
  }
  const bars = container.children;

  values.forEach((ms, i) => {
    const bar = bars[i] as HTMLElement | undefined;
    if (!bar) return;
    let height: string;
    let empty = false;
    let high = false;
    let placeholder = false;
    if (ms == null) {
      empty = true;
      height = "12%";
    } else {
      const pct = Math.max(10, Math.min(100, (ms / maxMs) * 100));
      height = `${pct}%`;
      high = !isPlaceholder && ms > highThreshold;
      placeholder = isPlaceholder;
    }
    if (bar.style.height !== height) bar.style.height = height;
    bar.classList.toggle("empty", empty);
    bar.classList.toggle("high", high);
    bar.classList.toggle("placeholder", placeholder);
  });
}

/** 出口区域 + 公网 IP 的 title 文案；IP 缺失时不拼接假数据。 */
function formatEgressTitle(
  regionLabel?: string | null,
  egressIp?: string | null,
): string {
  const region = regionLabel?.trim() || "出口暂不可用";
  const ip = egressIp?.trim();
  if (!ip) return region;
  return `${ip} · ${region}`;
}

function renderLatency(state: PanelState): void {
  const lat = state.latency;
  const el = $("latency-ms");
  const regionEl = $("latency-region");
  const ipEl = $("latency-egress-ip");
  const egressWrap = $("latency-egress");
  el.classList.remove("high");

  if (lat.fetchedAt) {
    pushLatencySample(lat);
  }

  if (!lat.fetchedAt) {
    el.textContent = "—";
    regionEl.textContent = "—";
    regionEl.classList.remove("muted-hint");
    ipEl.textContent = "";
    ipEl.hidden = true;
    egressWrap.title = "";
    renderLatencySpark(state.highLatencyMs);
    return;
  }

  const regionText = lat.regionLabel?.trim() || "出口暂不可用";
  const ip = lat.egressIp?.trim() || "";
  regionEl.textContent = regionText;
  regionEl.classList.toggle(
    "muted-hint",
    regionText === "出口暂不可用" || regionText === "—",
  );
  if (ip) {
    ipEl.textContent = ip;
    ipEl.hidden = false;
  } else {
    ipEl.textContent = "";
    ipEl.hidden = true;
  }
  egressWrap.title = formatEgressTitle(lat.regionLabel, lat.egressIp);

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
  renderCodex(state);
  renderDeepSeek(state);
  renderSystem(state);
  renderLatency(state);
  renderFooter(state);
  applyBoardLayout(state);
  scheduleAutoRefresh(state);
  requestAnimationFrame(() => {
    syncPanelScrollFade?.();
    syncSettingsScrollFade?.();
  });
}

let scheduledCursorMs: number | undefined;
let scheduledSystemMs: number | undefined;

function scheduleAutoRefresh(state: PanelState): void {
  const cursorMs = Math.max(60, state.autoRefreshSec) * 1000;
  const systemMs = Math.min(30, Math.max(10, state.systemRefreshSec)) * 1000;

  // 间隔未变时不要重置 timer，避免每次 render 打断计时并叠加刷新感。
  if (scheduledCursorMs !== cursorMs) {
    if (cursorTimer != null) window.clearInterval(cursorTimer);
    scheduledCursorMs = cursorMs;
    cursorTimer = window.setInterval(() => {
      void refreshProvidersOnly();
    }, cursorMs);
  }

  if (scheduledSystemMs !== systemMs) {
    if (systemTimer != null) window.clearInterval(systemTimer);
    scheduledSystemMs = systemMs;
    systemTimer = window.setInterval(() => {
      void refreshSystemAndLatency();
    }, systemMs);
  }
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
    // Codex 与 Cursor 同轮 best-effort；失败由快照状态表达，不中断其它卡。
    await invoke<UsageSnapshot>("refresh_codex");
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
  requestAnimationFrame(() => syncPanelScrollFade?.());
}

function showSettings(): void {
  const main = $("view-main");
  const settings = $("view-settings");
  main.classList.add("view-hidden");
  main.setAttribute("aria-hidden", "true");
  settings.classList.remove("view-hidden");
  settings.removeAttribute("aria-hidden");
  void loadSettingsForm();
  requestAnimationFrame(() => syncSettingsScrollFade?.());
}

function visibilityHintText(mode: VisibilityMode): string {
  if (mode === "hidden") return "已隐藏";
  if (mode === "always") return "始终显示";
  return "自动（已配置才显示）";
}

function readVisibilityMode(id: ProviderId): VisibilityMode {
  const el = document.getElementById(`visibility-${id}`);
  return parseVisibilityMode(el?.dataset.mode);
}

function setVisibilityUi(id: ProviderId, mode: VisibilityMode): void {
  const switchEl = document.getElementById(
    `visibility-${id}`,
  ) as HTMLButtonElement | null;
  const alwaysEl = document.getElementById(
    `visibility-${id}-always`,
  ) as HTMLButtonElement | null;
  const hintEl = document.getElementById(`visibility-${id}-hint`);
  if (!switchEl) return;

  const on = mode !== "hidden";
  switchEl.dataset.mode = mode;
  switchEl.setAttribute("aria-checked", on ? "true" : "false");
  if (alwaysEl) {
    alwaysEl.hidden = !on;
    alwaysEl.setAttribute("aria-pressed", mode === "always" ? "true" : "false");
  }
  if (hintEl) hintEl.textContent = visibilityHintText(mode);
}

function fillVisibilityControls(settings: AppSettings): void {
  for (const id of PROVIDER_IDS) {
    setVisibilityUi(id, settings.providerVisibility[id]);
  }
}

function setShowSystemSwitch(on: boolean): void {
  const el = $("show-system-section") as HTMLButtonElement;
  el.setAttribute("aria-checked", on ? "true" : "false");
}

function isShowSystemOn(): boolean {
  return (
    ($("show-system-section") as HTMLButtonElement).getAttribute(
      "aria-checked",
    ) === "true"
  );
}

async function persistProviderVisibility(
  id: ProviderId,
  mode: VisibilityMode,
): Promise<void> {
  const msg = $("settings-msg");
  const providerVisibility = {
    ...currentSettings.providerVisibility,
    [id]: mode,
  };
  try {
    await persistSettingsPatch({ providerVisibility });
    setVisibilityUi(id, mode);
    msg.textContent = "看板显示已保存";
    msg.className = "settings-msg ok";
  } catch (err) {
    msg.textContent = `保存失败：${String(err)}`;
    msg.className = "settings-msg error";
    fillVisibilityControls(currentSettings);
  }
}

async function persistSettingsPatch(
  patch: Partial<AppSettings>,
): Promise<AppSettings> {
  const saved = await invoke<AppSettings>("update_settings", { patch });
  currentSettings = normalizeSettings(saved);
  if (panelState) applyBoardLayout(panelState);
  return currentSettings;
}

function isProviderId(value: string | undefined): value is ProviderId {
  return value === "cursor" || value === "codex" || value === "deepseek";
}

/** 读取主面板当前可见供应商节点（DOM 顺序，不含 System/Latency）。 */
function visibleProviderNodes(): HTMLElement[] {
  const overview = $("panel-overview");
  const nodes: HTMLElement[] = [];
  for (const child of Array.from(overview.children)) {
    const el = child as HTMLElement;
    if (!isProviderId(el.dataset.provider)) continue;
    if (el.classList.contains("hidden")) continue;
    nodes.push(el);
  }
  return nodes;
}

/**
 * 将可见项的新顺序合并回完整 providerOrder：
 * 隐藏项保持在原序列槽位，可见槽位按 visibleOrdered 依次填入。
 */
function mergeVisibleIntoProviderOrder(
  fullOrder: ProviderId[],
  visibleOrdered: ProviderId[],
): ProviderId[] {
  const visibleSet = new Set(visibleOrdered);
  const queue = [...visibleOrdered];
  return fullOrder.map((id) => {
    if (!visibleSet.has(id)) return id;
    return queue.shift() ?? id;
  });
}

function clearProviderDropIndicators(): void {
  document
    .querySelectorAll(".provider-sortable.drop-before, .provider-sortable.drop-after")
    .forEach((el) => {
      el.classList.remove("drop-before", "drop-after");
    });
}

function bindProviderDragSort(): void {
  const overview = $("panel-overview");
  const DRAG_THRESHOLD_PX = 6;

  type DragState = {
    pointerId: number;
    source: HTMLElement;
    sourceId: ProviderId;
    startY: number;
    started: boolean;
    insertBefore: HTMLElement | null;
    /** insertBefore 为 null 时，插到最后一个可见供应商之后（System 之前） */
    insertAfterLast: boolean;
  };

  let drag: DragState | null = null;

  const endDrag = (cancelled: boolean) => {
    if (!drag) return;
    const { source, sourceId, insertBefore, started } = drag;
    try {
      source.releasePointerCapture(drag.pointerId);
    } catch {
      /* ignore */
    }
    source.classList.remove("provider-dragging");
    overview.classList.remove("provider-sorting");
    clearProviderDropIndicators();
    providerDragActive = false;
    drag = null;

    if (cancelled || !started) return;

    const visible = visibleProviderNodes().filter((n) => n !== source);
    const nextVisible: ProviderId[] = [];
    let placed = false;
    for (const node of visible) {
      if (insertBefore && node === insertBefore) {
        nextVisible.push(sourceId);
        placed = true;
      }
      const id = node.dataset.provider;
      if (isProviderId(id)) nextVisible.push(id);
    }
    if (!placed) nextVisible.push(sourceId);

    const full = normalizeProviderOrder(currentSettings.providerOrder);
    const merged = mergeVisibleIntoProviderOrder(full, nextVisible);
    const unchanged =
      merged.length === full.length &&
      merged.every((id, i) => id === full[i]);
    if (unchanged) return;

    const previousOrder = full;
    // 乐观更新 DOM，再持久化
    currentSettings = { ...currentSettings, providerOrder: merged };
    if (panelState) applyBoardLayout(panelState);

    void (async () => {
      try {
        await persistSettingsPatch({ providerOrder: merged });
      } catch (err) {
        console.error("persist providerOrder failed", err);
        currentSettings = {
          ...currentSettings,
          providerOrder: previousOrder,
        };
        if (panelState) applyBoardLayout(panelState);
      }
    })();
  };

  const updateDropTarget = (clientY: number) => {
    if (!drag) return;
    clearProviderDropIndicators();
    const others = visibleProviderNodes().filter((n) => n !== drag!.source);
    if (others.length === 0) {
      drag.insertBefore = null;
      drag.insertAfterLast = true;
      return;
    }

    let insertBefore: HTMLElement | null = null;
    let insertAfterLast = false;
    for (const node of others) {
      const rect = node.getBoundingClientRect();
      const mid = rect.top + rect.height / 2;
      if (clientY < mid) {
        insertBefore = node;
        break;
      }
    }
    if (!insertBefore) {
      insertAfterLast = true;
      const last = others[others.length - 1]!;
      last.classList.add("drop-after");
    } else {
      insertBefore.classList.add("drop-before");
    }
    drag.insertBefore = insertBefore;
    drag.insertAfterLast = insertAfterLast;
  };

  overview.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement | null;
    if (!target) return;
    // 避免与紧凑模式按钮等冲突（按钮在 overview 外；此处仅拦交互控件）
    if (target.closest("button, a, input, textarea, select")) return;

    const source = target.closest<HTMLElement>(".provider-sortable[data-provider]");
    if (!source || source.classList.contains("hidden")) return;
    if (!overview.contains(source)) return;
    const sourceId = source.dataset.provider;
    if (!isProviderId(sourceId)) return;
    if (visibleProviderNodes().length < 2) return;

    drag = {
      pointerId: e.pointerId,
      source,
      sourceId,
      startY: e.clientY,
      started: false,
      insertBefore: null,
      insertAfterLast: false,
    };
    try {
      source.setPointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
  });

  overview.addEventListener("pointermove", (e) => {
    if (!drag || e.pointerId !== drag.pointerId) return;
    const dy = Math.abs(e.clientY - drag.startY);
    if (!drag.started) {
      if (dy < DRAG_THRESHOLD_PX) return;
      drag.started = true;
      providerDragActive = true;
      drag.source.classList.add("provider-dragging");
      overview.classList.add("provider-sorting");
    }
    e.preventDefault();
    updateDropTarget(e.clientY);
  });

  overview.addEventListener("pointerup", (e) => {
    if (!drag || e.pointerId !== drag.pointerId) return;
    endDrag(false);
  });

  overview.addEventListener("pointercancel", (e) => {
    if (!drag || e.pointerId !== drag.pointerId) return;
    endDrag(true);
  });
}

async function loadSettingsForm(): Promise<void> {
  const msg = $("settings-msg");
  msg.textContent = "";
  msg.className = "settings-msg";
  try {
    const settings = normalizeSettings(
      await invoke<AppSettings>("get_settings"),
    );
    currentSettings = settings;
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
    setShowSystemSwitch(settings.showSystemSection !== false);
    fillVisibilityControls(settings);

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

function updateScrollFade(el: HTMLElement): void {
  const { scrollTop, scrollHeight, clientHeight } = el;
  const maxScroll = scrollHeight - clientHeight;
  const canScroll = maxScroll > 1;
  const atTop = scrollTop <= 1;
  const atBottom = scrollTop >= maxScroll - 1;
  el.classList.toggle("fade-top", canScroll && !atTop);
  el.classList.toggle("fade-bottom", canScroll && !atBottom);
}

function bindScrollFade(el: HTMLElement): () => void {
  const sync = () => updateScrollFade(el);
  el.addEventListener("scroll", sync, { passive: true });
  const ro = new ResizeObserver(sync);
  ro.observe(el);
  for (const child of el.children) {
    if (child instanceof HTMLElement) ro.observe(child);
  }
  sync();
  return sync;
}

let syncPanelScrollFade: (() => void) | undefined;
let syncSettingsScrollFade: (() => void) | undefined;

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

  requestAnimationFrame(() => syncPanelScrollFade?.());
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
        requestAnimationFrame(() => syncSettingsScrollFade?.());
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
  bindProviderDragSort();
  syncPanelScrollFade = bindScrollFade($("panel-overview"));
  syncSettingsScrollFade = bindScrollFade($("settings-form"));

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

  for (const id of PROVIDER_IDS) {
    const switchEl = document.getElementById(
      `visibility-${id}`,
    ) as HTMLButtonElement | null;
    const alwaysEl = document.getElementById(
      `visibility-${id}-always`,
    ) as HTMLButtonElement | null;
    const row = document.querySelector<HTMLElement>(
      `[data-switch-for="visibility-${id}"]`,
    );

    const toggleVisibility = () => {
      const current = readVisibilityMode(id);
      const next: VisibilityMode = current === "hidden" ? "auto" : "hidden";
      void persistProviderVisibility(id, next);
    };

    switchEl?.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleVisibility();
    });
    row?.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).closest(".ios-switch")) return;
      toggleVisibility();
    });
    alwaysEl?.addEventListener("click", (e) => {
      e.stopPropagation();
      const current = readVisibilityMode(id);
      if (current === "hidden") return;
      const next: VisibilityMode = current === "always" ? "auto" : "always";
      void persistProviderVisibility(id, next);
    });
  }

  {
    const sysSwitch = $("show-system-section") as HTMLButtonElement;
    const sysRow = document.querySelector<HTMLElement>(
      '[data-switch-for="show-system-section"]',
    );
    const toggleSystem = () => {
      void (async () => {
        const msg = $("settings-msg");
        const next = !isShowSystemOn();
        setShowSystemSwitch(next);
        try {
          await persistSettingsPatch({ showSystemSection: next });
          msg.textContent = "系统指标显示已保存";
          msg.className = "settings-msg ok";
        } catch (err) {
          msg.textContent = `保存失败：${String(err)}`;
          msg.className = "settings-msg error";
          setShowSystemSwitch(currentSettings.showSystemSection !== false);
        }
      })();
    };
    sysSwitch.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleSystem();
    });
    sysRow?.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).closest(".ios-switch")) return;
      toggleSystem();
    });
  }

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
      providerVisibility: {
        cursor: readVisibilityMode("cursor"),
        codex: readVisibilityMode("codex"),
        deepseek: readVisibilityMode("deepseek"),
      },
      providerOrder: normalizeProviderOrder(currentSettings.providerOrder),
      showSystemSection: isShowSystemOn(),
    };
    try {
      await persistSettingsPatch(patch);
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
  void (async () => {
    try {
      currentSettings = normalizeSettings(
        await invoke<AppSettings>("get_settings"),
      );
    } catch (err) {
      console.error("get_settings failed", err);
    }
    await refreshAll();
  })();

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
