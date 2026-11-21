(() => {
    const MODES = ['system', 'light', 'dark'];
    const root = document.documentElement;
    root.classList.toggle('android', /Android/i.test(navigator.userAgent));
    const systemLight = window.matchMedia('(prefers-color-scheme: light)');
    let mode = MODES.includes(window.__APP_THEME__) ? window.__APP_THEME__ : 'system';

    function apply() {
        root.dataset.theme = mode === 'system' ? (systemLight.matches ? 'light' : 'dark') : mode;
    }

    systemLight.addEventListener('change', apply);
    apply();

    window.appTheme = {
        get mode() {
            return mode;
        },
        set(next) {
            if (!MODES.includes(next)) return;
            mode = next;
            apply();
        },
    };
})();
