import { invoke } from "@tauri-apps/api/core";
import { LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import {
  disable as disableAutostart,
  enable as enableAutostart,
  isEnabled as isAutostartEnabled,
} from "@tauri-apps/plugin-autostart";

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
  diskUsedBytes?: number | null;
  diskAvailableBytes?: number | null;
  vpnIp?: string | null;
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
  showLatencySection: boolean;
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

/** 与 tauri.conf.json / CSS 对齐的窗口尺寸约束 */
const PANEL_WIDTH = 360;
const PANEL_MIN_HEIGHT = 180;
const PANEL_MAX_HEIGHT_FALLBACK = 580;
/** 壳层边框占位（border-box 下避免裁切） */
const PANEL_CHROME_PX = 2;

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
  showLatencySection: true,
};
/** 主面板正在拖拽排序时为 true，避免 refresh 打断 DOM 顺序。 */
let providerDragActive = false;
let cursorTimer: number | undefined;
let systemTimer: number | undefined;
let refreshing = false;
/** 面板刚显示后的失焦保护窗口（毫秒时间戳） */
let ignoreBlurUntil = 0;
const BLUR_GRACE_MS = 350;
const SETTINGS_PROVIDER_COLLAPSED_KEY = "usages-settings-provider-collapsed";

type UsageTone = "ok" | "warn" | "danger" | "neutral";

/** 重置临近预警：剩余 < 24h → warn；< 6h（含已过重置点）→ danger */
const RESET_WARN_MS = 24 * 60 * 60 * 1000;
const RESET_DANGER_MS = 6 * 60 * 60 * 1000;

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

function worseTone(a: UsageTone, b: UsageTone): UsageTone {
  const rank: Record<UsageTone, number> = {
    neutral: 0,
    ok: 1,
    warn: 2,
    danger: 3,
  };
  return rank[a] >= rank[b] ? a : b;
}

/** periodEnd 距现在的重置紧迫度（非用量色阶）。 */
function periodResetTone(periodEnd?: string | null): UsageTone {
  if (!periodEnd) return "neutral";
  const end = new Date(periodEnd).getTime();
  if (Number.isNaN(end)) return "neutral";
  const remain = end - Date.now();
  if (remain <= RESET_DANGER_MS) return "danger";
  if (remain <= RESET_WARN_MS) return "warn";
  return "neutral";
}

/** 紧凑时钟：`8/10 15:57`（用于副行「重置 …」）。 */
function formatResetClock(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const timePart = d.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  return `${d.getMonth() + 1}/${d.getDate()} ${timePart}`;
}

/** 副行内联重置文案；已过重置点显示「已到重置点」。 */
function formatResetInline(periodEnd?: string | null): string {
  if (!periodEnd) return "";
  const d = new Date(periodEnd);
  if (Number.isNaN(d.getTime())) return "";
  if (d.getTime() <= Date.now()) return "已到重置点";
  const clock = formatResetClock(periodEnd);
  return clock ? `重置 ${clock}` : "";
}

type HeroSubPart = { text: string; tone?: UsageTone };

/** 拼装主卡副行；带 tone 的片段包成 `.hero-reset`，避免新增整行。 */
function setHeroSubParts(el: HTMLElement, parts: HeroSubPart[]): void {
  const items = parts.filter((p) => p.text.trim());
  const key = items.map((p) => `${p.text}\0${p.tone ?? ""}`).join("\n");
  if (el.dataset.subKey === key) return;
  el.dataset.subKey = key;
  el.replaceChildren();
  items.forEach((part, i) => {
    if (i > 0) el.appendChild(document.createTextNode(" · "));
    const tone = part.tone;
    if (tone === "warn" || tone === "danger") {
      const span = document.createElement("span");
      span.className = "hero-reset";
      applyTone(span, tone);
      span.textContent = part.text;
      el.appendChild(span);
    } else {
      el.appendChild(document.createTextNode(part.text));
    }
  });
}

function setTextIfChanged(el: HTMLElement, text: string): void {
  if (el.textContent !== text) el.textContent = text;
}

function setWidthIfChanged(el: HTMLElement, width: string): void {
  if (el.style.width !== width) el.style.width = width;
}

/** 设置进度条宽度；可叠加重置临近 tone（取更严重者）。 */
function setProgressWidth(
  fillId: string,
  percent: number | null | undefined,
  boostTone: UsageTone = "neutral",
): void {
  const fill = $(fillId);
  if (percent == null || Number.isNaN(percent)) {
    setWidthIfChanged(fill, "0%");
    fill.classList.remove("ok", "warn", "danger");
    return;
  }
  setWidthIfChanged(fill, `${clampPercent(percent)}%`);
  const tone = worseTone(usageTone(percent), boostTone);
  fill.classList.toggle("ok", tone === "ok");
  fill.classList.toggle("warn", tone === "warn");
  fill.classList.toggle("danger", tone === "danger");
}

function setHeroValue(
  numId: string,
  unitId: string,
  numText: string,
  unitText = "",
): void {
  setTextIfChanged($(numId), numText);
  setTextIfChanged($(unitId), unitText);
}

/** Cursor 主百分比：优先 Auto，其次聚合 percent，再 API。 */
function cursorPrimaryPercent(snap: UsageSnapshot): number | null {
  if (snap.autoPercentUsed != null && !Number.isNaN(snap.autoPercentUsed)) {
    return snap.autoPercentUsed;
  }
  if (snap.percentUsed != null && !Number.isNaN(snap.percentUsed)) {
    return snap.percentUsed;
  }
  if (snap.apiPercentUsed != null && !Number.isNaN(snap.apiPercentUsed)) {
    return snap.apiPercentUsed;
  }
  return null;
}

function formatHeroPeriod(
  membership?: string | null,
  periodEnd?: string | null,
): string {
  const parts: string[] = [];
  if (membership?.trim()) parts.push(membership.trim());
  const end = formatPeriodEnd(periodEnd);
  if (end) parts.push(end);
  return parts.join(" · ");
}

