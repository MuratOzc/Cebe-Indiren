# Cebe İndiren

Android için sade ve hızlı video indirme uygulaması. Windows uygulaması [Video İndiren](https://github.com/anilyildiz8/Video-Indiren)'in Android sürümüdür.

## İndir ve Kullan

[Son sürüm](https://github.com/MuratOzc/Cebe-Indiren/releases/latest) sayfasından `CebeIndiren.apk` dosyasını telefonunuza indirip kurun. Android ilk kurulumda bilinmeyen kaynaklardan yüklemeye izin vermenizi ister.

Android 7.0 ve üzeri, 64 bit ARM (arm64) işlemcili telefonlarda çalışır.

## Özellikler

- **Video ve Ses İndirme:** YouTube, X (Twitter), Instagram, Reddit, TikTok ve yüzlerce siteden MP4 (4K, 1440p, 1080p, 720p, 480p, En İyi) veya MP3 olarak indirme
- **Paylaş Menüsünden İndirme:** Herhangi bir uygulamada bağlantıyı paylaşıp "Cebe İndiren ile indir"i seçmeniz yeterli; panodaki bağlantıyı da tek dokunuşla ekleyebilirsiniz
- **Arka Planda İndirme:** Uygulamadan çıksanız da indirme sürer, ilerleme bildirimde görünür ve bitince haber verir
- **Video Kesme:** Videonun tamamı yerine sadece istediğiniz başlangıç ve bitiş aralığını kesip indirme
- **Sponsorları Otomatik Çıkarma:** İsteğe bağlı olarak YouTube videolarındaki sponsor, tanıtım ve "abone ol" bölümlerini otomatik atlama
- **Oynatma Listesi ve Çoklu İndirme:** Birden fazla bağlantıyı alt alta yapıştırarak sırayla indirme; oynatma listelerini kendi klasöründe düzenli kaydetme
- **İndirilenler Klasörüne Kayıt:** Dosyalar telefonun İndirilenler (Download) klasörüne iner; isterseniz içindeki bir alt klasörü seçebilirsiniz
- **İndirme Geçmişi:** İndirdiğiniz videoları uygulama içinden arama, doğrudan oynatma veya klasöründe açma
- **Giriş Gerektiren Videolar:** Uygulama içinden siteye giriş yaparak ya da `cookies.txt` dosyası içe aktararak hesap isteyen videoları indirme
- **Açık / Koyu Tema:** Telefonunuzun temasına otomatik uyum sağlar, üst köşedeki düğmeyle istediğiniz zaman değiştirebilirsiniz
- **Tek Dokunuşla Güncelleme:** Yeni sürüm çıktığında uygulama içinden güncellenir ve yenilikleri gösterir

## İzinler

- **Depolama:** Yalnızca Android 10 ve altında, İndirilenler klasörüne yazabilmek için istenir
- **Bildirimler:** İndirme ilerlemesini ve bittiğini göstermek için
- **Bilinmeyen uygulamaları yükleme:** Uygulama içi güncellemeyi kurabilmek için

## Derleme

Gereksinimler: Rust araç zinciri, Tauri CLI (`cargo install tauri-cli --version "^2"`), Android SDK ile NDK ve JDK 21.

```
cd rust
cargo tauri android build --apk --target aarch64
```

İmzalı yayın sürümü için `rust/android-release.ps1` kullanılır; imza bilgilerini `rust/gen/android/keystore.properties` dosyasından okur (bu dosya depoda yer almaz).

## Lisans

[MIT Lisansı](LICENSE) ile dağıtılmaktadır.
