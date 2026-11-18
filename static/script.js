let currentSavedPath = ""; // Global tracker for the last saved file
let videoDuration = 0; // Total video length in seconds
let trimSelectionTouched = false;
let infoAbortController = null;
let lastFetchedUrl = "";
const TRIM_COMPARE_EPSILON = 0.001;
const MIN_TRIM_GAP_SECONDS = 0.01;

// Add listeners on DOM load
document.addEventListener("DOMContentLoaded", () => {
    const urlInput = document.getElementById('urlInput');
    const playlistToggle = document.getElementById('playlistToggle');
    const trimmingSection = document.getElementById('trimmingSection');

    async function fetchInfoForUrl() {
        if (urlInput.disabled) return;
        let url = urlInput.value.trim();
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

        if (infoAbortController) {
            infoAbortController.abort();
        }
        infoAbortController = new AbortController();

        titleLabel.innerText = "Bilgi getiriliyor...";

        try {
            const res = await fetch('/api/info', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ url: url }),
                signal: infoAbortController.signal
            });
            const data = await res.json();

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
            if (e.name !== 'AbortError') {
                titleLabel.innerText = "Hata oluştu.";
                trimmingSection.style.display = 'none';
            }
        }
    }

    urlInput.addEventListener('blur', fetchInfoForUrl);
    urlInput.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
            if (infoAbortController) {
                infoAbortController.abort();
            }
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
});

// Time formatting helpers
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

// Slider logic
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
        let maxVal = parseFloat(maxSlider.value);
        if (minVal > maxVal - MIN_TRIM_GAP_SECONDS) {
            minVal = clampTime(maxVal - MIN_TRIM_GAP_SECONDS, 0, videoDuration);
            minSlider.value = minVal;
        }
        startInput.value = formatTimeLimit(minVal);
        updateSliderTrack();
    });

    maxSlider.addEventListener('input', () => {
        trimSelectionTouched = true;
        let minVal = parseFloat(minSlider.value);
        let maxVal = parseFloat(maxSlider.value);
        if (maxVal < minVal + MIN_TRIM_GAP_SECONDS) {
            maxVal = clampTime(minVal + MIN_TRIM_GAP_SECONDS, 0, videoDuration);
            maxSlider.value = maxVal;
        }
        endInput.value = formatTimeLimit(maxVal);
        updateSliderTrack();
    });

    // Update sliders when text inputs are manually changed
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

    let minPercent = (minSlider.value / videoDuration) * 100;
    let maxPercent = (maxSlider.value / videoDuration) * 100;

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

    // Reset state
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

    let isDownloading = true;

    // Polling function
    const pollInterval = setInterval(async () => {
        if (!isDownloading) return;
        try {
            const res = await fetch('/api/progress');
            if (!isDownloading) return;
            const data = await res.json();
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
        } catch (e) {
            console.error("Progress poll failed", e);
        }
    }, 800);

    try {
        let final_start = null;
        let final_end = null;

        // Send trim parameters if trimmed or manually set
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

        const response = await fetch('/api/download', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({
                url: url,
                download_dir: downloadDir,
                quality: quality,
                audio_only: audio_only,
                download_playlist: download_playlist,
                start_time: final_start,
                end_time: final_end
            }),
        });

        const data = await response.json();

        if (response.ok) {
            if (data.status === "cancelled") {
                status.textContent = "İndirme iptal edildi";
                status.className = "status error";
                return;
            }
            status.textContent = "İndirme başarılı!";
            status.className = "status success";
            filePath.textContent = "Kaydedildi: " + data.filename;
            currentSavedPath = data.full_path; // Store the full absolute path
            resultCard.classList.remove('hidden');
            input.value = ""; // Clear input on success
            lastFetchedUrl = "";
            document.getElementById('trimmingSection').style.display = 'none';
        } else {
            let errorMsg = "İndirme başarısız";
            if (data.detail) {
                if (typeof data.detail === 'string') {
                    errorMsg = data.detail;
                } else if (Array.isArray(data.detail)) {
                    errorMsg = data.detail.map(e => e.msg || JSON.stringify(e)).join(', ');
                }
            }
            throw new Error(errorMsg);
        }
    } catch (error) {
        status.textContent = error.message;
        status.className = "status error";
    } finally {
        isDownloading = false;
        clearInterval(pollInterval);

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
        const response = await fetch('/api/select_folder');
        const data = await response.json();
        if (data.path) {
            document.getElementById('dirInput').value = data.path;
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
        await fetch('/api/cancel', { method: 'POST' });
    } catch (e) {
        console.error("Cancel failed", e);
    }
}

// yt-dlp update check: the server checks once at startup; this only waits for that result.
async function pollUpdateStatus() {
    try {
        const response = await fetch('/api/update_status');
        const data = await response.json();
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
    btn.disabled = true;
    btn.innerText = "G\u00fcncelleniyor...";
    try {
        const response = await fetch('/api/update_ytdlp', { method: 'POST' });
        if (!response.ok) {
            const err = await response.json().catch(() => ({}));
            throw new Error(err.detail || "G\u00fcncelleme ba\u015far\u0131s\u0131z");
        }
    } catch (e) {
        text.innerText = e.message;
        btn.disabled = false;
        btn.innerText = "Tekrar Dene";
        return;
    }
    text.innerText = "Yeniden ba\u015flat\u0131l\u0131yor...";
    btn.classList.add('hidden');

    // Wait for the old server to go down and the new one to come up, then reload.
    let sawDown = false;
    const started = Date.now();
    const tick = async () => {
        try {
            const response = await fetch('/api/update_status', { cache: 'no-store' });
            if (response.ok && (sawDown || Date.now() - started > 15000)) {
                location.reload();
                return;
            }
        } catch (e) {
            sawDown = true;
        }
        setTimeout(tick, 1000);
    };
    setTimeout(tick, 1000);
}

// Fetch default config on load
window.addEventListener('load', async () => {
    pollUpdateStatus();
    try {
        const response = await fetch('/api/config');
        const data = await response.json();
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

// Quality change listener
document.getElementById('qualitySelect').addEventListener('change', async (e) => {
    try {
        await fetch('/api/set_quality', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ quality: e.target.value })
        });
    } catch (e) {
        console.error("Failed to save quality", e);
    }
});

// Heartbeat system to keep server alive
async function sendHeartbeat() {
    try {
        await fetch('/api/heartbeat');
    } catch (e) {
        console.error("Heartbeat failed", e);
    }
}
sendHeartbeat();
setInterval(sendHeartbeat, 3000);

window.addEventListener('beforeunload', () => {
    try {
        navigator.sendBeacon('/api/shutdown');
    } catch (e) {
        console.error("Shutdown beacon failed", e);
    }
});

async function openResultFolder() {
    if (!currentSavedPath) return;

    try {
        await fetch('/api/open_folder', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ file_path: currentSavedPath })
        });
    } catch (e) {
        console.error("Failed to open folder", e);
    }
}
