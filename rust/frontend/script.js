const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const TRIM_COMPARE_EPSILON = 0.001;
const MIN_TRIM_GAP_SECONDS = 0.01;
const TRIM_KEY_STEP_SECONDS = 1;
const TRIM_KEY_BIG_STEP_SECONDS = 10;
const PLAYLIST_ONLY_URL = /^https?:\/\/(?:[\w-]+\.)?youtube\.com\/playlist\?(?:\S*&)?list=/i;
const YOUTUBE_VIDEO_ID = /^https?:\/\/(?:[\w-]+\.)?(?:youtube\.com\/(?:watch\?(?:\S*&)?v=|shorts\/|embed\/|live\/)|youtu\.be\/)([\w-]{11})(?![\w-])/i;
const HTTP_URL = /^https?:\/\/\S+$/i;
const GRAPH_WINDOW_MS = 30000;
const GRAPH_MIN_SPAN_MS = 5000;
const GRAPH_STEP_MS = 100;
const GRAPH_SMOOTHING_MS = 900;
const HISTORY_SEARCH_MIN_ITEMS = 6;
const SVG_NS = 'http://www.w3.org/2000/svg';
const SEGMENT_LABELS = { sponsor: "Sponsor", selfpromo: "Kendi tanıtımı", interaction: "Abone ol hatırlatması" };

const $ = (id) => document.getElementById(id);
const IS_ANDROID = /Android/i.test(navigator.userAgent);

const el = {
    urlInput: $('urlInput'),
    clearUrlBtn: $('clearUrlBtn'),
    pasteBtn: $('pasteBtn'),
    clipChip: $('clipChip'),
    clipText: $('clipText'),
    clipAddBtn: $('clipAddBtn'),
    clipDismissBtn: $('clipDismissBtn'),
    modeVideo: $('modeVideo'),
    modeAudio: $('modeAudio'),
    qualitySelect: $('qualitySelect'),
    audioNote: $('audioNote'),
    playlistToggle: $('playlistToggle'),
    sponsorToggle: $('sponsorToggle'),
    previewCard: $('previewCard'),
    previewThumb: $('previewThumb'),
    videoTitle: $('videoTitleLabel'),
    videoDuration: $('videoDurationLabel'),
    segmentBar: $('segmentBar'),
    segmentCaption: $('segmentCaption'),
    trimmingSection: $('trimmingSection'),
    rangeMin: $('rangeMin'),
    rangeMax: $('rangeMax'),
    sliderTrack: $('sliderTrack'),
    startTimeInput: $('startTimeInput'),
    endTimeInput: $('endTimeInput'),
    trimLength: $('trimLengthLabel'),
    trimResetBtn: $('trimResetBtn'),
    dirInput: $('dirInput'),
    browseBtn: $('browseBtn'),
    composer: document.querySelector('.composer'),
    downloadBtn: $('downloadBtn'),
    btnLabel: $('btnLabel'),
    status: $('statusMessage'),
    progressContainer: $('progressContainer'),
    progressBar: $('progressBar'),
    progressPercent: $('progressPercent'),
    progressThumb: $('progressThumb'),
    playlistCounter: $('playlistCounter'),
    downloadSpeed: $('downloadSpeed'),
    progressInfo: $('progressInfo'),
    speedGraph: $('speedGraph'),
    queueList: $('queueList'),
    cancelBtn: $('cancelBtn'),
    resultCard: $('resultCard'),
    resultThumb: $('resultThumb'),
    resultTitle: $('resultTitle'),
    resultMeta: $('resultMeta'),
    filePath: $('filePath'),
    openFileBtn: $('openFileBtn'),
    openFolderBtn: $('openFolderBtn'),
    redownloadBtn: $('redownloadBtn'),
    progressTitle: $('progressTitle'),
    progressTrack: $('progressTrack'),
    batchCounter: $('batchCounter'),
    historySection: $('historySection'),
    historyCount: $('historyCount'),
    historySearch: $('historySearch'),
    historyList: $('historyList'),
    historyEmpty: $('historyEmpty'),
    clearHistoryBtn: $('clearHistoryBtn'),
    updateBanner: $('updateBanner'),
    updateText: $('updateText'),
    updateBtn: $('updateBtn'),
    appUpdateBanner: $('appUpdateBanner'),
    appUpdateText: $('appUpdateText'),
    appUpdateBtn: $('appUpdateBtn'),
    appVersion: $('appVersion'),
    themeToggle: $('themeToggle'),
    shortcutsBtn: $('shortcutsBtn'),
    shortcutsDialog: $('shortcutsDialog'),
    patchNotesDialog: $('patchNotesDialog'),
    signInRow: $('signInRow'),
    signInBtn: $('signInBtn'),
    cookiesBtn: $('cookiesBtn'),
    cookiesDialog: $('cookiesDialog'),
    cookiesState: $('cookiesState'),
    importCookiesBtn: $('importCookiesBtn'),
    clearCookiesBtn: $('clearCookiesBtn'),
    patchNotesTitle: $('patchNotesTitle'),
    patchNotesBody: $('patchNotesBody'),
    dropOverlay: $('dropOverlay'),
    ambient: $('ambient'),
};

const ambientImages = [...el.ambient.querySelectorAll('.ambient-img')];
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

let currentSavedPath = "";
let lastRequest = null;
let videoDuration = 0;
let trimSelectionTouched = false;
let infoRequestId = 0;
let lastFetchedUrl = "";
let lastInfo = null;
let lastSegments = null;
let segmentRequestId = 0;
let pendingInfoUrl = "";
let lastInfoPlaylist = false;
let pendingInfoPlaylist = false;
let updatePollActive = false;
let isDownloading = false;
let inputDebounceTimer = null;
let playlistAutoEnabled = false;
let cancelRequested = false;

let itemAnnounced = false;
let activeQueueRow = null;
let historyItems = [];
let clipLinks = [];
let clipDismissedKey = "";
const usedLinks = new Set();
const failedThumbs = new Set();
let internalDrag = false;
let dragDepth = 0;
let ambientUrl = "";
let ambientSlot = 0;
const graph = { points: [], target: 0, value: 0, peak: 1, startedAt: 0, lastFrame: 0, frame: 0 };

function errorText(error, fallback) {
    if (typeof error === 'string' && error) return error;
    return (error && error.message) || fallback;
}

function setStatus(text, kind) {
    el.status.textContent = text;
    el.status.className = kind && text ? `status ${kind}` : 'status';
}

function icon(name, extraClass) {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('class', extraClass ? `ico ${extraClass}` : 'ico');
    svg.setAttribute('aria-hidden', 'true');
    const use = document.createElementNS(SVG_NS, 'use');
    use.setAttribute('href', `#i-${name}`);
    svg.append(use);
    return svg;
}

function parseLinks(text) {
    const links = [];
    for (const line of String(text).split(/\r?\n/)) {
        const entry = line.trim();
        if (!entry) continue;
        const tokens = entry.split(/\s+/);
        const parts = tokens.length > 1 && tokens.every((token) => HTTP_URL.test(token)) ? tokens : [entry];
        for (const part of parts) {
            if (!links.includes(part)) links.push(part);
        }
    }
    return links;
}

function hostLabel(url) {
    try {
        return new URL(url).hostname.replace(/^(www|m)\./i, '');
    } catch {
        return "";
    }
}

function shortUrl(url) {
    return String(url).replace(/^https?:\/\/(www\.)?/i, '');
}

function youtubeThumbnail(url) {
    const match = YOUTUBE_VIDEO_ID.exec(url);
    return match ? `https://i.ytimg.com/vi/${match[1]}/mqdefault.jpg` : "";
}

function autosizeUrlInput() {
    el.urlInput.style.height = 'auto';
    el.urlInput.style.height = `${el.urlInput.scrollHeight}px`;
}

function setUrlValue(text) {
    el.urlInput.value = text;
    autosizeUrlInput();
    refreshUrlControls();
}

function refreshUrlControls() {
    el.clearUrlBtn.classList.toggle('hidden', !el.urlInput.value);
    refreshDownloadLabel();
}

