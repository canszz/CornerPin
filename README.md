# CornerPin

Telegram'ı (veya seçtiğin herhangi bir pencereyi) ekranın istediğin köşesine sabitler. Win + D, Win + M veya Win + Ok tuşlarıyla yerinden oynatılamaz; bekçi zamanlayıcı yarım saniye içinde pencereyi otomatik olarak yerine geri getirir. Tepsi simgesinden kapatana kadar sessizce çalışır.

Rust + saf Win32 ile yazıldı, tek exe **~300 KB**. Kurulum gerektirmez, .NET veya başka bir runtime istemez.

## Özellikler

- Otomatik Telegram algılama, tepsi menüsünden başka pencere de seçilebilir
- 4 köşe: Sağ Alt, Sağ Üst, Sol Alt, Sol Üst
- 3 boyut modu: Mevcut Boyut, Tam Yükseklik 420 px, Tam Yükseklik 520 px
- Her Zaman Üstte seçeneği
- Windows İle Başlat seçeneği
- Açıldığında bildirim balonu ile haber verir, ikinci kez tıklanırsa "zaten çalışıyor" der
- Ayarlar `%AppData%\CornerPin\settings.json` dosyasında saklanır

## Kullanım

1. CornerPin.exe dosyasını çift tıkla
2. Bildirim balonu "çalışıyorum" diyecek, simge sağ alttaki tepsiye yerleşir (gizliyse `^` okuna tıkla)
3. Tepsi simgesine sağ tık → istersen köşe ve boyut seç

Pencereyi taşırsan, simge durumuna küçültürsen veya Win + D yaparsan pencere otomatik olarak yerine döner.

## Kaynaktan Derleme (Windows)

```
rustup target add x86_64-pc-windows-msvc
cargo build --release
```

Çıktı: `target\release\cornerpin.exe`

## Kaynaktan Derleme (Linux'tan Windows'a)

```
rustup target add x86_64-pc-windows-gnu
sudo apt install -y mingw-w64
cargo build --release --target x86_64-pc-windows-gnu
```

Çıktı: `target/x86_64-pc-windows-gnu/release/cornerpin.exe`

## Eski Sürüm

İlk C# (WinForms) sürümü `legacy-csharp/` klasöründe duruyor, kullanman gerekmiyor.
