# CornerPin

Telegram'ı (veya seçtiğin herhangi bir pencereyi) ekranın istediğin köşesine sabitler. Win + D, Win + M veya Win + Ok tuşlarıyla yerinden oynatılamaz; bekçi zamanlayıcı yarım saniye içinde pencereyi otomatik olarak yerine geri getirir. Tepsi simgesinden kapatana kadar sessizce çalışır.

## Özellikler

- Otomatik Telegram algılama, tepsi menüsünden başka pencere de seçilebilir
- 4 köşe: Sağ Alt, Sağ Üst, Sol Alt, Sol Üst
- 3 boyut modu: Mevcut Boyut, Tam Yükseklik 420 px, Tam Yükseklik 520 px
- Her Zaman Üstte seçeneği
- Windows İle Başlat seçeneği
- Ayarlar `%AppData%\CornerPin\settings.json` dosyasında saklanır
- Kurulum gerektirmez, tek exe, kayıt defterine dokunmaz (başlangıç seçeneği hariç)

## Kullanım

1. CornerPin.exe dosyasını çift tıkla
2. Sağ alttaki tepsi simgesine sağ tık
3. İstersen köşe ve boyut seç, hepsi bu

Pencereyi taşırsan, simge durumuna küçültürsen veya Win + D yaparsan pencere otomatik olarak yerine döner.

## Kaynaktan Derleme (Windows)

```
dotnet publish -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -o publish
```

Çıktı: `publish\CornerPin.exe`