function formatTimeLimit(seconds) {
    const sec = Math.max(0, Number(seconds) || 0);
    const totalCs = Math.round(sec * 100);
    const cs = totalCs % 100;
    const totalSec = Math.floor(totalCs / 100);
    const s = totalSec % 60;
    const m = Math.floor(totalSec / 60) % 60;
    const h = Math.floor(totalSec / 3600);
    let res = `${h.toString().padStart(2, '0')}:${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
    if (cs > 0) {
        res += `.${cs.toString().padStart(2, '0')}`;
    }
    return res;
}

function formatClock(seconds) {
    const total = Math.max(0, Math.round(Number(seconds) || 0));
    const s = (total % 60).toString().padStart(2, '0');
    const m = Math.floor(total / 60) % 60;
    const h = Math.floor(total / 3600);
    return h > 0 ? `${h}:${m.toString().padStart(2, '0')}:${s}` : `${m}:${s}`;
}

function formatBytes(bytes) {
    const value = Number(bytes) || 0;
    if (value <= 0) return "";
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const exponent = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
    const scaled = value / 1024 ** exponent;
    const digits = exponent >= 2 && scaled < 100 ? 1 : 0;
    return `${scaled.toLocaleString('tr-TR', { maximumFractionDigits: digits })} ${units[exponent]}`;
}

function parseTimeInput(timeStr) {
    if (!timeStr) return 0;
    const parts = String(timeStr).trim().replace(',', '.').split(':');
    if (parts.length > 3) return 0;
    let totalSeconds = 0;
    for (const part of parts) {
        totalSeconds = totalSeconds * 60 + Number(part);
    }
    return Number.isFinite(totalSeconds) ? Math.max(0, totalSeconds) : 0;
}

function clampTime(seconds, min, max) {
    return Math.min(Math.max(seconds, min), max);
}

function audioOnly() {
    return el.modeAudio.checked;
}

function selectedQuality() {
    return el.qualitySelect.value;
}

function trimAvailable() {
    return !el.playlistToggle.checked && videoDuration > 0;
}

function refreshTrimVisibility() {
    el.trimmingSection.classList.toggle('hidden', !trimAvailable());
    refreshDownloadLabel();
}

function refreshFormatControls() {
    const audio = audioOnly();
    el.qualitySelect.parentElement.classList.toggle('hidden', audio);
    el.audioNote.classList.toggle('hidden', !audio);
    el.qualitySelect.disabled = isDownloading || audio;
    refreshDownloadLabel();
}

function refreshDownloadLabel() {
    const count = parseLinks(el.urlInput.value).length;
    const trimming = count <= 1 && selectedTrimRange() !== null;
    let label = "İndir";
    if (count > 1) {
        label = `${count} bağlantıyı indir`;
    } else if (trimming) {
        label = audioOnly() ? "Kesiti MP3 olarak indir" : "Kesiti indir";
    } else if (el.playlistToggle.checked) {
        label = audioOnly() ? "Listeyi MP3 olarak indir" : "Listeyi indir";
    } else if (audioOnly()) {
        label = "MP3 olarak indir";
    }
    el.btnLabel.textContent = label;
}

function setThumb(img, url) {
    if (url && !failedThumbs.has(url)) {
        if (img.getAttribute('src') !== url) img.src = url;
        img.classList.remove('hidden');
    } else {
        img.removeAttribute('src');
        img.classList.add('hidden');
    }
    refreshAmbient();
}

function setAmbient(url) {
    if (url === ambientUrl) return;
    ambientUrl = url;
    const current = ambientImages[ambientSlot];
    if (!url) {
        current.classList.remove('show');
        el.ambient.classList.remove('has-image');
        return;
    }
    const incoming = ambientImages[ambientSlot ^ 1];
    incoming.onload = () => {
        if (ambientUrl !== url) return;
        current.classList.remove('show');
        incoming.classList.add('show');
        el.ambient.classList.add('has-image');
        ambientSlot ^= 1;
    };
    incoming.onerror = () => {
        if (ambientUrl === url) setAmbient("");
    };
    incoming.src = url;
}

function shownThumb(img) {
    return img.classList.contains('hidden') ? "" : (img.getAttribute('src') || "");
}

function refreshAmbient() {}

function showPreview(state, info = {}) {
    if (state === 'hidden') {
        el.previewCard.className = 'preview hidden';
        refreshAmbient();
        return;
    }
    const ready = state === 'ready';
    el.previewCard.className = `preview ${state}`;
    el.videoTitle.textContent = info.title || "";
    el.videoTitle.title = ready ? (info.title || "") : "";
    el.videoDuration.textContent = [
        info.site,
        info.meta,
        ready && !info.meta && info.duration > 0 ? formatClock(info.duration) : "",
    ].filter(Boolean).join(" · ");
    setThumb(el.previewThumb, ready ? info.thumbnail : "");
}

function showSegments() {
    const asPlaylist = el.playlistToggle.checked && lastInfo && lastInfo.playlist_title;
    const overview = lastInfo && !asPlaylist ? lastSegments : null;
    const segments = overview && videoDuration > 0 ? overview.segments : [];
    const idle = overview ? `SponsorBlock: ${overview.summary}` : "";
    el.segmentBar.classList.toggle('hidden', segments.length === 0);
    el.segmentCaption.classList.toggle('hidden', !overview);
    el.segmentCaption.textContent = idle;
    const spanMark = (className, start, end) => {
        const mark = document.createElement('span');
        mark.className = className;
        mark.style.left = `${(start / videoDuration) * 100}%`;
        mark.style.width = `${((end - start) / videoDuration) * 100}%`;
        return mark;
    };
    const spans = segments.map((segment) => {
        const start = clampTime(segment.start, 0, videoDuration);
        return { category: segment.category, start, end: clampTime(segment.end, start, videoDuration) };
    });
    el.segmentBar.replaceChildren(...spans.map(({ category, start, end }) => {
        const label = `${SEGMENT_LABELS[category] || category} · ${formatClock(start)}–${formatClock(end)}`;
        const mark = spanMark('segment', start, end);
        mark.tabIndex = 0;
        mark.title = label;
        mark.setAttribute('role', 'listitem');
        mark.setAttribute('aria-label', label);
        for (const type of ['mouseenter', 'focus']) {
            mark.addEventListener(type, () => {
                el.segmentCaption.textContent = label;
            });
        }
        for (const type of ['mouseleave', 'blur']) {
            mark.addEventListener(type, () => {
                el.segmentCaption.textContent = idle;
            });
        }
        return mark;
    }));
    el.sliderTrack.replaceChildren(...spans.map(({ start, end }) => spanMark('trim-mark', start, end)));
}

function clearSegments() {
    segmentRequestId++;
    lastSegments = null;
    showSegments();
}

async function loadSegments(url) {
    const reqId = ++segmentRequestId;
    let overview = null;
    try {
        overview = await invoke('get_sponsor_segments', { url });
    } catch (e) {
        console.error("Failed to look up SponsorBlock segments", e);
    }
    if (reqId !== segmentRequestId) return;
    lastSegments = overview;
    showSegments();
}

function resetVideoInfo() {
    infoRequestId++;
    clearSegments();
    videoDuration = 0;
    lastFetchedUrl = "";
    lastInfo = null;
    pendingInfoUrl = "";
    trimSelectionTouched = false;
    showPreview('hidden');
    refreshTrimVisibility();
}

function showBatchPreview(count) {
    resetVideoInfo();
    showPreview('batch', {
        title: `${count} bağlantı sırayla indirilecek`,
        meta: "Seçili biçim ve kalite hepsine uygulanır.",
    });
}

function showFetchedInfo() {
    if (!lastInfo) return;
    const asPlaylist = el.playlistToggle.checked && lastInfo.playlist_title;
    showPreview('ready', {
        site: hostLabel(lastFetchedUrl),
        title: asPlaylist ? lastInfo.playlist_title : (lastInfo.title || "Başlıksız video"),
        duration: Number(lastInfo.duration) || 0,
        meta: asPlaylist
            ? (lastInfo.playlist_count ? `Oynatma listesi · ${lastInfo.playlist_count} video` : "Oynatma listesi")
            : "",
        thumbnail: lastInfo.thumbnail,
    });
    showSegments();
}

function infoCoversMode(playlist) {
    return lastInfoPlaylist === playlist || lastInfoPlaylist !== Boolean(lastInfo.playlist_title);
}

async function fetchInfoForUrl() {
    if (isDownloading) return;
    const links = parseLinks(el.urlInput.value);
    if (links.length === 0) {
        resetVideoInfo();
        return;
    }
    if (links.length > 1) {
        showBatchPreview(links.length);
        return;
    }
    const url = links[0];
    const playlist = el.playlistToggle.checked;
    if (url === pendingInfoUrl && playlist === pendingInfoPlaylist) return;
    if (url === lastFetchedUrl && infoCoversMode(playlist)) return;

    const keptDuration = url === lastFetchedUrl ? videoDuration : 0;
    const reqId = ++infoRequestId;
    pendingInfoUrl = url;
    pendingInfoPlaylist = playlist;
    lastFetchedUrl = "";
    lastInfo = null;
    videoDuration = 0;
    clearSegments();
    refreshTrimVisibility();
    showPreview('loading', { site: hostLabel(url), title: "Bilgi alınıyor..." });

    let data = null;
    let failure = "";
    try {
        data = await invoke('get_video_info', { url, playlist });
    } catch (e) {
        failure = errorText(e, "Video bilgisi alınamadı.");
    }
    if (reqId !== infoRequestId) return;
    pendingInfoUrl = "";
    if (isDownloading) return;

    if (!data) {
        showPreview('error', { title: failure });
        return;
    }

    lastFetchedUrl = url;
    lastInfo = data;
    lastInfoPlaylist = playlist;
    videoDuration = Number(data.duration) || 0;
    showFetchedInfo();
    loadSegments(url);
    if (videoDuration > 0 && Math.abs(videoDuration - keptDuration) >= 0.5) {
        setupSliders();
    }
    refreshTrimVisibility();
}

function setupSliders() {
    el.rangeMin.max = videoDuration;
    el.rangeMax.max = videoDuration;
    el.rangeMin.value = 0;
    el.rangeMax.value = videoDuration;
    trimSelectionTouched = false;

    el.startTimeInput.value = formatTimeLimit(0);
    el.endTimeInput.value = formatTimeLimit(videoDuration);

    updateSliderTrack();
}

function updateSliderTrack() {
    if (videoDuration <= 0) return;

    const minVal = parseFloat(el.rangeMin.value);
    const maxVal = parseFloat(el.rangeMax.value);
    const minPercent = (minVal / videoDuration) * 100;
    const maxPercent = (maxVal / videoDuration) * 100;

    el.sliderTrack.style.setProperty('--trim-start', `${minPercent}%`);
    el.sliderTrack.style.setProperty('--trim-end', `${maxPercent}%`);
    el.trimLength.textContent = `Süre ${formatTimeLimit(maxVal - minVal)}`;

    // Whichever thumb sits at the far end must stay on top, or it gets trapped under the other.
    el.rangeMin.style.zIndex = minVal >= videoDuration - MIN_TRIM_GAP_SECONDS * 2 ? 3 : 2;
    refreshDownloadLabel();
}

function setTrimBound(isStart, seconds) {
    trimSelectionTouched = true;
    const other = parseFloat((isStart ? el.rangeMax : el.rangeMin).value);
    const sec = isStart
        ? clampTime(seconds, 0, Math.max(0, other - MIN_TRIM_GAP_SECONDS))
        : clampTime(seconds, Math.min(videoDuration, other + MIN_TRIM_GAP_SECONDS), videoDuration);
    (isStart ? el.rangeMin : el.rangeMax).value = sec;
    (isStart ? el.startTimeInput : el.endTimeInput).value = formatTimeLimit(sec);
    updateSliderTrack();
}

function setupSliderEvents() {
    el.rangeMin.addEventListener('input', () => setTrimBound(true, parseFloat(el.rangeMin.value)));
    el.rangeMax.addEventListener('input', () => setTrimBound(false, parseFloat(el.rangeMax.value)));
    el.startTimeInput.addEventListener('change', () => setTrimBound(true, parseTimeInput(el.startTimeInput.value)));
    el.endTimeInput.addEventListener('change', () => {
        setTrimBound(false, parseTimeInput(el.endTimeInput.value) || videoDuration);
    });
    el.trimResetBtn.addEventListener('click', () => {
        if (videoDuration > 0) setupSliders();
    });
}

function setupSliderKeys() {
    const deltas = { ArrowLeft: -1, ArrowDown: -1, ArrowRight: 1, ArrowUp: 1 };
    for (const range of [el.rangeMin, el.rangeMax]) {
        range.addEventListener('keydown', (e) => {
            const direction = deltas[e.key];
            if (!direction) return;
            e.preventDefault();
            const step = e.shiftKey ? TRIM_KEY_BIG_STEP_SECONDS : TRIM_KEY_STEP_SECONDS;
            range.value = clampTime(parseFloat(range.value) + direction * step, 0, videoDuration);
            range.dispatchEvent(new Event('input'));
        });
    }
}

function setProgressBar(percentText) {
    const value = parseFloat(percentText);
    const known = Number.isFinite(value);
    el.progressBar.classList.toggle('indeterminate', !known);
    el.progressBar.style.width = known ? `${clampTime(value, 0, 100)}%` : '100%';
    if (known) {
        el.progressTrack.setAttribute('aria-valuenow', Math.round(clampTime(value, 0, 100)));
    } else {
        el.progressTrack.removeAttribute('aria-valuenow');
    }
}

function setProgressPercent(percentText) {
    const value = parseFloat(percentText);
    el.progressPercent.textContent = Number.isFinite(value) ? `${Math.floor(clampTime(value, 0, 100))}%` : "";
}

function parseSpeed(text) {
    const match = /^([\d.]+)\s*([KMGT]?)i?B\/s$/i.exec(String(text).trim());
    if (!match) return null;
    const scale = { '': 0, K: 1, M: 2, G: 3, T: 4 }[match[2].toUpperCase()];
    const value = parseFloat(match[1]) * 1024 ** scale;
    return Number.isFinite(value) ? value : null;
}

function spaceUnits(text) {
    return String(text).replace(/(\d)([KMGT]?B)\b/g, '$1 $2');
}

function advanceGraph(now) {
    const dt = Math.min(Math.max(now - graph.lastFrame, 0), 1000);
    graph.lastFrame = now;
    graph.value += (graph.target - graph.value) * (1 - Math.exp(-dt / GRAPH_SMOOTHING_MS));
    const last = graph.points[graph.points.length - 1];
    if (!last || now - last[0] >= GRAPH_STEP_MS) graph.points.push([now, graph.value]);
    while (graph.points.length > 2 && graph.points[1][0] < now - GRAPH_WINDOW_MS) graph.points.shift();

    const top = Math.max(graph.target, ...graph.points.map((point) => point[1]), 1) * 1.2;
    const pace = top > graph.peak ? GRAPH_SMOOTHING_MS / 4 : GRAPH_SMOOTHING_MS * 2;
    graph.peak += (top - graph.peak) * (1 - Math.exp(-dt / pace));
}

function drawGraph(now) {
    const canvas = el.speedGraph;
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    const ctx = canvas.getContext('2d');
    if (!width || !ctx) return;
    const ratio = window.devicePixelRatio || 1;
    if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
        canvas.width = Math.round(width * ratio);
        canvas.height = Math.round(height * ratio);
    }
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const elapsed = now - graph.startedAt;
    const span = clampTime(elapsed, GRAPH_MIN_SPAN_MS, GRAPH_WINDOW_MS);
    const origin = elapsed > GRAPH_WINDOW_MS ? now - GRAPH_WINDOW_MS : graph.startedAt;
    const points = [...graph.points, [now, graph.value]].map(([time, value]) => [
        ((time - origin) / span) * width,
        height - 1 - clampTime(value / graph.peak, 0, 1) * (height - 6),
    ]);
    if (points.length < 2) return;

    ctx.beginPath();
    ctx.moveTo(points[0][0], points[0][1]);
    for (let i = 1; i < points.length - 1; i++) {
        const [x, y] = points[i];
        ctx.quadraticCurveTo(x, y, (x + points[i + 1][0]) / 2, (y + points[i + 1][1]) / 2);
    }
    const [endX, endY] = points[points.length - 1];
    ctx.lineTo(endX, endY);

    ctx.strokeStyle = ctx.fillStyle = getComputedStyle(canvas).color;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = 'round';
    ctx.stroke();
    ctx.lineTo(endX, height);
    ctx.lineTo(points[0][0], height);
    ctx.closePath();
    ctx.globalAlpha = 0.12;
    ctx.fill();
    ctx.globalAlpha = 1;
}

function graphFrame(now) {
    advanceGraph(now);
    drawGraph(now);
    graph.frame = requestAnimationFrame(graphFrame);
}

function startGraph() {
    cancelAnimationFrame(graph.frame);
    const now = performance.now();
    Object.assign(graph, { points: [], target: 0, value: 0, peak: 1, startedAt: now, lastFrame: now });
    drawGraph(now);
    if (!reducedMotion.matches) graph.frame = requestAnimationFrame(graphFrame);
}

function stopGraph() {
    cancelAnimationFrame(graph.frame);
    graph.frame = 0;
}

function setGraphSpeed(bytesPerSecond) {
    graph.target = bytesPerSecond;
    if (!graph.frame) {
        const now = performance.now();
        advanceGraph(now);
        drawGraph(now);
    }
}

function setProgressThumb(url) {
    setThumb(el.progressThumb, url || "");
}

function setProgressTitle(title) {
    el.progressTitle.textContent = title;
    el.progressTitle.title = title;
    if (activeQueueRow && title) {
        const name = activeQueueRow.querySelector('.queue-name');
        name.textContent = title;
    }
}

function showStage(label, percentText = "") {
    setProgressPercent(percentText);
    el.downloadSpeed.textContent = label;
    el.progressInfo.textContent = "";
    setGraphSpeed(0);
    setProgressBar(percentText);
}

function setupProgressListener() {
    listen('download-item', (event) => {
        if (!isDownloading) return;
        const item = event.payload || {};
        itemAnnounced = true;
        if (item.title) setProgressTitle(item.title);
        if (item.thumbnail) setProgressThumb(item.thumbnail);
    });

    listen('download-progress', (event) => {
        if (!isDownloading) return;
        const data = event.payload;

        if (data.status === 'downloading') {
            itemAnnounced = true;
            if (data.title) setProgressTitle(data.title);
            setProgressPercent(data.percent);
            if (data.playlist_info) {
                el.playlistCounter.textContent = data.playlist_info;
                el.playlistCounter.classList.remove('hidden');
            } else {
                el.playlistCounter.classList.add('hidden');
            }
            const speed = parseSpeed(data.speed);
            el.downloadSpeed.textContent = speed === null ? spaceUnits(data.speed || "") : `${formatBytes(speed)}/sn`;
            if (speed !== null) setGraphSpeed(speed);
            el.progressInfo.textContent = [spaceUnits(data.size_info || ""), data.eta && `Kalan ${data.eta}`]
                .filter(Boolean)
                .join(" · ");
            setProgressBar(data.overall || data.percent);
        } else if (data.status === 'merging') {
            // SponsorBlock's lookup reports itself as a processing step before anything is downloaded.
            if (itemAnnounced) showStage("Dosya işleniyor");
        } else if (data.status === 'retrying') {
            el.playlistCounter.classList.add('hidden');
            showStage("Tarayıcı oturumuyla yeniden deneniyor");
        } else if (data.status === 'section') {
            showStage("Kesit indiriliyor");
        } else if (data.status === 'trimming') {
            showStage("Kesit hazırlanıyor", data.percent);
        } else if (data.status === 'cutting') {
            showStage("Sponsor bölümleri çıkarılıyor", data.percent);
        }
    });
}

function setFormDisabled(disabled) {
    for (const control of [
        el.urlInput, el.browseBtn, el.modeVideo, el.modeAudio, el.playlistToggle, el.sponsorToggle,
        el.startTimeInput, el.endTimeInput, el.rangeMin, el.rangeMax, el.trimResetBtn,
        el.pasteBtn, el.clearUrlBtn, el.downloadBtn,
    ]) {
        control.disabled = disabled;
    }
    el.composer.classList.toggle('busy', disabled);
    refreshFormatControls();
}

function selectedTrimRange() {
    if (!trimSelectionTouched || !trimAvailable()) return null;

    const start = parseFloat(el.rangeMin.value);
    const end = parseFloat(el.rangeMax.value);
    const hasCustomStart = start > TRIM_COMPARE_EPSILON;
    const hasCustomEnd = end < videoDuration - TRIM_COMPARE_EPSILON;
    if (!hasCustomStart && !hasCustomEnd) return null;

    return { start: formatTimeLimit(start), end: formatTimeLimit(end) };
}

function requestsFromForm() {
    const links = parseLinks(el.urlInput.value);
    if (links.length === 0) {
        setStatus("Bir bağlantı girin.", 'error');
        el.urlInput.focus();
        return null;
    }
    const batch = links.length > 1;
    const trim = batch ? null : selectedTrimRange();
    return links.map((url) => ({
        url,
        quality: selectedQuality(),
        audio_only: audioOnly(),
        download_playlist: el.playlistToggle.checked || (batch && PLAYLIST_ONLY_URL.test(url)),
        start_time: trim ? trim.start : null,
        end_time: trim ? trim.end : null,
        duration: trim ? videoDuration : null,
        save_copy: false,
        sponsorblock: el.sponsorToggle.checked,
    }));
}

function knownPreview(request) {
    const previewed = request.url === lastFetchedUrl && lastInfo && !request.download_playlist;
    return {
        title: previewed ? (lastInfo.title || "") : "",
        thumbnail: (previewed && lastInfo.thumbnail) || (request.download_playlist ? "" : youtubeThumbnail(request.url)),
    };
}

function resetProgress(index, total, request) {
    const known = knownPreview(request);
    itemAnnounced = false;
    el.cancelBtn.disabled = false;
    el.cancelBtn.textContent = "İptal";
    el.batchCounter.textContent = total > 1 ? `Bağlantı ${index + 1} / ${total}` : "";
    el.batchCounter.classList.toggle('hidden', total <= 1);
    setProgressTitle(known.title);
    setProgressThumb(known.thumbnail);
    el.progressPercent.textContent = "0%";
    el.playlistCounter.textContent = "";
    el.playlistCounter.classList.add('hidden');
    el.downloadSpeed.textContent = "Bağlanılıyor";
    el.progressInfo.textContent = "";
    setProgressBar("0%");
}

function buildQueue(requests) {
    activeQueueRow = null;
    const batch = requests.length > 1;
    el.queueList.classList.toggle('hidden', !batch);
    if (!batch) {
        el.queueList.replaceChildren();
        return [];
    }
    const rows = requests.map((request) => {
        const row = document.createElement('li');
        const name = document.createElement('span');
        name.className = 'queue-name';
        name.textContent = shortUrl(request.url);
        name.title = request.url;
        const state = document.createElement('span');
        state.className = 'queue-state';
        state.textContent = "Sırada";
        row.append(name, state);
        return row;
    });
    el.queueList.replaceChildren(...rows);
    return rows;
}

function markQueueRow(row, state, label, detail) {
    if (!row) return;
    row.className = state;
    row.querySelector('.queue-state').textContent = label;
    if (detail) row.title = detail;
    if (state === 'active') {
        activeQueueRow = row;
        el.queueList.scrollTop = Math.max(0, row.offsetTop - el.queueList.clientHeight / 2);
    } else if (activeQueueRow === row) {
        activeQueueRow = null;
    }
}

function showResult(request, data, batch) {
    const existed = Boolean(data.already_existed);
    el.resultCard.classList.toggle('existed', existed);
    el.resultTitle.textContent = existed ? "Zaten indirilmiş" : "İndirildi";
    el.filePath.textContent = (batch ? "Son dosya: " : "") + data.filename;
    el.filePath.title = data.full_path;
    el.resultMeta.textContent = [
        data.is_dir ? "Oynatma listesi klasörü" : "",
        formatBytes(data.size),
        data.duration > 0 ? formatClock(data.duration) : "",
        data.removed,
    ].filter(Boolean).join(" · ");
    setThumb(el.resultThumb, data.thumbnail || "");
    currentSavedPath = data.full_path;
    el.openFileBtn.classList.toggle('hidden', Boolean(data.is_dir));
    lastRequest = request;
    el.redownloadBtn.classList.toggle('hidden', !existed || Boolean(data.is_dir));
    el.resultCard.classList.remove('hidden');
    refreshAmbient();
}

function hideResult() {
    el.resultCard.classList.add('hidden');
    refreshAmbient();
}

function reportSingleOutcome(request, data) {
    if (data.warning) {
        setStatus(data.warning, 'warning');
    } else if (data.already_existed) {
        setStatus(
            request.audio_only
                ? "Bu ses dosyası klasörde zaten var; yeniden indirilmedi."
                : "Bu video klasörde zaten var; yeniden indirilmedi.",
            'warning'
        );
    } else {
        setStatus("");
    }
}

function reportBatchOutcome(done, failed, cancelled) {
    const existed = done.filter(({ data }) => data.already_existed).length;
    const parts = [`İndirilen: ${done.length - existed}`];
    if (existed) parts.push(`Zaten indirilmiş: ${existed}`);
    if (failed.length) parts.push(`Başarısız: ${failed.length} (alanda bırakıldı)`);
    if (cancelled) parts.push("Kalanlar iptal edildi");

    const note = failed.length
        ? failed[failed.length - 1].message
        : done.map(({ data }) => data.warning).filter(Boolean).pop();
    const text = parts.join(" · ") + (note ? ` — ${note}` : "");

    if (failed.length || cancelled) {
        setStatus(text, done.length ? 'warning' : 'error');
    } else {
        setStatus(text, note ? 'warning' : 'success');
    }
}

async function startDownload(override) {
    if (isDownloading) return;

    const requests = override ? [override] : requestsFromForm();
    if (!requests) return;
    const batch = requests.length > 1;

    clearTimeout(inputDebounceTimer);
    infoRequestId++;
    pendingInfoUrl = "";

    isDownloading = true;
    cancelRequested = false;
    setStatus("");
    hideSignIn();
    hideResult();
    hideClipChip();
    for (const request of requests) usedLinks.add(request.url);
    setFormDisabled(true);
    const rows = buildQueue(requests);
    resetProgress(0, requests.length, requests[0]);
    el.progressContainer.classList.remove('hidden');
    startGraph();
    el.progressContainer.scrollIntoView({ block: 'nearest', behavior: 'smooth' });

    const done = [];
    const failed = [];
    let index = 0;
    try {
        while (index < requests.length && !cancelRequested) {
            const request = requests[index];
            resetProgress(index, requests.length, request);
            markQueueRow(rows[index], 'active', "İndiriliyor");
            try {
                const data = await invoke('start_download', { request });
                if (data.status === "cancelled") break;
                done.push({ request, data });
                markQueueRow(rows[index], data.already_existed ? 'exists' : 'done',
                    data.already_existed ? "Zaten var" : "Tamamlandı");
            } catch (error) {
                const message = errorText(error, "İndirme başarısız");
                failed.push({ request, message });
                markQueueRow(rows[index], 'failed', "Başarısız", message);
            }
            index++;
        }
    } finally {
        isDownloading = false;
        activeQueueRow = null;
        stopGraph();
        el.progressContainer.classList.add('hidden');
        setProgressThumb("");
        setFormDisabled(false);
    }
    const cancelled = index < requests.length;

    if (done.length) {
        const last = done[done.length - 1];
        showResult(last.request, last.data, batch);

        const saved = new Set(done.map(({ request }) => request.url));
        const links = parseLinks(el.urlInput.value);
        const remaining = links.filter((link) => !saved.has(link));
        if (remaining.length !== links.length) {
            setUrlValue(remaining.join("\n"));
            fetchInfoForUrl();
        }
        loadHistory();
    }

    if (batch) {
        reportBatchOutcome(done, failed, cancelled);
    } else if (cancelled) {
        setStatus("İndirme iptal edildi.", 'info');
    } else if (failed.length) {
        setStatus(failed[0].message, 'error');
        offerSignIn(failed[0].request, failed[0].message);
    } else {
        reportSingleOutcome(done[0].request, done[0].data);
    }

    if (el.previewCard.classList.contains('loading')) {
        fetchInfoForUrl();
    }
    if (failed.length) {
        pollUpdateStatus();
    }

    const outcome = el.resultCard.classList.contains('hidden') ? el.status : el.resultCard;
    outcome.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

function redownloadAsCopy() {
    if (!lastRequest) return;
    startDownload({ ...lastRequest, save_copy: true });
}

function relativeTime(savedAt) {
    if (!savedAt) return "";
    const minutes = Math.floor((Date.now() / 1000 - savedAt) / 60);
    if (minutes < 1) return "az önce";
    if (minutes < 60) return `${minutes} dk önce`;
    if (minutes < 60 * 24) return `${Math.floor(minutes / 60)} sa önce`;
    if (minutes < 60 * 24 * 7) return `${Math.floor(minutes / (60 * 24))} gün önce`;
    return new Date(savedAt * 1000).toLocaleDateString('tr-TR');
}

function historyButton(symbol, label, onClick) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'icon-btn';
    button.append(icon(symbol));
    button.title = label;
    button.setAttribute('aria-label', label);
    button.addEventListener('click', onClick);
    return button;
}

function isAudioItem(item) {
    return Boolean(item.audio) || /\.(mp3|m4a|opus|flac|wav|ogg)$/i.test(item.name);
}

function historyRow(item) {
    const row = document.createElement('li');
    row.classList.toggle('missing', !item.exists);

    const thumb = document.createElement('div');
    thumb.className = 'thumb history-thumb';
    thumb.append(icon(item.is_dir ? 'list' : (isAudioItem(item) ? 'music' : 'film')));
    if (item.thumbnail) {
        const img = document.createElement('img');
        img.alt = "";
        img.loading = 'lazy';
        img.referrerPolicy = 'no-referrer';
        img.addEventListener('error', () => img.remove());
        img.src = item.thumbnail;
        thumb.append(img);
    }

    const text = document.createElement('div');
    text.className = 'history-text';
    const name = document.createElement('span');
    name.className = 'history-name';
    name.textContent = item.title || item.name;
    name.title = item.path;
    const meta = document.createElement('span');
    meta.className = 'history-meta';
    meta.textContent = [
        item.exists ? relativeTime(item.saved_at) : "Dosya bulunamadı",
        item.is_dir ? "Oynatma listesi" : (isAudioItem(item) ? "MP3" : ""),
        formatBytes(item.size),
        item.duration > 0 ? formatClock(item.duration) : "",
    ].filter(Boolean).join(" · ");
    text.append(name, meta);

    const actions = document.createElement('div');
    actions.className = 'history-actions';
    if (item.exists) {
        if (!item.is_dir) {
            actions.append(historyButton('play', "Oynat", () => openSavedPath('open_file', item.path)));
        }
        actions.append(historyButton('folder', "Klasörde göster", () => openSavedPath('open_folder', item.path)));
    }
    if (item.url) {
        actions.append(historyButton('repeat', "Bağlantıyı yeniden kullan", () => {
            if (isDownloading) return;
            addLinks([item.url]);
            el.urlInput.scrollIntoView({ block: 'center', behavior: 'smooth' });
        }));
    }
    actions.append(historyButton('close', "Listeden kaldır", async () => {
        try {
            renderHistory(await invoke('remove_history_entry', { filePath: item.path }));
        } catch (e) {
            console.error("Failed to remove history entry", e);
        }
    }));

    row.append(thumb, text, actions);
    return row;
}

function drawHistory() {
    const query = el.historySearch.value.trim().toLocaleLowerCase('tr-TR');
    const visible = query
        ? historyItems.filter((item) => `${item.title} ${item.name}`.toLocaleLowerCase('tr-TR').includes(query))
        : historyItems;
    el.historyList.replaceChildren(...visible.map(historyRow));
    el.historyEmpty.classList.toggle('hidden', visible.length > 0);
}

function renderHistory(items) {
    if (currentSavedPath && !items.some((item) => item.path === currentSavedPath)) {
        currentSavedPath = "";
        hideResult();
    }
    historyItems = items;
    el.historySection.classList.toggle('hidden', items.length === 0);
    const totalBytes = items.reduce((sum, item) => sum + (item.exists ? Number(item.size) || 0 : 0), 0);
    el.historyCount.textContent = [`${items.length} kayıt`, formatBytes(totalBytes)].filter(Boolean).join(" · ");
    const searchable = items.length >= HISTORY_SEARCH_MIN_ITEMS;
    el.historySearch.classList.toggle('hidden', !searchable);
    if (!searchable) el.historySearch.value = "";
    drawHistory();
}

async function loadHistory() {
    try {
        renderHistory(await invoke('get_history'));
    } catch (e) {
        console.error("Failed to load history", e);
    }
}

async function openSavedPath(command, filePath) {
    if (!filePath) return;
    try {
        await invoke(command, { filePath });
    } catch (e) {
        console.error("Failed to open saved download", e);
        setStatus("Dosya açılamadı; taşınmış veya silinmiş olabilir.", 'error');
        loadHistory();
    }
}

async function clearHistory() {
    try {
        await invoke('clear_history');
        renderHistory([]);
    } catch (e) {
        console.error("Failed to clear history", e);
    }
}

function showFolder(path) {
    el.dirInput.textContent = path;
    el.dirInput.title = path;
}

async function browseFolder() {
    try {
        const path = await invoke('select_folder');
        if (path) {
            showFolder(path);
        }
    } catch (e) {
        console.error("Failed to open folder picker", e);
    }
}

async function cancelDownload() {
    cancelRequested = true;
    el.cancelBtn.disabled = true;
    el.cancelBtn.textContent = "İptal ediliyor";
    setGraphSpeed(0);
    try {
        await invoke('cancel_download');
    } catch (e) {
        console.error("Cancel failed", e);
        cancelRequested = false;
        el.cancelBtn.disabled = false;
        el.cancelBtn.textContent = "İptal";
    }
}

function enterUrlText(text) {
    setUrlValue(text);
    el.urlInput.focus();
    el.urlInput.dispatchEvent(new Event('input'));
    clearTimeout(inputDebounceTimer);
    fetchInfoForUrl();
}

function addLinks(links) {
    const current = parseLinks(el.urlInput.value);
    enterUrlText(current.concat(links.filter((link) => !current.includes(link))).join("\n"));
}

function modalOpen() {
    return el.shortcutsDialog.open || el.patchNotesDialog.open || el.cookiesDialog.open;
}

let signInRequest = null;

function hideSignIn() {
    signInRequest = null;
    el.signInRow.classList.add('hidden');
}

function offerSignIn(request, message) {
    if (!IS_ANDROID || !/giriş yap/i.test(message)) return;
    signInRequest = request;
    el.signInRow.classList.remove('hidden');
}

async function signInAndRetry() {
    const request = signInRequest;
    if (!request || isDownloading) return;
    hideSignIn();
    try {
        if (await invoke('sign_in', { url: request.url })) {
            startDownload(request);
        } else {
            setStatus("Giriş tamamlanmadı; çerez kaydedilmedi.", 'info');
            offerSignIn(request, "giriş yap");
        }
    } catch (e) {
        setStatus(errorText(e, "Giriş ekranı açılamadı."), 'error');
    }
}

async function refreshCookiesState(note) {
    let saved = false;
    try {
        saved = await invoke('cookies_saved');
    } catch (e) {
        console.error("Failed to read cookie state", e);
    }
    el.cookiesState.textContent = note || (saved
        ? "Kayıtlı çerezler var; oturum isteyen videolarda kullanılır."
        : "Kayıtlı çerez yok. Oturum isteyen bir videoda \"Giriş yap\" düğmesi çıkar; bilgisayardan dışa aktarılmış bir cookies.txt da içe aktarabilirsiniz.");
    el.clearCookiesBtn.disabled = !saved;
}

async function openCookies() {
    await refreshCookiesState();
    el.cookiesDialog.showModal();
}

async function runCookieAction(command, doneNote) {
    try {
        const changed = await invoke(command);
        await refreshCookiesState(changed || command === 'clear_cookies' ? doneNote : "");
    } catch (e) {
        await refreshCookiesState(errorText(e, "İşlem başarısız."));
    }
}

function pasteIntoUrlField(e) {
    if (isDownloading || modalOpen()) return;
    if (e.target instanceof Element && e.target.closest('input, textarea, select')) return;
    const text = (e.clipboardData?.getData('text') || "").trim();
    if (!text) return;
    e.preventDefault();
    enterUrlText(text);
}

function separatePastedLink(e) {
    const text = (e.clipboardData?.getData('text') || "").trim();
    const { selectionStart: caret, selectionEnd, value } = el.urlInput;
    if (!/^https?:\/\//i.test(text) || caret !== selectionEnd) return;

    const joinsBefore = caret > 0 && value[caret - 1] !== '\n';
    const joinsAfter = caret < value.length && value[caret] !== '\n';
    if (joinsBefore === joinsAfter) return;

    e.preventDefault();
    el.urlInput.setRangeText((joinsBefore ? '\n' : '') + text + (joinsAfter ? '\n' : ''), caret, caret, 'end');
    el.urlInput.dispatchEvent(new Event('input'));
}

async function readClipboardLinks() {
    try {
        const links = await invoke('clipboard_links');
        return Array.isArray(links) ? links : [];
    } catch (e) {
        console.error("Failed to read the clipboard", e);
        return [];
    }
}

function hideClipChip() {
    clipLinks = [];
    el.clipChip.classList.add('hidden');
}

async function suggestClipboardLinks() {
    if (isDownloading || modalOpen()) return;
    const found = await readClipboardLinks();
    if (isDownloading) return;
    const current = parseLinks(el.urlInput.value);
    const links = found.filter((link) => !current.includes(link) && !usedLinks.has(link));
    if (links.length === 0 || links.join("\n") === clipDismissedKey) {
        hideClipChip();
        return;
    }
    clipLinks = links;
    el.clipText.textContent = links.length > 1 ? `${links.length} bağlantı` : shortUrl(links[0]);
    el.clipText.title = links.join("\n");
    el.clipChip.classList.remove('hidden');
}

function acceptClipboardLinks() {
    const links = clipLinks;
    hideClipChip();
    if (links.length && !isDownloading) addLinks(links);
}

async function takeSharedLinks() {
    if (isDownloading) return;
    try {
        const links = await invoke('shared_links');
        if (Array.isArray(links) && links.length && !isDownloading) addLinks(links);
    } catch (e) {
        console.error("Failed to read shared links", e);
    }
}

function onAppShown() {
    takeSharedLinks().then(suggestClipboardLinks);
}

async function pasteFromClipboard() {
    if (isDownloading) return;
    const links = await readClipboardLinks();
    if (links.length === 0) {
        setStatus("Panoda bağlantı yok.", 'info');
        el.urlInput.focus();
        return;
    }
    hideClipChip();
    addLinks(links);
}

function dragCarriesText(e) {
    const types = [...(e.dataTransfer?.types || [])];
    return types.includes('text/uri-list') || types.includes('text/plain');
}

function setupDragAndDrop() {
    const hide = () => {
        dragDepth = 0;
        el.dropOverlay.classList.add('hidden');
    };
    document.addEventListener('dragstart', () => {
        internalDrag = true;
    });
    document.addEventListener('dragend', () => {
        internalDrag = false;
    });
    document.addEventListener('dragenter', (e) => {
        if (internalDrag) return;
        e.preventDefault();
        dragDepth++;
        if (dragCarriesText(e) && !isDownloading && !modalOpen()) {
            el.dropOverlay.classList.remove('hidden');
        }
    });
    document.addEventListener('dragover', (e) => {
        if (internalDrag) return;
        // Without this the webview navigates to whatever is dropped on it.
        e.preventDefault();
        e.dataTransfer.dropEffect = dragCarriesText(e) && !isDownloading ? 'copy' : 'none';
    });
    document.addEventListener('dragleave', () => {
        if (internalDrag) return;
        dragDepth = Math.max(0, dragDepth - 1);
        if (dragDepth === 0) hide();
    });
    document.addEventListener('drop', (e) => {
        if (internalDrag) return;
        e.preventDefault();
        hide();
        if (isDownloading || modalOpen()) return;
        const raw = e.dataTransfer.getData('text/uri-list') || e.dataTransfer.getData('text/plain') || "";
        const text = raw.split(/\r?\n/).filter((line) => !line.startsWith('#')).join("\n");
        const links = parseLinks(text).filter((link) => HTTP_URL.test(link));
        if (links.length === 0) {
            setStatus("Bırakılan içerikte bağlantı yok.", 'info');
            return;
        }
        addLinks(links);
    });
}

async function pollUpdateStatus() {
    if (updatePollActive) return;
    updatePollActive = true;
    try {
        let data = await invoke('get_update_status');
        while (data.status === 'checking') {
            await new Promise((resolve) => setTimeout(resolve, 1000));
            data = await invoke('get_update_status');
        }
        if (data.status === 'available') {
            el.updateText.textContent = `yt-dlp güncellemesi var (${data.current} → ${data.latest})`;
            el.updateBanner.classList.remove('hidden');
        }
    } catch (e) {
        console.error("Update status failed", e);
    } finally {
        updatePollActive = false;
    }
}

async function updateYtdlp() {
    el.updateBtn.disabled = true;
    el.updateBtn.textContent = "Güncelleniyor";
    try {
        const updated = await invoke('update_ytdlp');
        el.updateText.textContent = `yt-dlp güncellendi (${updated.current}).`;
        el.updateBtn.classList.add('hidden');
        setTimeout(() => el.updateBanner.classList.add('hidden'), 2500);
    } catch (e) {
        el.updateText.textContent = errorText(e, "Güncelleme başarısız");
        el.updateBtn.disabled = false;
        el.updateBtn.textContent = "Tekrar dene";
    }
}

let patchNotesShown = false;

function appendInlineMarkdown(parent, text) {
    const tokenRe = /\*\*(.+?)\*\*|`([^`]+)`/g;
    let last = 0;
    for (const match of text.matchAll(tokenRe)) {
        if (match.index > last) {
            parent.appendChild(document.createTextNode(text.slice(last, match.index)));
        }
        if (match[1] !== undefined) {
            const strong = document.createElement('strong');
            strong.textContent = match[1];
            parent.appendChild(strong);
        } else {
            const code = document.createElement('code');
            code.textContent = match[2];
            parent.appendChild(code);
        }
        last = match.index + match[0].length;
    }
    if (last < text.length) {
        parent.appendChild(document.createTextNode(text.slice(last)));
    }
}

function showPatchNotes(version, markdown) {
    if (patchNotesShown || !markdown || !markdown.trim()) return;
    patchNotesShown = true;

    el.patchNotesTitle.textContent = version ? `${version} — Yenilikler` : "Yenilikler";
    el.patchNotesBody.replaceChildren();

    let list = null;
    for (const rawLine of markdown.replace(/\r\n/g, '\n').split('\n')) {
        const line = rawLine.trim();
        if (!line) {
            list = null;
            continue;
        }
        const heading = line.match(/^#{1,6}\s+(.*)$/);
        if (heading) {
            list = null;
            const h3 = document.createElement('h3');
            appendInlineMarkdown(h3, heading[1]);
            el.patchNotesBody.appendChild(h3);
            continue;
        }
        const bullet = line.match(/^[-*•]\s+(.*)$/);
        if (bullet) {
            if (!list) {
                list = document.createElement('ul');
                el.patchNotesBody.appendChild(list);
            }
            const li = document.createElement('li');
            appendInlineMarkdown(li, bullet[1]);
            list.appendChild(li);
            continue;
        }
        list = null;
        const p = document.createElement('p');
        appendInlineMarkdown(p, line);
        el.patchNotesBody.appendChild(p);
    }

    if (!modalOpen()) {
        el.patchNotesDialog.showModal();
    }
}

async function pollAppUpdateStatus() {
    try {
        const data = await invoke('get_app_update_status');
        if (data.current) {
            el.appVersion.textContent = data.current;
        }
        if (data.patch_notes) {
            showPatchNotes(data.current, data.patch_notes);
        }
        if (data.status === 'checking') {
            setTimeout(pollAppUpdateStatus, 1000);
            return;
        }
        if (data.status === 'available') {
            el.appUpdateText.textContent = `Yeni sürüm var (${data.current} → ${data.latest})`;
            el.appUpdateBanner.classList.remove('hidden');
        }
    } catch (e) {
        console.error("App update status failed", e);
    }
}

async function updateApp() {
    el.appUpdateBtn.disabled = true;
    el.appUpdateBtn.textContent = "İndiriliyor";

    let unlisten = null;
    try {
        unlisten = await listen('app-update-progress', (event) => {
            const pct = event.payload ?? 0;
            el.appUpdateBtn.textContent = pct < 100 ? `İndiriliyor %${pct}` : "Kuruluyor";
        });
        await invoke('update_app');
        el.appUpdateText.textContent = "Güncellendi, yeniden başlatılıyor.";
    } catch (e) {
        el.appUpdateText.textContent = errorText(e, "Uygulama güncellemesi başarısız");
        el.appUpdateBtn.disabled = false;
        el.appUpdateBtn.textContent = "Tekrar dene";
    } finally {
        if (unlisten) unlisten();
    }
}

function refreshThemeToggle() {
    const light = document.documentElement.dataset.theme === 'light';
    const label = light ? "Koyu temaya geç" : "Açık temaya geç";
    $('themeIcon').setAttribute('href', light ? '#i-light' : '#i-dark');
    el.themeToggle.title = label;
    el.themeToggle.setAttribute('aria-label', label);
}

function applyThemeWithReveal(next) {
    const apply = () => {
        window.appTheme.set(next);
        refreshThemeToggle();
    };
    if (!document.startViewTransition || reducedMotion.matches) {
        apply();
        return;
    }
    const rect = el.themeToggle.getBoundingClientRect();
    const x = rect.left + rect.width / 2;
    const y = rect.top + rect.height / 2;
    const radius = Math.hypot(Math.max(x, window.innerWidth - x), Math.max(y, window.innerHeight - y));
    document.startViewTransition(apply).ready
        .then(() => {
            document.documentElement.animate(
                { clipPath: [`circle(0px at ${x}px ${y}px)`, `circle(${radius}px at ${x}px ${y}px)`] },
                { duration: 450, easing: 'cubic-bezier(0.3, 0, 0.2, 1)', pseudoElement: '::view-transition-new(root)' },
            );
        })
        .catch(() => {});
}

async function toggleTheme() {
    const next = document.documentElement.dataset.theme === 'light' ? 'dark' : 'light';
    applyThemeWithReveal(next);
    try {
        await invoke('set_theme', { theme: next });
    } catch (e) {
        console.error("Failed to save theme", e);
    }
}

async function loadConfig() {
    try {
        const data = await invoke('get_config');
        if (data.default_dir) {
            showFolder(data.default_dir);
        }
        if ([...el.qualitySelect.options].some((option) => option.value === data.quality)) {
            el.qualitySelect.value = data.quality;
        }
        if (data.theme && data.theme !== window.appTheme.mode) {
            window.appTheme.set(data.theme);
            refreshThemeToggle();
        }
        if (data.audio_only && !isDownloading) {
            el.modeAudio.checked = true;
        }
        el.sponsorToggle.checked = Boolean(data.sponsorblock);
        refreshFormatControls();
    } catch (e) {
        console.error("Failed to fetch config", e);
    }
}

function rememberHistoryOpen() {
    try {
        localStorage.setItem('historyOpen', el.historySection.open ? '1' : '0');
    } catch {}
}

function restoreHistoryOpen() {
    let stored = null;
    try {
        stored = localStorage.getItem('historyOpen');
    } catch {}
    el.historySection.open = stored !== '0';
}

function setupShortcuts() {
    document.addEventListener('keydown', (e) => {
        if (modalOpen()) {
            if (e.key === 'Escape') {
                e.preventDefault();
                if (el.shortcutsDialog.open) el.shortcutsDialog.close();
                if (el.patchNotesDialog.open) el.patchNotesDialog.close();
            }
            return;
        }
        if (e.key === 'Escape' && isDownloading && !el.cancelBtn.disabled) {
            cancelDownload();
            return;
        }
        if (!e.ctrlKey || e.altKey || e.shiftKey) return;
        const key = e.key.toLowerCase();
        if (key === 'l') {
            e.preventDefault();
            el.urlInput.focus();
            el.urlInput.select();
        } else if (key === 'm' && !isDownloading) {
            e.preventDefault();
            (audioOnly() ? el.modeVideo : el.modeAudio).checked = true;
            el.modeAudio.dispatchEvent(new Event('change', { bubbles: true }));
        }
    });
}

function init() {
    el.urlInput.addEventListener('paste', (e) => {
        separatePastedLink(e);
        clearTimeout(inputDebounceTimer);
        setTimeout(fetchInfoForUrl, 0);
    });
    el.urlInput.addEventListener('input', () => {
        clearTimeout(inputDebounceTimer);
        autosizeUrlInput();
        refreshUrlControls();
        if (el.status.classList.contains('error') || el.status.classList.contains('info')) {
            setStatus("");
        }
        const links = parseLinks(el.urlInput.value);
        if (clipLinks.length && clipLinks.every((link) => links.includes(link))) {
            hideClipChip();
        }
        if (links.length === 0) {
            resetVideoInfo();
            return;
        }
        const playlistOnly = links.length === 1 && PLAYLIST_ONLY_URL.test(links[0]);
        if (playlistOnly && !el.playlistToggle.checked) {
            el.playlistToggle.checked = true;
            playlistAutoEnabled = true;
            refreshTrimVisibility();
        } else if (!playlistOnly && playlistAutoEnabled) {
            el.playlistToggle.checked = false;
            playlistAutoEnabled = false;
            refreshTrimVisibility();
        }
        if (links.length > 1) {
            showBatchPreview(links.length);
        } else if (/^https?:\/\/\S+\.\S+/i.test(links[0])) {
            inputDebounceTimer = setTimeout(fetchInfoForUrl, 350);
        }
    });
    el.urlInput.addEventListener('blur', () => {
        clearTimeout(inputDebounceTimer);
        fetchInfoForUrl();
    });
    el.urlInput.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' && !e.isComposing && !e.shiftKey) {
            e.preventDefault();
            startDownload();
        }
    });
    el.clearUrlBtn.addEventListener('click', () => {
        setUrlValue("");
        el.urlInput.dispatchEvent(new Event('input'));
        el.urlInput.focus();
    });
    el.pasteBtn.addEventListener('click', pasteFromClipboard);
    el.clipAddBtn.addEventListener('click', acceptClipboardLinks);
    el.clipDismissBtn.addEventListener('click', () => {
        clipDismissedKey = clipLinks.join("\n");
        hideClipChip();
    });

    for (const img of [el.previewThumb, el.progressThumb, el.resultThumb]) {
        img.addEventListener('error', () => {
            failedThumbs.add(img.getAttribute('src'));
            setThumb(img, "");
        });
    }

    el.playlistToggle.addEventListener('change', () => {
        playlistAutoEnabled = false;
        refreshTrimVisibility();
        showFetchedInfo();
        clearTimeout(inputDebounceTimer);
        fetchInfoForUrl();
    });
    const onModeChange = async () => {
        refreshFormatControls();
        try {
            await invoke('set_audio_only', { audioOnly: audioOnly() });
        } catch (e) {
            console.error("Failed to save audio preference", e);
        }
    };
    el.modeVideo.addEventListener('change', onModeChange);
    el.modeAudio.addEventListener('change', onModeChange);
    el.qualitySelect.addEventListener('change', async () => {
        try {
            await invoke('set_quality', { quality: selectedQuality() });
        } catch (e) {
            console.error("Failed to save quality", e);
        }
    });
    el.sponsorToggle.addEventListener('change', async () => {
        try {
            await invoke('set_sponsorblock', { sponsorblock: el.sponsorToggle.checked });
        } catch (e) {
            console.error("Failed to save SponsorBlock preference", e);
        }
    });

    el.browseBtn.addEventListener('click', browseFolder);
    el.downloadBtn.addEventListener('click', () => startDownload());
    el.redownloadBtn.addEventListener('click', redownloadAsCopy);
    el.clearHistoryBtn.addEventListener('click', clearHistory);
    el.historySearch.addEventListener('input', drawHistory);
    el.historySection.addEventListener('toggle', () => {
        rememberHistoryOpen();
        if (el.historySection.open) loadHistory();
    });
    el.cancelBtn.addEventListener('click', cancelDownload);
    el.openFileBtn.addEventListener('click', () => openSavedPath('open_file', currentSavedPath));
    el.openFolderBtn.addEventListener('click', () => openSavedPath('open_folder', currentSavedPath));
    document.addEventListener('paste', pasteIntoUrlField);
    el.updateBtn.addEventListener('click', updateYtdlp);
    el.appUpdateBtn.addEventListener('click', updateApp);
    el.themeToggle.addEventListener('click', toggleTheme);
    window.matchMedia('(prefers-color-scheme: light)').addEventListener('change', refreshThemeToggle);
    el.shortcutsBtn.addEventListener('click', () => {
        if (!modalOpen()) el.shortcutsDialog.showModal();
    });
    el.shortcutsDialog.addEventListener('click', (e) => {
        if (e.target === el.shortcutsDialog) el.shortcutsDialog.close();
    });
    el.patchNotesDialog.addEventListener('click', (e) => {
        if (e.target === el.patchNotesDialog) el.patchNotesDialog.close();
    });
    el.signInBtn.addEventListener('click', signInAndRetry);
    if (IS_ANDROID) el.cookiesBtn.classList.remove('hidden');
    el.cookiesBtn.addEventListener('click', openCookies);
    el.cookiesDialog.addEventListener('click', (e) => {
        if (e.target === el.cookiesDialog) el.cookiesDialog.close();
    });
    el.importCookiesBtn.addEventListener('click', () => runCookieAction('import_cookies', "Çerezler içe aktarıldı."));
    el.clearCookiesBtn.addEventListener('click', () => runCookieAction('clear_cookies', "Çerezler silindi."));
    refreshThemeToggle();

    window.addEventListener('focus', onAppShown);
    document.addEventListener('visibilitychange', () => {
        el.ambient.classList.toggle('paused', document.hidden);
        if (!document.hidden) onAppShown();
    });

    restoreHistoryOpen();
    setupSliderEvents();
    setupSliderKeys();
    setupProgressListener();
    setupDragAndDrop();
    setupShortcuts();
    refreshFormatControls();
    pollAppUpdateStatus();
    pollUpdateStatus();
    loadConfig().then(onAppShown);
    loadHistory();
    el.urlInput.focus();
}

init();
