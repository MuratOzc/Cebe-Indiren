const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let currentSavedPath = "";
let videoDuration = 0;
let trimSelectionTouched = false;
let infoRequestId = 0;
let lastFetchedUrl = "";
let isDownloading = false;
const TRIM_COMPARE_EPSILON = 0.001;
const MIN_TRIM_GAP_SECONDS = 0.01;

document.addEventListener("DOMContentLoaded", () => {
    const urlInput = document.getElementById('urlInput');
    const playlistToggle = document.getElementById('playlistToggle');
    const trimmingSection = document.getElementById('trimmingSection');

    async function fetchInfoForUrl() {
        if (urlInput.disabled) return;
        const url = urlInput.value.trim();
        if (!url) {
            trimmingSection.style.display = 'none';
            document.getElementById('videoTitleLabel').innerText = "";
            videoDuration = 0;
            lastFetchedUrl = "";
            return;
        }

        if (url === lastFetchedUrl) return;

        const titleLabel = document.getElementById('videoTitleLabel');
        const durationLabel = document.getElementById('videoDurationLabel');
        const reqId = ++infoRequestId;

        titleLabel.innerText = "Bilgi getiriliyor...";

        try {
            const data = await invoke('get_video_info', { url });
            if (reqId !== infoRequestId) return;

            if (data.status === 'success') {
                lastFetchedUrl = url;
                videoDuration = Number(data.duration) || 0;
                titleLabel.innerText = data.title || "Bilinmeyen Başlık";
                durationLabel.innerText = formatTimeLimit(videoDuration);

                if (!playlistToggle.checked && videoDuration > 0) {
                    trimmingSection.style.display = 'flex';
                    setupSliders();
                } else {
                    trimmingSection.style.display = 'none';
                }
            } else {
                titleLabel.innerText = "Video bilgisi alınamadı.";
                trimmingSection.style.display = 'none';
            }
        } catch (e) {
            if (reqId === infoRequestId) {
                titleLabel.innerText = "Hata oluştu.";
                trimmingSection.style.display = 'none';
            }
        }
    }

    let inputDebounceTimer = null;

    urlInput.addEventListener('paste', () => {
        clearTimeout(inputDebounceTimer);
        setTimeout(fetchInfoForUrl, 0);
    });
    urlInput.addEventListener('input', () => {
        clearTimeout(inputDebounceTimer);
        const val = urlInput.value.trim();
        if (!val) {
            fetchInfoForUrl();
            return;
        }
        if (val.startsWith('http://') || val.startsWith('https://')) {
            inputDebounceTimer = setTimeout(fetchInfoForUrl, 350);
        }
    });
    urlInput.addEventListener('blur', () => {
        clearTimeout(inputDebounceTimer);
        fetchInfoForUrl();
    });
    urlInput.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
            clearTimeout(inputDebounceTimer);
            infoRequestId++;
            startDownload();
        }
    });

    playlistToggle.addEventListener('change', () => {
        if (playlistToggle.checked) {
            trimmingSection.style.display = 'none';
        } else if (urlInput.value.trim() && videoDuration > 0) {
            trimmingSection.style.display = 'flex';
        }
    });

    setupSliderEvents();
    setupProgressListener();
});

