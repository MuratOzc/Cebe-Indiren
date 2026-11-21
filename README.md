# Video İndiren

Video İndiren, Windows için taşınabilir bir masaüstü video indirme uygulamasıdır.
[Tauri 2](https://tauri.app) ile geliştirilmiştir (Rust arka uç, sade HTML/CSS/JS arayüz) ve
arka planda [yt-dlp](https://github.com/yt-dlp/yt-dlp) ile [FFmpeg](https://ffmpeg.org) kullanır.

## Kullanım

[Releases](https://github.com/anilyildiz8/Video-Indiren/releases/latest) sayfasından
`VideoIndiren.exe` dosyasını indirip doğrudan çalıştırabilirsiniz; kurulum gerekmez.

## Özellikler

- yt-dlp'nin desteklediği her siteden MP4 (En iyi, 4K, 1440p, 1080p, 720p, 480p) veya MP3 indirme
- Kalite sınırı görüntünün kısa kenarına uygulanır; dikey bir video (Shorts, Reels, TikTok)
  1080p seçildiğinde 608x1080'e küçülmez, 1080x1920 iner
- Aynı anda birden fazla bağlantı: her satıra bir bağlantı yazın (veya bir liste yapıştırın),
  seçilen kalitede sırayla indirilir. Listedeki oynatma listeleri kendi klasörlerine iner,
  başarısız olan ya da iptal edilen bağlantılar yeniden denemek için alanda kalır, özet ise
  kaçının indirildiğini, zaten var olduğunu veya başarısız olduğunu gösterir
- Oynatma listesi indirme, her liste kendi klasörüne; oynatma listesi seçeneği açıkken tek bir
  video indirilirse normal bir video gibi kaydedilir
- MP3 dosyalarına başlık, sanatçı ve tarih etiketleri ile kapak resmi eklenir; klasörde aynı
  bağlantının videosu varsa ona dokunulmaz
- Zaten indirilmiş bir bağlantı tekrar indirilmez ve sonuç kartı bunu belirtir; tek tıkla
  mevcut dosyaya dokunmadan ayrı bir kopya (`Başlık [id] (2).mp4`) indirilebilir, örneğin
  başka bir kalitede
- Son indirmeler uygulama kapansa da listelenir: kapak resmi, boyut ve süreyle birlikte oynat,
  klasörde göster, bağlantıyı yeniden kullan ve kaldır düğmeleri ve bir arama kutusu; sonradan
  taşınan veya silinen dosyalar işaretlenir
- İsteğe bağlı SponsorBlock: YouTube videolarındaki sponsor, kendi tanıtım ve "abone ol"
  bölümleri kesilip çıkarılır, kırpılmış kliplerde de çalışır (aralık yine orijinal zaman
  çizelgesinde seçilir). Bölümler indirmeden önce önizlemedeki zaman çizelgesinde gösterilir.
  SponsorBlock'a ulaşılamazsa video bir notla kesilmeden kaydedilir
- Pencere odaklandığında panodaki bağlantı önerilir (sayfaya yalnızca bağlantılardan oluşan pano
  metni aktarılır); bağlantılar pencereye sürüklenip bırakılabilir
- Pencere, önizlenen veya indirilen videonun renklerini alır; ilerleme kartında kapak resmi,
  canlı hız grafiği ve çoklu indirmede sıra gösterilir
- İndirmeden önce başlık, süre ve kapak resmi önizlemesi
- Bir oynatma listesi bağlantısı, adı ve video sayısıyla listenin kendisi olarak önizlenir;
  listeden açılmış bir video, oynatma listesi seçeneği açılana kadar o video olarak önizlenir.
  Böylece önizleme her zaman indirilecek şeyi gösterir
- Başlangıç/bitiş aralık seçiciyle kırpma. Klipler tek bir yeniden kodlamayla tam seçilen
  karelerden kesilir ve her oynatıcıda düzgün oynar; uzun bir videonun kısa bir parçası için
  yalnızca o bölüm indirilir. Klipler aralıklarıyla adlandırılmış ayrı dosyalar olarak
  kaydedilir (`Başlık [id] (01.30-02.45).mp4`) ve aynı videonun tam indirmesinin yerini almaz
- Açık ve koyu tema: varsayılan olarak Windows temasını izler (ve değiştiğinde anında geçer),
  başlığın yanındaki düğmeyle açık veya koyu sabitlenebilir; seçim hatırlanır ve başlık çubuğu
  da uyar
- Pencerenin herhangi bir yerine bağlantı yapıştırma, Enter ile indirme (Shift+Enter yeni satır),
  Esc ile iptal; bitmiş dosyayı doğrudan sonuç kartından oynatma veya klasöründe gösterme;
  Ctrl+L bağlantı alanına gider, Ctrl+M video ile MP3 arasında geçiş yapar
- Güncel başlık, hız, boyut ve kalan süreyle canlı ilerleme, görev çubuğu düğmesinde de
  gösterilir. Bir indirmenin video ve ses akışları tek bir ilerleme sayılır, oynatma listesi
  çubuğu tüm videolar boyunca bir kez doldurur; her an iptal edilebilir
- Uzun başlıklar dosya adlarında kısaltılır, böylece indirmeler Windows yol sınırına takılmaz
- İndirme sürerken bilgisayar uyku moduna geçmez; indirme sırasında pencereyi kapatmak önce
  onay ister
- Giriş gerektiren içerikler için tarayıcı çerezlerine otomatik geçiş
- Hem yt-dlp hem de uygulamanın kendisi için SHA-256 ile doğrulanan dahili güncellemeler
  (yt-dlp sürüm sağlama dosyasına, uygulama ise GitHub'ın sürüm dosyası için kaydettiği özete
  göre). Güncel olmayan bir yt-dlp'den kaynaklanabilecek bir hata bekleyen güncellemeyi
  gösterir; açılışta başarısız olan güncelleme denetimi tekrarlanır

## Proje yapısı

```
rust/
  src/            Rust arka uç (Tauri komutları, yt-dlp/FFmpeg yönetimi, güncelleyiciler)
  frontend/       Arayüz (index.html, script.js, styles.css, dahili Outfit yazı tipi)
  binaries/       yt-dlp.exe, ffmpeg.exe, ffprobe.exe (build.rs indirir, depoda yer almaz)
  build.rs        Sabitlenmiş araç sürümleri ve SHA-256 değerleri
  tauri.conf.json
```

## Derleme

Gereksinimler: Rust araç zinciri ve Tauri CLI (`cargo install tauri-cli --version "^2"`).

```
cd rust
cargo tauri dev        # geliştirme modunda çalıştır
cargo tauri build      # yayın sürümü .exe dosyasını üret
cargo test             # birim testlerini çalıştır
```

İlk derlemede `build.rs`, sabitlenmiş yt-dlp sürümünü ve gyan.dev FFmpeg essentials paketini
(ffmpeg ve ffprobe) `rust/binaries/` klasörüne indirir ve SHA-256 değerlerini doğrular;
sonraki derlemeler çevrimdışı çalışır. Bir aracı yükseltmek için `build.rs` içindeki adresi ve
SHA-256 değerlerini birlikte değiştirin. Yayın derlemeleri araçları zstd ile sıkıştırıp
çalıştırılabilir dosyaya gömer; böylece `VideoIndiren.exe` yaklaşık 100 MB'lık tek bir taşınabilir
dosyadır ve ilk açılışta araçları `%APPDATA%\VideoIndiren\bin` klasörüne çıkarır. Geliştirme
derlemeleri araçları doğrudan `rust/binaries/` klasöründen kullanır.

## Sürümler ve güncellemeler

Uygulama sürümü yalnızca `rust/Cargo.toml` dosyasında tanımlıdır. Uygulama içi güncelleyici bu
sürümü GitHub'daki en son sürümle karşılaştırır; sürüm numarası etiketten (`v2.0.5`), etiket bir
sürüm numarası değilse sürüm adından okunur. Sürüme `VideoIndiren.exe` adlı bir dosya eklenmelidir.
GitHub bu dosyanın SHA-256 özetini otomatik olarak hesaplar; güncelleme yalnızca bu özet varsa
önerilir ve indirilen dosya ona göre doğrulanır.

Uygulama içinden güncellenen yt-dlp, uygulamanın kendisi güncellendiğinde gömülü eski kopyayla
değiştirilmez.

Ayarlar ve günlükler `%APPDATA%\VideoIndiren` klasöründe tutulur (`config.json`, `history.json`,
`app.log`).

## Lisans

MIT Lisansı ile dağıtılmaktadır. Ayrıntılar için [LICENSE](LICENSE) dosyasına bakın. Dahili Outfit
yazı tipi SIL Open Font License ile lisanslanmıştır (`rust/frontend/fonts/OFL.txt`).