function formatCursorAmount(snap: UsageSnapshot): string {
  if (snap.unit === "cents") {
    const limit = snap.limit != null ? centsToDollars(snap.limit) : "—";
    return `${centsToDollars(snap.used)} / ${limit}`;
  }
  if (snap.limit != null) return `${snap.used} / ${snap.limit}`;
  return String(snap.used);
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
    // 旧设置仅有 showSystemSection 时：Latency 与之同值；后端也会迁移。
    showLatencySection:
      raw.showLatencySection !== undefined
        ? raw.showLatencySection !== false
        : raw.showSystemSection !== false,
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

/**
 * 主供应商 = 当前可见列表中排序第一项（与拖拽 order 一致）。
 * 其余可见供应商渲染为紧凑 strip。
 */
function applyPrimaryProviderRoles(): void {
  const visible = visibleProviderNodes();
  const primary = visible[0] ?? null;
  for (const id of PROVIDER_IDS) {
    const node = providerDomNode(id);
    if (!node) continue;
    if (node.classList.contains("hidden")) {
      node.classList.remove("is-primary", "is-strip");
      continue;
    }
    const isPrimary = node === primary;
    node.classList.toggle("is-primary", isPrimary);
    node.classList.toggle("is-strip", !isPrimary);
  }
}

function applyBoardLayout(state: PanelState): void {
  const overview = $("panel-overview");
  const secondaryBand = $("secondary-band");
  const systemCard = $("card-system");
  const latencyCard = $("card-latency");
  const order = normalizeProviderOrder(currentSettings.providerOrder);
  const showSystem = currentSettings.showSystemSection !== false;
  const showLatency = currentSettings.showLatencySection !== false;

  // 显隐：仅当结果相对上次变化时才改 class，避免 refresh 时先藏再显闪一下。
  for (const id of order) {
    const node = providerDomNode(id);
    if (!node) continue;
    const mode = parseVisibilityMode(currentSettings.providerVisibility[id]);
    const show = shouldShowProvider(mode, isConfigured(id, state));
    setHiddenIfChanged(node, !show);
  }
  setHiddenIfChanged(systemCard, !showSystem);
  setHiddenIfChanged(latencyCard, !showLatency);
  setHiddenIfChanged(secondaryBand, !showSystem && !showLatency);

  // 拖拽中不重排；排序仅在 order 或次级带相对位置变化时动手。
  if (!providerDragActive) {
    const needReorder =
      !providerOrderMatchesDom(order) ||
      overview.children[overview.children.length - 1] !== secondaryBand;

    if (needReorder) {
      for (const id of order) {
        const node = providerDomNode(id);
        if (!node) continue;
        overview.insertBefore(node, secondaryBand);
      }
      overview.appendChild(secondaryBand);
    }
  }

  const visibleCount = order.filter((id) => {
    const node = providerDomNode(id);
    return node != null && !node.classList.contains("hidden");
  }).length;
  overview.classList.toggle("provider-sort-disabled", visibleCount < 2);
  applyPrimaryProviderRoles();
  schedulePanelWindowResize();
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
  const periodEl = $("cursor-hero-period");
  const subEl = $("cursor-hero-sub");
  const stripPct = $("cursor-strip-pct");
  const alertEl = $("cursor-alert");

  const heroValue = $("cursor-hero-num").parentElement as HTMLElement;

  const apiRow = $("cursor-api-row");
  const stripApiRow = $("cursor-strip-api-row");
  const autoLabel = $("cursor-auto-label");
  const stripAutoLabel = $("cursor-strip-auto-label");

  if (!snap) {
    periodEl.textContent = "";
    setHeroValue("cursor-hero-num", "cursor-hero-unit", "—", "");
    setHeroSubParts(subEl, []);
    setProgressWidth("cursor-hero-fill", null);
    setProgressWidth("cursor-strip-fill", null);
    setProgressWidth("cursor-api-fill", null);
    setProgressWidth("cursor-strip-api-fill", null);
    apiRow.classList.add("hidden");
    apiRow.setAttribute("aria-hidden", "true");
    stripApiRow.classList.add("hidden");
    stripApiRow.setAttribute("aria-hidden", "true");
    autoLabel.classList.add("hidden");
    autoLabel.setAttribute("aria-hidden", "true");
    stripAutoLabel.classList.add("hidden");
    stripAutoLabel.setAttribute("aria-hidden", "true");
    setTextIfChanged(stripPct, "—");
    stripPct.removeAttribute("title");
    applyTone(heroValue, "neutral");
    applyTone(stripPct, "neutral");
    alertEl.classList.add("hidden");
    return;
  }

  // 周期信息放在副行（对齐 03b），顶部 period 节点仅作兼容占位
  periodEl.textContent = formatHeroPeriod(snap.membership, snap.periodEnd);
  const resetInline = formatResetInline(snap.periodEnd);
  const resetTone = periodResetTone(snap.periodEnd);
  const primary = cursorPrimaryPercent(snap);
  if (primary != null) {
    setHeroValue(
      "cursor-hero-num",
      "cursor-hero-unit",
      `${clampPercent(primary).toFixed(0)}`,
      "%",
    );
    setTextIfChanged(stripPct, formatPercent(primary));
  } else {
    setHeroValue("cursor-hero-num", "cursor-hero-unit", "—", "");
    setTextIfChanged(stripPct, "—");
  }
  // 主轨/主读数：Auto（或聚合 percent）；色阶按主百分比，重置紧迫度只影响数字与副行
  setProgressWidth("cursor-hero-fill", primary);
  setProgressWidth("cursor-strip-fill", primary);
  const tone = worseTone(usageTone(primary), resetTone);
  applyTone(heroValue, tone);
  applyTone(stripPct, tone);
  stripPct.title = resetInline || "";

  // API 轨：仅绑定 apiPercentUsed；与主轨独立色阶。主读数已回退到 API 时不重复一条轨
  const apiPct =
    snap.apiPercentUsed != null && !Number.isNaN(snap.apiPercentUsed)
      ? snap.apiPercentUsed
      : null;
  const showApiTrack =
    apiPct != null &&
    (snap.autoPercentUsed != null || snap.percentUsed != null);
  apiRow.classList.toggle("hidden", !showApiTrack);
  apiRow.setAttribute("aria-hidden", showApiTrack ? "false" : "true");
  stripApiRow.classList.toggle("hidden", !showApiTrack);
  stripApiRow.setAttribute("aria-hidden", showApiTrack ? "false" : "true");
  // Auto 标签与 API 轨同步：仅双轨时显示，单轨保持干净
  autoLabel.classList.toggle("hidden", !showApiTrack);
  autoLabel.setAttribute("aria-hidden", showApiTrack ? "false" : "true");
  stripAutoLabel.classList.toggle("hidden", !showApiTrack);
  stripAutoLabel.setAttribute("aria-hidden", showApiTrack ? "false" : "true");
  // 色阶只用 apiPct，勿混入主百分比或重置紧迫度
  setProgressWidth("cursor-api-fill", showApiTrack ? apiPct : null);
  setProgressWidth("cursor-strip-api-fill", showApiTrack ? apiPct : null);

  // 副行：金额额度 + 重置时间；不再写 API 文字百分比
  const subParts: HeroSubPart[] = [{ text: formatCursorAmount(snap) }];
  if (resetInline) {
    subParts.push({ text: resetInline, tone: resetTone });
  }
  setHeroSubParts(subEl, subParts);

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
}

function renderCodex(state: PanelState): void {
  const snap = codexUsage(state);
  const periodEl = $("codex-hero-period");
  const subEl = $("codex-hero-sub");
  const stripPct = $("codex-strip-pct");
  const alertEl = $("codex-alert");

  const heroValue = $("codex-hero-num").parentElement as HTMLElement;

  if (!snap) {
    periodEl.textContent = "";
    setHeroValue("codex-hero-num", "codex-hero-unit", "—", "");
    setHeroSubParts(subEl, []);
    setProgressWidth("codex-hero-fill", null);
    setProgressWidth("codex-strip-fill", null);
    setTextIfChanged(stripPct, "—");
    stripPct.removeAttribute("title");
    applyTone(heroValue, "neutral");
    applyTone(stripPct, "neutral");
    alertEl.classList.add("hidden");
    return;
  }

  periodEl.textContent = formatHeroPeriod(snap.membership, snap.periodEnd);
  const resetInline = formatResetInline(snap.periodEnd);
  const resetTone = periodResetTone(snap.periodEnd);

  // 非 ok：明确失败/needs_auth，不渲染假成功进度条。
  if (snap.status !== "ok") {
    setProgressWidth("codex-hero-fill", null);
    setProgressWidth("codex-strip-fill", null);
    setHeroValue("codex-hero-num", "codex-hero-unit", "—", "");
    applyTone(heroValue, "neutral");
    applyTone(stripPct, "neutral");
    stripPct.removeAttribute("title");
    if (snap.status === "needs_auth") {
      setHeroSubParts(subEl, [{ text: "需要登录" }]);
      setTextIfChanged(stripPct, "登录");
      alertEl.textContent =
        snap.message || "请先在本机 Codex / CLI 登录 ChatGPT";
    } else {
      setHeroSubParts(subEl, [{ text: "本地不可用" }]);
      setTextIfChanged(stripPct, "—");
      alertEl.textContent = snap.message || "本地 Codex 不可用";
    }
    alertEl.classList.remove("hidden");
    return;
  }

  const pct = snap.percentUsed;
  if (pct != null && !Number.isNaN(pct)) {
    setHeroValue(
      "codex-hero-num",
      "codex-hero-unit",
      `${clampPercent(pct).toFixed(0)}`,
      "%",
    );
    setTextIfChanged(stripPct, formatPercent(pct));
  } else {
    setHeroValue("codex-hero-num", "codex-hero-unit", "—", "");
    setTextIfChanged(stripPct, "—");
  }
  setProgressWidth("codex-hero-fill", pct, resetTone);
  setProgressWidth("codex-strip-fill", pct, resetTone);
  const tone = worseTone(usageTone(pct), resetTone);
  applyTone(heroValue, tone);
  applyTone(stripPct, tone);
  stripPct.title = resetInline || "";
  const subParts: HeroSubPart[] = [
    { text: snap.message?.trim() || "Rate limit" },
  ];
  if (resetInline) {
    subParts.push({ text: resetInline, tone: resetTone });
  }
  setHeroSubParts(subEl, subParts);
  alertEl.textContent = "";
  alertEl.classList.add("hidden");
}

function renderDeepSeek(state: PanelState): void {
  const snap = deepseekUsage(state);
  const periodEl = $("deepseek-hero-period");
  const subEl = $("deepseek-hero-sub");
  const stripPct = $("deepseek-strip-pct");
  const alertEl = $("deepseek-alert");

  if (!snap) {
    periodEl.textContent = "";
    setHeroValue("deepseek-hero-num", "deepseek-hero-unit", "—", "");
    subEl.textContent = "加载中…";
    setTextIfChanged(stripPct, "—");
    alertEl.classList.add("hidden");
    return;
  }

  periodEl.textContent = snap.membership?.trim() || "";

  if (snap.status === "needs_auth") {
    setHeroValue("deepseek-hero-num", "deepseek-hero-unit", "—", "");
    subEl.textContent = "需要 API Key";
    setTextIfChanged(stripPct, "—");
    alertEl.textContent = snap.message || "请在设置中配置 DeepSeek API Key";
    alertEl.classList.remove("hidden");
    return;
  }

  if (snap.status !== "ok") {
    setHeroValue("deepseek-hero-num", "deepseek-hero-unit", "—", "");
    subEl.textContent = snap.message || "查询失败";
    setTextIfChanged(stripPct, "—");
    alertEl.textContent = snap.message || "DeepSeek 余额查询失败";
    alertEl.classList.remove("hidden");
    return;
  }

  const total = snap.remaining ?? 0;
  const balance = formatBalanceAmount(snap.membership, total);
  setHeroValue("deepseek-hero-num", "deepseek-hero-unit", balance, "");
  setTextIfChanged(stripPct, balance);
  // 余额型：无进度条、无「无进度条/可用余额」类说明文案；金额本身即主读数
  subEl.textContent = "";

  if (snap.message) {
    alertEl.textContent = snap.message;
    alertEl.classList.remove("hidden");
  } else {
    alertEl.textContent = "";
    alertEl.classList.add("hidden");
  }
}

/** 容量数值（GiB，一位小数）。 */
function bytesToGbNumber(bytes: number): number {
  return bytes / (1024 * 1024 * 1024);
}

/** 已用 / 剩余，单位只写一次：`40.0 / 44.0 GB`。 */
function formatUsedRemaining(
  usedBytes: number | null | undefined,
  remainingBytes: number | null | undefined,
): string {
  if (
    usedBytes == null ||
    remainingBytes == null ||
    Number.isNaN(usedBytes) ||
    Number.isNaN(remainingBytes)
  ) {
    return "—";
  }
  return `${bytesToGbNumber(usedBytes).toFixed(1)} / ${bytesToGbNumber(remainingBytes).toFixed(1)} GB`;
}

function setNetValue(el: HTMLElement, value: string | null | undefined): void {
  const text = value?.trim() || "—";
  setTextIfChanged(el, text);
  el.classList.toggle("is-empty", text === "—");
}

/** 系统轨进度；与供应商进度条共用 usageTone 色阶。 */
function setSysTrack(
  fillId: string,
  percent: number | null | undefined,
): void {
  setProgressWidth(fillId, percent);
}

function renderSystem(state: PanelState): void {
  const sys = state.system;
  const hasData = !!sys.fetchedAt;
  const cpuEl = $("sys-cpu-pct");
  const gpuEl = $("sys-gpu-pct");
  const memEl = $("sys-mem-text");
  const diskEl = $("sys-disk-text");
  const vpnEl = $("sys-vpn-ip");

  const cpuPct = hasData ? sys.cpuPercent : null;
  setTextIfChanged(cpuEl, formatPercent(cpuPct));
  applyTone(cpuEl, usageTone(cpuPct));
  setSysTrack("sys-cpu-fill", cpuPct);

  if (hasData && sys.gpuPercent != null) {
    setTextIfChanged(gpuEl, formatPercent(sys.gpuPercent));
    applyTone(gpuEl, usageTone(sys.gpuPercent));
    setSysTrack("sys-gpu-fill", sys.gpuPercent);
  } else {
    setTextIfChanged(gpuEl, hasData ? "N/A" : "—");
    applyTone(gpuEl, "neutral");
    setSysTrack("sys-gpu-fill", null);
  }

  if (hasData && sys.memTotalBytes > 0) {
    // 后端已按 total−available 给出互补已用；此处钳制仅作防护
    const used = Math.min(sys.memUsedBytes, sys.memTotalBytes);
    const remaining = Math.max(0, sys.memTotalBytes - used);
    const memPct = (used / sys.memTotalBytes) * 100;
    setTextIfChanged(memEl, formatUsedRemaining(used, remaining));
    // MEM 文字保持中性；进度条仍按比例色
    applyTone(memEl, "neutral");
    setSysTrack("sys-mem-fill", memPct);
  } else {
    setTextIfChanged(memEl, "—");
    applyTone(memEl, "neutral");
    setSysTrack("sys-mem-fill", null);
  }

  if (
    hasData &&
    sys.diskUsedBytes != null &&
    sys.diskAvailableBytes != null
  ) {
    const diskTotal = sys.diskUsedBytes + sys.diskAvailableBytes;
    const diskPct = diskTotal > 0 ? (sys.diskUsedBytes / diskTotal) * 100 : null;
    // 展示已用/剩余数值，不带「已用」「剩余」字样
    setTextIfChanged(
      diskEl,
      formatUsedRemaining(sys.diskUsedBytes, sys.diskAvailableBytes),
    );
    // DISK 文字保持中性；进度条仍按比例色
    applyTone(diskEl, "neutral");
    setSysTrack("sys-disk-fill", diskPct);
  } else {
    setTextIfChanged(diskEl, "—");
    applyTone(diskEl, "neutral");
    setSysTrack("sys-disk-fill", null);
  }

  // 有 VPN 时与 IP 同行显示；无 VPN 时隐藏整段，避免「VPN —」空胶囊
  const vpnPart = $("net-vpn-part");
  const vpn = hasData ? sys.vpnIp?.trim() : "";
  if (vpn) {
    setTextIfChanged(vpnEl, vpn);
    vpnEl.classList.remove("is-empty");
    vpnPart.classList.remove("hidden");
  } else {
    setTextIfChanged(vpnEl, "");
    vpnEl.classList.remove("is-empty");
    vpnPart.classList.add("hidden");
  }
}

/** 出口区域 + 公网 IP 的 title 文案；IP 缺失时不拼接假数据。 */
function formatEgressTitle(
  regionLabel?: string | null,
  egressIp?: string | null,
  vpnIp?: string | null,
): string {
  const parts: string[] = [];
  const vpn = vpnIp?.trim();
  const ip = egressIp?.trim();
  const region = regionLabel?.trim();
  if (vpn) parts.push(`VPN ${vpn}`);
  if (ip) parts.push(`Public ${ip}`);
  if (region) parts.push(region);
  return parts.join(" · ");
}

function renderLatency(state: PanelState): void {
  const lat = state.latency;
  const el = $("latency-ms");
  const regionEl = $("latency-region");
  const ipEl = $("latency-egress-ip");
  const egressWrap = $("latency-egress");
  el.classList.remove("high", "warn");
  applyTone(el, "neutral");

  if (!lat.fetchedAt) {
    setTextIfChanged(el, "—");
    setNetValue(regionEl, null);
    regionEl.classList.add("muted-hint");
    setNetValue(ipEl, null);
    egressWrap.title = "";
    return;
  }

  const regionText = lat.regionLabel?.trim() || "—";
  setNetValue(regionEl, regionText === "—" ? null : regionText);
  regionEl.classList.toggle("muted-hint", !lat.regionLabel?.trim());
  setNetValue(ipEl, lat.egressIp);
  egressWrap.title = formatEgressTitle(
    lat.regionLabel,
    lat.egressIp,
    state.system.vpnIp,
  );

  if (lat.status !== "ok" || lat.latencyMs == null) {
    setTextIfChanged(el, lat.status === "timeout" ? "超时" : "错误");
    el.classList.add("high", "warn");
    applyTone(el, "danger");
    return;
  }

  setTextIfChanged(el, `${lat.latencyMs} ms`);
  if (lat.latencyMs > state.highLatencyMs) {
    el.classList.add("high", "warn");
    applyTone(el, "warn");
  } else {
    applyTone(el, "ok");
  }
}

function renderFooter(state: PanelState): void {
  $("footer-auto").textContent = "Auto";
  $("footer-updated").textContent = formatUpdated(latestFetchedAt(state));
}

let cardsReadyScheduled = false;
function markCardsReady(): void {
  if (cardsReadyScheduled || document.body.classList.contains("cards-ready")) {
    return;
  }
  cardsReadyScheduled = true;
  // 等首屏 card-appear（260ms + 最大 delay）播完再冻结，避免中途掐断
  window.setTimeout(() => {
    document.body.classList.add("cards-ready");
  }, 420);
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
  markCardsReady();
  requestAnimationFrame(() => {
    syncPanelScrollFade?.();
    syncSettingsScrollFade?.();
  });
}

function isDomVisible(el: HTMLElement): boolean {
  if (el.classList.contains("hidden")) return false;
  const style = getComputedStyle(el);
  return style.display !== "none" && style.visibility !== "hidden";
}

/** 按子节点自然高度求和（避免 flex:1 撑满时 scrollHeight≈clientHeight） */
function measureNaturalBlockHeight(el: HTMLElement): number {
  const style = getComputedStyle(el);
  const paddingY =
    (parseFloat(style.paddingTop) || 0) + (parseFloat(style.paddingBottom) || 0);
  const gap = parseFloat(style.rowGap || style.gap) || 0;
  const kids = Array.from(el.children).filter(
    (c): c is HTMLElement => c instanceof HTMLElement && isDomVisible(c),
  );
  if (kids.length === 0) return Math.ceil(paddingY);
  let content = 0;
  for (let i = 0; i < kids.length; i++) {
    content += kids[i].getBoundingClientRect().height;
    if (i < kids.length - 1) content += gap;
  }
  return Math.ceil(content + paddingY);
}

function readPanelMaxHeight(): number {
  const raw = getComputedStyle(document.documentElement)
    .getPropertyValue("--panel-max-height")
    .trim();
  const n = parseFloat(raw);
  return Number.isFinite(n) && n > 0 ? n : PANEL_MAX_HEIGHT_FALLBACK;
}

/** 当前可见视图的内容高度（未 clamp） */
function measureDesiredPanelHeight(): number {
  const settings = $("view-settings");
  const onSettings = !settings.classList.contains("view-hidden");
  const view = onSettings ? settings : $("view-main");
  let height = 0;
  for (const child of Array.from(view.children)) {
    if (!(child instanceof HTMLElement) || !isDomVisible(child)) continue;
    if (
      child.classList.contains("panel-body") ||
      child.classList.contains("settings-body")
    ) {
      height += measureNaturalBlockHeight(child);
    } else {
      height += child.getBoundingClientRect().height;
    }
  }
  return Math.ceil(height + PANEL_CHROME_PX);
}

let lastAppliedPanelHeight = 0;
let panelResizeRaf = 0;

function schedulePanelWindowResize(): void {
  if (panelResizeRaf) cancelAnimationFrame(panelResizeRaf);
  // 双 rAF：等 DOM/布局稳定后再量高
  panelResizeRaf = requestAnimationFrame(() => {
    panelResizeRaf = requestAnimationFrame(() => {
      panelResizeRaf = 0;
      void applyPanelWindowSize();
    });
  });
}

async function applyPanelWindowSize(): Promise<void> {
  if (providerDragActive) return;
  const desired = measureDesiredPanelHeight();
  const maxH = readPanelMaxHeight();
  const height = Math.max(PANEL_MIN_HEIGHT, Math.min(maxH, desired));
  if (Math.abs(height - lastAppliedPanelHeight) < 0.5) {
    syncPanelScrollFade?.();
    syncSettingsScrollFade?.();
    return;
  }
  try {
    await getCurrentWindow().setSize(new LogicalSize(PANEL_WIDTH, height));
    lastAppliedPanelHeight = height;
  } catch (err) {
    console.warn("panel setSize failed", err);
  }
  syncPanelScrollFade?.();
  syncSettingsScrollFade?.();
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
  schedulePanelWindowResize();
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
  schedulePanelWindowResize();
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

function setShowLatencySwitch(on: boolean): void {
  const el = $("show-latency-section") as HTMLButtonElement;
  el.setAttribute("aria-checked", on ? "true" : "false");
}

function isShowLatencyOn(): boolean {
  return (
    ($("show-latency-section") as HTMLButtonElement).getAttribute(
      "aria-checked",
    ) === "true"
  );
}

function setLaunchAtLoginSwitch(on: boolean): void {
  const el = $("launch-at-login") as HTMLButtonElement;
  el.setAttribute("aria-checked", on ? "true" : "false");
}

function isLaunchAtLoginOn(): boolean {
  return (
    ($("launch-at-login") as HTMLButtonElement).getAttribute("aria-checked") ===
    "true"
  );
}

function setLaunchAtLoginAvailable(available: boolean): void {
  const field = document.getElementById("launch-at-login-field");
  const el = document.getElementById(
    "launch-at-login",
  ) as HTMLButtonElement | null;
  if (!field || !el) return;
  field.hidden = !available;
  el.disabled = !available;
  el.setAttribute("aria-disabled", available ? "false" : "true");
}

async function syncLaunchAtLoginSwitch(): Promise<void> {
  // 本应用仅面向 macOS；非 macOS 隐藏开关。
  const isMac =
    typeof navigator !== "undefined" &&
    /Mac|iPhone|iPad|iPod/i.test(navigator.platform || navigator.userAgent);
  if (!isMac) {
    setLaunchAtLoginAvailable(false);
    return;
  }
  setLaunchAtLoginAvailable(true);
  try {
    setLaunchAtLoginSwitch(await isAutostartEnabled());
  } catch (err) {
    console.warn("读取开机启动状态失败", err);
    setLaunchAtLoginSwitch(false);
  }
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
  options?: { skipLayout?: boolean },
): Promise<AppSettings> {
  const saved = await invoke<AppSettings>("update_settings", { patch });
  currentSettings = normalizeSettings(saved);
  // 拖拽松手后 DOM 往往已是最终序；再 layout 会 insertBefore 造成闪一下。
  if (!options?.skipLayout && panelState) {
    applyBoardLayout(panelState);
  } else if (
    options?.skipLayout &&
    panelState &&
    !providerOrderMatchesDom(
      normalizeProviderOrder(currentSettings.providerOrder),
    )
  ) {
    applyBoardLayout(panelState);
  }
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

function bindProviderDragSort(): void {
  const overview = $("panel-overview");
  /** 超过该位移才算拖动；过小会把点击误判为 drag，并触发视觉锁导致闪烁 */
  const DRAG_THRESHOLD_PX = 6;
  const EDGE_SCROLL_PX = 28;
  const EDGE_SCROLL_STEP = 10;
  const FLIP_MS = 220;
  const SETTLE_MS = 180;
  const FLOAT_SCALE = 1.02;

  type DragState = {
    pointerId: number;
    source: HTMLElement;
    sourceId: ProviderId;
    startX: number;
    startY: number;
    started: boolean;
    settling: boolean;
    /** 按下时的完整 providerOrder，取消时回滚 */
    orderBefore: ProviderId[];
    grabOffsetX: number;
    grabOffsetY: number;
    placeholder: HTMLDivElement | null;
  };

  let drag: DragState | null = null;

  const detachDocListeners = () => {
    document.removeEventListener("pointermove", onDocPointerMove);
    document.removeEventListener("pointerup", onDocPointerUp);
    document.removeEventListener("pointercancel", onDocPointerCancel);
  };

  const restoreOrder = (order: ProviderId[]) => {
    const secondaryBand = $("secondary-band");
    for (const id of order) {
      const node = providerDomNode(id);
      if (!node) continue;
      overview.insertBefore(node, secondaryBand);
    }
    applyPrimaryProviderRoles();
  };

  /** 软锁：拦截 refresh / tray 失焦隐藏，但不改视觉 class（避免打断 card-appear）。 */
  const setPointerGuard = (active: boolean) => {
    providerDragActive = active;
    void invoke("set_panel_drag_active", { active }).catch(() => {
      /* 旧后端无此命令时忽略 */
    });
  };

  /** 硬锁视觉：仅真实拖动开始后挂上，结束时摘掉。 */
  const setDragVisualActive = (active: boolean) => {
    document.body.classList.toggle("provider-drag-active", active);
  };

  const setDragLock = (active: boolean) => {
    setPointerGuard(active);
    setDragVisualActive(active);
  };

  const clearFlipStyles = (nodes: HTMLElement[]) => {
    for (const n of nodes) {
      n.classList.remove("provider-flip");
      n.style.transition = "";
      n.style.transform = "";
    }
  };

  const clearFloatStyles = (source: HTMLElement) => {
    source.classList.remove("provider-drag-float", "provider-drag-settling");
    source.style.position = "";
    source.style.left = "";
    source.style.top = "";
    source.style.width = "";
    source.style.height = "";
    source.style.zIndex = "";
    source.style.pointerEvents = "";
    source.style.transform = "";
    source.style.transition = "";
    source.style.willChange = "";
    source.style.transformOrigin = "";
    source.style.boxShadow = "";
    source.style.padding = "";
    source.style.background = "";
    source.style.border = "";
    source.style.backdropFilter = "";
    source.style.removeProperty("-webkit-backdrop-filter");
  };

  /** 拖拽中 source 已挂到 body：按占位符位置拼出可见顺序 */
  const visibleOrderWithPlaceholder = (
    sourceId: ProviderId,
    placeholder: HTMLElement,
  ): ProviderId[] => {
    const ids: ProviderId[] = [];
    for (const child of Array.from(overview.children)) {
      if (child === placeholder) {
        ids.push(sourceId);
        continue;
      }
      const el = child as HTMLElement;
      const id = el.dataset.provider;
      if (!isProviderId(id) || el.classList.contains("hidden")) continue;
      ids.push(id);
    }
    return ids;
  };

  const measureTops = (nodes: HTMLElement[]) => {
    const map = new Map<HTMLElement, number>();
    for (const n of nodes) {
      map.set(n, n.getBoundingClientRect().top);
    }
    return map;
  };

  let flipGen = 0;

  /** FLIP：列表让位，220ms ease，避免瞬间 jump */
  const flipToNewLayout = (nodes: HTMLElement[], first: Map<HTMLElement, number>) => {
    const gen = ++flipGen;
    for (const n of nodes) {
      const prev = first.get(n);
      if (prev == null) continue;
      const next = n.getBoundingClientRect().top;
      const dy = prev - next;
      if (Math.abs(dy) < 0.5) continue;
      n.classList.add("provider-flip");
      n.style.transition = "none";
      n.style.transform = `translateY(${dy}px)`;
    }
    // 强制回流后再播到位动画
    void overview.offsetHeight;
    for (const n of nodes) {
      if (!n.classList.contains("provider-flip")) continue;
      n.style.transition = `transform ${FLIP_MS}ms var(--ease-mac)`;
      n.style.transform = "";
    }
    window.setTimeout(() => {
      if (gen !== flipGen) return;
      clearFlipStyles(nodes);
    }, FLIP_MS + 40);
  };

  const updateFloatPosition = (clientX: number, clientY: number) => {
    if (!drag) return;
    const x = clientX - drag.grabOffsetX;
    const y = clientY - drag.grabOffsetY;
    drag.source.style.transform = `translate3d(${x}px, ${y}px, 0) scale(${FLOAT_SCALE})`;
  };

  const beginFloat = (clientX: number, clientY: number) => {
    if (!drag) return;
    const { source } = drag;
    const rect = source.getBoundingClientRect();
    drag.grabOffsetX = clientX - rect.left;
    drag.grabOffsetY = clientY - rect.top;

    const placeholder = document.createElement("div");
    placeholder.className = "provider-drag-placeholder";
    placeholder.style.height = `${rect.height}px`;
    placeholder.setAttribute("aria-hidden", "true");
    overview.insertBefore(placeholder, source);
    drag.placeholder = placeholder;

    // 移出文档流，避免 live 重排时源节点干扰顺序
    document.body.appendChild(source);
    source.classList.add("provider-drag-float");
    source.style.position = "fixed";
    source.style.left = "0";
    source.style.top = "0";
    source.style.width = `${rect.width}px`;
    source.style.zIndex = "1000";
    source.style.pointerEvents = "none";
    source.style.transformOrigin = "0 0";
    source.style.willChange = "transform";
    source.style.transition = "none";
    updateFloatPosition(clientX, clientY);
  };

  const teardownFloatDom = (source: HTMLElement, placeholder: HTMLElement | null) => {
    if (placeholder?.isConnected) {
      overview.insertBefore(source, placeholder);
      placeholder.remove();
    } else if (!overview.contains(source)) {
      overview.insertBefore(source, $("secondary-band"));
    }
    clearFloatStyles(source);
  };

  const persistAfterDrop = (
    previousOrder: ProviderId[],
    nextVisible: ProviderId[],
  ) => {
    const full = normalizeProviderOrder(previousOrder);
    const merged = mergeVisibleIntoProviderOrder(full, nextVisible);
    const unchanged =
      merged.length === full.length &&
      merged.every((id, i) => id === full[i]);
    if (unchanged) {
      // DOM 已是目标序：勿再 applyBoardLayout，避免无谓重排闪烁
      applyPrimaryProviderRoles();
      setDragLock(false);
      return;
    }

    currentSettings = { ...currentSettings, providerOrder: merged };
    // 仅当乐观 DOM 与完整 order（含隐藏项槽位）不一致时才纠正
    if (panelState && !providerOrderMatchesDom(merged)) {
      applyBoardLayout(panelState);
    } else {
      applyPrimaryProviderRoles();
    }

    // 锁持续到写回完成，避免松手瞬间 refresh/focus 再跑 layout
    void (async () => {
      try {
        await persistSettingsPatch(
          { providerOrder: merged },
          { skipLayout: true },
        );
      } catch (err) {
        console.error("persist providerOrder failed", err);
        currentSettings = {
          ...currentSettings,
          providerOrder: previousOrder,
        };
        restoreOrder(previousOrder);
      } finally {
        setDragLock(false);
      }
    })();
  };

  const endDrag = (cancelled: boolean) => {
    if (!drag || drag.settling) return;
    const state = drag;
    const { source, started, orderBefore, placeholder } = state;
    detachDocListeners();
    try {
      if (source.hasPointerCapture(state.pointerId)) {
        source.releasePointerCapture(state.pointerId);
      }
    } catch {
      /* ignore */
    }
    overview.classList.remove("provider-sorting");

    if (!started) {
      // 纯点击：只解软锁，从未挂过视觉 class，不会重播入场动画
      drag = null;
      setPointerGuard(false);
      return;
    }

    if (cancelled) {
      teardownFloatDom(source, placeholder);
      clearFlipStyles(visibleProviderNodes());
      drag = null;
      currentSettings = { ...currentSettings, providerOrder: orderBefore };
      restoreOrder(orderBefore);
      setDragLock(false);
      return;
    }

    // settle：浮层落到占位最终位，再清浮层写回
    state.settling = true;
    const ph = placeholder;
    const nextVisible = ph
      ? visibleOrderWithPlaceholder(state.sourceId, ph)
      : visibleProviderNodes()
          .map((n) => n.dataset.provider)
          .filter(isProviderId);

    const finishSettle = () => {
      if (drag !== state) return;
      teardownFloatDom(source, ph);
      clearFlipStyles(visibleProviderNodes());
      drag = null;
      persistAfterDrop(orderBefore, nextVisible);
    };

    if (!ph?.isConnected) {
      finishSettle();
      return;
    }

    const target = ph.getBoundingClientRect();
    source.classList.add("provider-drag-settling");
    source.style.transition = `transform ${SETTLE_MS}ms var(--ease-mac), box-shadow ${SETTLE_MS}ms var(--ease-mac)`;
    source.style.transform = `translate3d(${target.left}px, ${target.top}px, 0) scale(1)`;

    let settled = false;
    const onSettleEnd = (ev: TransitionEvent) => {
      if (ev.propertyName !== "transform") return;
      if (settled) return;
      settled = true;
      source.removeEventListener("transitionend", onSettleEnd);
      finishSettle();
    };
    source.addEventListener("transitionend", onSettleEnd);
    window.setTimeout(() => {
      if (settled) return;
      settled = true;
      source.removeEventListener("transitionend", onSettleEnd);
      finishSettle();
    }, SETTLE_MS + 60);
  };

  const autoScrollIfNeeded = (clientY: number) => {
    const rect = overview.getBoundingClientRect();
    if (clientY < rect.top + EDGE_SCROLL_PX) {
      overview.scrollTop -= EDGE_SCROLL_STEP;
    } else if (clientY > rect.bottom - EDGE_SCROLL_PX) {
      overview.scrollTop += EDGE_SCROLL_STEP;
    }
  };

  /** 按指针 Y 移动占位符；其它卡 FLIP 让位 */
  const syncLiveOrder = (clientY: number) => {
    if (!drag?.started || drag.settling || !drag.placeholder) return;
    const { source, placeholder } = drag;
    const secondaryBand = $("secondary-band");
    const others = visibleProviderNodes().filter((n) => n !== source);

    if (others.length === 0) {
      if (placeholder.nextElementSibling !== secondaryBand) {
        overview.insertBefore(placeholder, secondaryBand);
      }
      return;
    }

    let insertBefore: HTMLElement | null = null;
    for (const node of others) {
      const rect = node.getBoundingClientRect();
      if (clientY < rect.top + rect.height / 2) {
        insertBefore = node;
        break;
      }
    }

    const ref = insertBefore ?? secondaryBand;
    if (placeholder.nextElementSibling === ref) return;

    const animNodes = [...others, placeholder];
    // 清掉未完成的 FLIP，避免测量到残留 transform
    clearFlipStyles(animNodes);
    const first = measureTops(animNodes);
    overview.insertBefore(placeholder, ref);
    flipToNewLayout(animNodes, first);
  };

  const onDocPointerMove = (e: PointerEvent) => {
    if (!drag || e.pointerId !== drag.pointerId || drag.settling) return;
    const dx = Math.abs(e.clientX - drag.startX);
    const dy = Math.abs(e.clientY - drag.startY);
    if (!drag.started) {
      if (dx < DRAG_THRESHOLD_PX && dy < DRAG_THRESHOLD_PX) return;
      drag.started = true;
      // 真正进入拖拽才上视觉锁 / 禁选，避免纯点击触发 card-appear 重播闪烁
      setDragVisualActive(true);
      overview.classList.add("provider-sorting");
      window.getSelection()?.removeAllRanges();
      e.preventDefault();
      beginFloat(e.clientX, e.clientY);
    }
    e.preventDefault();
    updateFloatPosition(e.clientX, e.clientY);
    autoScrollIfNeeded(e.clientY);
    syncLiveOrder(e.clientY);
  };

  const onDocPointerUp = (e: PointerEvent) => {
    if (!drag || e.pointerId !== drag.pointerId) return;
    endDrag(false);
  };

  const onDocPointerCancel = (e: PointerEvent) => {
    if (!drag || e.pointerId !== drag.pointerId) return;
    endDrag(true);
  };

  overview.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    if (drag) return;
    if (overview.classList.contains("provider-sort-disabled")) return;
    const target = e.target as HTMLElement | null;
    if (!target) return;
    // 避免与交互控件冲突
    if (target.closest("button, a, input, textarea, select, label")) return;

    const source = target.closest<HTMLElement>(
      ".provider-sortable[data-provider]",
    );
    if (!source || source.classList.contains("hidden")) return;
    if (!overview.contains(source)) return;
    const sourceId = source.dataset.provider;
    if (!isProviderId(sourceId)) return;
    if (visibleProviderNodes().length < 2) return;

    // 软锁：阈值前挡住 refresh / 失焦隐藏；视觉锁延后到真正开始拖动
    setPointerGuard(true);
    drag = {
      pointerId: e.pointerId,
      source,
      sourceId,
      startX: e.clientX,
      startY: e.clientY,
      started: false,
      settling: false,
      orderBefore: normalizeProviderOrder(currentSettings.providerOrder),
      grabOffsetX: 0,
      grabOffsetY: 0,
      placeholder: null,
    };

    document.addEventListener("pointermove", onDocPointerMove, {
      passive: false,
    });
    document.addEventListener("pointerup", onDocPointerUp);
    document.addEventListener("pointercancel", onDocPointerCancel);

    try {
      source.setPointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
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
    setShowLatencySwitch(settings.showLatencySection !== false);
    fillVisibilityControls(settings);
    await syncLaunchAtLoginSwitch();

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
        schedulePanelWindowResize();
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
  bindSettingsProviderCollapse();
  bindProviderDragSort();
  syncPanelScrollFade = bindScrollFade($("panel-overview"));
  syncSettingsScrollFade = bindScrollFade($("settings-form"));

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

  {
    const latSwitch = $("show-latency-section") as HTMLButtonElement;
    const latRow = document.querySelector<HTMLElement>(
      '[data-switch-for="show-latency-section"]',
    );
    const toggleLatency = () => {
      void (async () => {
        const msg = $("settings-msg");
        const next = !isShowLatencyOn();
        setShowLatencySwitch(next);
        try {
          await persistSettingsPatch({ showLatencySection: next });
          msg.textContent = "网络延迟显示已保存";
          msg.className = "settings-msg ok";
        } catch (err) {
          msg.textContent = `保存失败：${String(err)}`;
          msg.className = "settings-msg error";
          setShowLatencySwitch(currentSettings.showLatencySection !== false);
        }
      })();
    };
    latSwitch.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleLatency();
    });
    latRow?.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).closest(".ios-switch")) return;
      toggleLatency();
    });
  }

  {
    const loginSwitch = $("launch-at-login") as HTMLButtonElement;
    const loginRow = document.querySelector<HTMLElement>(
      '[data-switch-for="launch-at-login"]',
    );
    const toggleLaunchAtLogin = () => {
      void (async () => {
        if (loginSwitch.disabled) return;
        const msg = $("settings-msg");
        const next = !isLaunchAtLoginOn();
        setLaunchAtLoginSwitch(next);
        try {
          if (next) {
            await enableAutostart();
          } else {
            await disableAutostart();
          }
          // 以系统 Login Item 为准再同步一次
          setLaunchAtLoginSwitch(await isAutostartEnabled());
          msg.textContent = next
            ? "已开启开机启动"
            : "已关闭开机启动";
          msg.className = "settings-msg ok";
        } catch (err) {
          msg.textContent = `开机启动设置失败：${String(err)}`;
          msg.className = "settings-msg error";
          try {
            setLaunchAtLoginSwitch(await isAutostartEnabled());
          } catch {
            setLaunchAtLoginSwitch(!next);
          }
        }
      })();
    };
    loginSwitch.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleLaunchAtLogin();
    });
    loginRow?.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).closest(".ios-switch")) return;
      toggleLaunchAtLogin();
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
      showLatencySection: isShowLatencyOn(),
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
      // 拖拽中勿 refresh，避免打断排序
      if (!providerDragActive) void refreshAll();
      return;
    }
    // 拖拽中保持面板，避免 tray 失焦 hide 把拖拽掐断
    if (providerDragActive) return;
    if (Date.now() < ignoreBlurUntil) return;
    void win.hide();
  });
});