function setupProgressListener() {
    const progressBar = document.getElementById('progressBar');
    const progressPercent = document.getElementById('progressPercent');
    const playlistCounter = document.getElementById('playlistCounter');
    const downloadSpeed = document.getElementById('downloadSpeed');
    const progressInfo = document.getElementById('progressInfo');

    listen('download-progress', (event) => {
        if (!isDownloading) return;
        const data = event.payload;
        if (data.status === 'downloading') {
            progressPercent.innerText = data.percent;
            if (data.playlist_info) {
                playlistCounter.innerText = data.playlist_info;
                playlistCounter.style.display = 'inline-block';
            } else {
                playlistCounter.style.display = 'none';
            }
            downloadSpeed.innerText = data.speed;
            progressInfo.innerText = data.size_info || "";
            progressBar.style.width = data.percent;
        } else if (data.status === 'merging') {
            downloadSpeed.innerText = "Birleştiriliyor...";
            progressInfo.innerText = "Dosya birleştiriliyor (FFmpeg)...";
            progressBar.style.width = "100%";
        } else if (data.status === 'trimming') {
            downloadSpeed.innerText = "Kesiliyor...";
            progressInfo.innerText = "Video kesiliyor...";
            progressBar.style.width = "100%";
        }
    });
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

function parseTimeInput(timeStr) {
    if (!timeStr) return 0;
    const parts = String(timeStr).trim().split(':');
    let totalSeconds = 0;

    if (parts.length === 3) {
        totalSeconds = parseInt(parts[0], 10) * 3600 + parseInt(parts[1], 10) * 60 + parseFloat(parts[2]);
    } else if (parts.length === 2) {
        totalSeconds = parseInt(parts[0], 10) * 60 + parseFloat(parts[1]);
    } else {
        totalSeconds = parseFloat(timeStr);
    }

    return Number.isFinite(totalSeconds) ? Math.max(0, totalSeconds) : 0;
}

function clampTime(seconds, min, max) {
    return Math.min(Math.max(seconds, min), max);
}

function setupSliders() {
    const minSlider = document.getElementById('rangeMin');
    const maxSlider = document.getElementById('rangeMax');
    const startInput = document.getElementById('startTimeInput');
    const endInput = document.getElementById('endTimeInput');

    minSlider.max = videoDuration;
    maxSlider.max = videoDuration;

    minSlider.value = 0;
    maxSlider.value = videoDuration;
    trimSelectionTouched = false;

    startInput.value = formatTimeLimit(0);
    endInput.value = formatTimeLimit(videoDuration);

    updateSliderTrack();
}

function setupSliderEvents() {
    const minSlider = document.getElementById('rangeMin');
    const maxSlider = document.getElementById('rangeMax');
    const startInput = document.getElementById('startTimeInput');
    const endInput = document.getElementById('endTimeInput');

    minSlider.addEventListener('input', () => {
        trimSelectionTouched = true;
        let minVal = parseFloat(minSlider.value);
        const maxVal = parseFloat(maxSlider.value);
        if (minVal > maxVal - MIN_TRIM_GAP_SECONDS) {
            minVal = clampTime(maxVal - MIN_TRIM_GAP_SECONDS, 0, videoDuration);
            minSlider.value = minVal;
        }
        startInput.value = formatTimeLimit(minVal);
        updateSliderTrack();
    });

    maxSlider.addEventListener('input', () => {
        trimSelectionTouched = true;
        const minVal = parseFloat(minSlider.value);
        let maxVal = parseFloat(maxSlider.value);
        if (maxVal < minVal + MIN_TRIM_GAP_SECONDS) {
            maxVal = clampTime(minVal + MIN_TRIM_GAP_SECONDS, 0, videoDuration);
            maxSlider.value = maxVal;
        }
        endInput.value = formatTimeLimit(maxVal);
        updateSliderTrack();
    });

    startInput.addEventListener('change', () => {
        trimSelectionTouched = true;
        let sec = parseTimeInput(startInput.value);
        const maxVal = parseFloat(maxSlider.value);
        if (maxVal > 0 && sec > maxVal - MIN_TRIM_GAP_SECONDS) {
            sec = maxVal - MIN_TRIM_GAP_SECONDS;
        }
        sec = clampTime(sec, 0, videoDuration);
        minSlider.value = sec;
        startInput.value = formatTimeLimit(sec);
        updateSliderTrack();
    });

    endInput.addEventListener('change', () => {
        trimSelectionTouched = true;
        let sec = parseTimeInput(endInput.value);
        const minVal = parseFloat(minSlider.value);
        if (sec < minVal + MIN_TRIM_GAP_SECONDS) {
            sec = minVal + MIN_TRIM_GAP_SECONDS;
        }
        sec = clampTime(sec, 0, videoDuration);
        maxSlider.value = sec;
        endInput.value = formatTimeLimit(sec);
        updateSliderTrack();
    });
}

function updateSliderTrack() {
    const minSlider = document.getElementById('rangeMin');
    const maxSlider = document.getElementById('rangeMax');
    const track = document.getElementById('sliderTrack');

    if (videoDuration <= 0) return;

    const minPercent = (minSlider.value / videoDuration) * 100;
    const maxPercent = (maxSlider.value / videoDuration) * 100;

    track.style.background = `linear-gradient(to right, rgba(255,255,255,0.1) ${minPercent}%, #a855f7 ${minPercent}%, #6366f1 ${maxPercent}%, rgba(255,255,255,0.1) ${maxPercent}%)`;
}

async function startDownload() {
    const input = document.getElementById('urlInput');
    const btn = document.getElementById('downloadBtn');
    const btnText = document.getElementById('btnText');
    const btnLoader = document.getElementById('btnLoader');
    const status = document.getElementById('statusMessage');
    const resultCard = document.getElementById('resultCard');
    const filePath = document.getElementById('filePath');

    const dirInput = document.getElementById('dirInput');
    const qualitySelect = document.getElementById('qualitySelect');
    const audioToggle = document.getElementById('audioOnlyToggle');
    const playlistToggle = document.getElementById('playlistToggle');
    const startTimeInput = document.getElementById('startTimeInput');
    const endTimeInput = document.getElementById('endTimeInput');

    const url = input.value.trim();
    const downloadDir = dirInput.value.trim();
    const quality = qualitySelect.value;
    const audio_only = audioToggle.checked;
    const download_playlist = playlistToggle.checked;
    const start_time = startTimeInput.value.trim();
    const end_time = endTimeInput.value.trim();

    if (!url) {
        status.textContent = "Lütfen geçerli bir URL girin";
        status.className = "status error";
        return;
    }

    status.textContent = "";
    resultCard.classList.add('hidden');
    input.disabled = true;
    dirInput.disabled = true;
    qualitySelect.disabled = true;
    audioToggle.disabled = true;
    playlistToggle.disabled = true;
    startTimeInput.disabled = true;
    endTimeInput.disabled = true;
    btn.disabled = true;
    btnText.style.display = 'none';
    btnLoader.style.display = 'block';

    const progressContainer = document.getElementById('progressContainer');
    const progressBar = document.getElementById('progressBar');
    const progressPercent = document.getElementById('progressPercent');
    const playlistCounter = document.getElementById('playlistCounter');
    const downloadSpeed = document.getElementById('downloadSpeed');
    const progressInfo = document.getElementById('progressInfo');

    progressContainer.classList.remove('hidden');
    progressPercent.innerText = "0%";
    playlistCounter.innerText = "";
    playlistCounter.style.display = download_playlist ? 'inline-block' : 'none';
    downloadSpeed.innerText = "Bağlanılıyor...";
    progressInfo.innerText = "0.0MB / 0.0MB";
    progressBar.style.width = "0%";

    isDownloading = true;

    try {
        let final_start = null;
        let final_end = null;

        if (trimSelectionTouched && document.getElementById('trimmingSection').style.display !== 'none') {
            const currentStart = parseTimeInput(start_time);
            const currentEnd = parseTimeInput(end_time);
            const hasCustomStart = currentStart > TRIM_COMPARE_EPSILON;
            const hasCustomEnd = currentEnd < videoDuration - TRIM_COMPARE_EPSILON;

            if (hasCustomStart || hasCustomEnd) {
                final_start = start_time;
                final_end = end_time;
            }
        }

        const data = await invoke('start_download', {
            request: {
                url,
                download_dir: downloadDir || null,
                quality,
                audio_only,
                download_playlist,
                start_time: final_start,
                end_time: final_end,
            }
        });

        if (data.status === "cancelled") {
            status.textContent = "İndirme iptal edildi";
            status.className = "status error";
            return;
        }

        status.textContent = "İndirme başarılı!";
        status.className = "status success";
        filePath.textContent = "Kaydedildi: " + data.filename;
        currentSavedPath = data.full_path;
        resultCard.classList.remove('hidden');
        input.value = "";
        lastFetchedUrl = "";
        document.getElementById('trimmingSection').style.display = 'none';
    } catch (error) {
        status.textContent = typeof error === 'string' ? error : (error.message || "İndirme başarısız");
        status.className = "status error";
    } finally {
        isDownloading = false;

        const cancelBtn = document.getElementById('cancelBtn');
        cancelBtn.disabled = false;
        cancelBtn.innerText = "İptal Et";

        progressContainer.classList.add('hidden');
        input.disabled = false;
        dirInput.disabled = false;
        qualitySelect.disabled = false;
        audioToggle.disabled = false;
        playlistToggle.disabled = false;
        startTimeInput.disabled = false;
        endTimeInput.disabled = false;
        btn.disabled = false;
        btnText.style.display = 'block';
        btnLoader.style.display = 'none';
    }
}

async function browseFolder() {
    try {
        const path = await invoke('select_folder');
        if (path) {
            document.getElementById('dirInput').value = path;
        }
    } catch (e) {
        console.error("Failed to open folder picker", e);
    }
}

async function cancelDownload() {
    try {
        const btn = document.getElementById('cancelBtn');
        btn.disabled = true;
        btn.innerText = "İptal ediliyor...";
        await invoke('cancel_download');
    } catch (e) {
        console.error("Cancel failed", e);
    }
}

async function pollUpdateStatus() {
    try {
        const data = await invoke('get_update_status');
        if (data.status === 'checking') {
            setTimeout(pollUpdateStatus, 1000);
            return;
        }
        if (data.status === 'available') {
            document.getElementById('updateText').innerText =
                `yt-dlp g\u00fcncellemesi var (${data.current} \u2192 ${data.latest})`;
            document.getElementById('updateBanner').classList.remove('hidden');
        }
    } catch (e) {
        console.error("Update status failed", e);
    }
}

async function updateYtdlp() {
    const btn = document.getElementById('updateBtn');
    const text = document.getElementById('updateText');
    const banner = document.getElementById('updateBanner');
    btn.disabled = true;
    btn.innerText = "G\u00fcncelleniyor...";
    try {
        const updated = await invoke('update_ytdlp');
        text.innerText = `yt-dlp g\u00fcncellendi (${updated.current})!`;
        btn.classList.add('hidden');
        setTimeout(() => banner.classList.add('hidden'), 2500);
    } catch (e) {
        text.innerText = typeof e === 'string' ? e : "G\u00fcncelleme ba\u015far\u0131s\u0131z";
        btn.disabled = false;
        btn.innerText = "Tekrar Dene";
    }
}

async function pollAppUpdateStatus() {
    try {
        const data = await invoke('get_app_update_status');
        if (data.current) {
            document.getElementById('appVersion').innerText = data.current;
        }
        if (data.status === 'checking') {
            setTimeout(pollAppUpdateStatus, 1000);
            return;
        }
        if (data.status === 'available') {
            document.getElementById('appUpdateText').innerText =
                `Yeni uygulama s\u00fcr\u00fcm\u00fc var (${data.current} \u2192 ${data.latest})`;
            document.getElementById('appUpdateBanner').classList.remove('hidden');
        }
    } catch (e) {
        console.error("App update status failed", e);
    }
}

async function updateApp() {
    const btn = document.getElementById('appUpdateBtn');
    const text = document.getElementById('appUpdateText');
    btn.disabled = true;
    btn.innerText = "\u0130ndiriliyor...";

    let unlisten = null;
    try {
        unlisten = await listen('app-update-progress', (event) => {
            const pct = event.payload?.percent ?? 0;
            btn.innerText = pct < 100 ? `\u0130ndiriliyor... ${pct}%` : "Y\u00fckleniyor...";
        });
        await invoke('update_app');
        text.innerText = "G\u00fcncellendi, yeniden ba\u015flat\u0131l\u0131yor...";
    } catch (e) {
        text.innerText = typeof e === 'string' ? e : "Uygulama g\u00fcncellemesi ba\u015far\u0131s\u0131z";
        btn.disabled = false;
        btn.innerText = "Tekrar Dene";
    } finally {
        if (unlisten) unlisten();
    }
}

window.addEventListener('load', async () => {
    pollAppUpdateStatus();
    pollUpdateStatus();
    try {
        const data = await invoke('get_config');
        if (data.default_dir) {
            document.getElementById('dirInput').value = data.default_dir;
        }
        if (data.quality) {
            document.getElementById('qualitySelect').value = data.quality;
        }
    } catch (e) {
        console.error("Failed to fetch config", e);
    }
});

document.getElementById('qualitySelect').addEventListener('change', async (e) => {
    try {
        await invoke('set_quality', { quality: e.target.value });
    } catch (e) {
        console.error("Failed to save quality", e);
    }
});

async function openResultFolder() {
    if (!currentSavedPath) return;

    try {
        await invoke('open_folder', { filePath: currentSavedPath });
    } catch (e) {
        console.error("Failed to open folder", e);
    }
}
