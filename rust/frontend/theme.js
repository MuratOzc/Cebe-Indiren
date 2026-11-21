(() => {
    const MODES = ['system', 'light', 'dark'];
    const root = document.documentElement;
    const android = /Android/i.test(navigator.userAgent);
    root.classList.toggle('android', android);
    if (android) {
        document.title = 'Cebe İndiren';
        document.addEventListener('DOMContentLoaded', () => {
            const title = document.querySelector('h1');
            if (title) title.textContent = 'Cebe İndiren';
        });
    }
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
