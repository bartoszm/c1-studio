# C1 Studio: szybka przeglądarka i indekser katalogu Capture One (plan wstępny)

## Kontekst
Capture One na Windows nie ma API do skryptów. Jego katalog (`C:\c1_catalogs\fuji\fiuji.cocatalogdb`, ~700 MB, 99 tys. zdjęć) to baza SQLite. W WSL istnieje już `c1cat` (`~/capture_one_playground`): narzędzie w Pythonie z ~1 tys. linii przetestowanej wiedzy o schemacie tej bazy. Ta wiedza obejmuje lokalną kopię katalogu z indeksem (pełnotekstowym i geograficznym), wspólną składnię filtrów, bezpieczny zapis (backup, transakcja, blokada, gdy C1 jest otwarty), drzewo słów kluczowych, albumy, trzy poziomy obrazów (miniatura, podgląd, oryginał) i analizę przez Gemini (agy).

Cel: aplikacja desktopowa z interfejsem podobnym do C1. Ma dawać szybki podgląd i wyszukiwanie, gwiazdki, kolory, słowa kluczowe i kategorie, synchronizację z bazą C1 w obie strony, opisy i słowa kluczowe generowane przez AI oraz lokalną analizę zdjęć (dominujące kolory, główny motyw, orientacja, EXIF, ostrość, ekspozycja).

## Decyzje
**Platforma: tylko Windows, natywnie.** Uzasadnienie:
- Capture One, katalog (C:) i zdjęcia (H:) są na Windows.
- Z WSL odczyt `/mnt/c` jest ok. 5 razy wolniejszy; skan katalogu trwa tam 30 s.
- Wykrywanie działającego C1 (procesy, blokady plików) jest natywne dla Windows.

Rdzeń w Pythonie zostaje wieloplatformowy (`pathlib`, bez `fcntl`), więc CLI i plugin dalej działają w WSL.

**Bez Dockera.** Na Windows Docker działa w maszynie wirtualnej Linuksa, więc ma ten sam wolny dostęp do C: i H: co WSL. Nie ma też natywnego GUI ani dostępu do procesów Windows. Dystrybucja: instalator (MSI/NSIS).

**Środowisko deweloperskie: tutaj, na Windows** (`C:\projects\c1-studio`). Z WSL zostaje tylko `agy`, wywoływany przez `wsl.exe`, chyba że zainstalujesz wersję na Windows. Testy na fikstura schematu działają na obu systemach.

**Stos: całość w Rust (Tauri 2), tylko interfejs w Svelte/TS.**

Odpowiednik każdego elementu, który dziś jest w Pythonie:

| Potrzeba | Crate |
|---|---|
| indeks i wyszukiwanie (nie-SQL) | `tantivy` (+ `rstar`, opcjonalnie `roaring`) |
| magazyn danych aplikacji (klucz–wartość) | `redb` + `serde`/`postcard` |
| tylko synchronizacja z katalogiem C1 | `rusqlite` (bundled) |
| obrazy, JPEG-XL, EXIF | `image`, `jxl-oxide`, `kamadak-exif` |
| analiza (k-means, laplasjan, hash percepcyjny) | `ndarray`/`imageproc`, `image_hasher` |
| równoległość | `rayon`, `tokio` |

AI nie jest trudniejsze niż w Pythonie:
- **Claude API:** nie ma oficjalnego SDK dla Rusta, ale Messages API to prosty JSON (`reqwest` + `serde`). Ustrukturyzowaną odpowiedź wymusza narzędzie (tool) z JSON Schema.
- **Gemini przez agy:** wywołanie procesu.
- Opcjonalnie crate `rig` (wspólny interfejs dla Anthropic i Gemini).

Logowanie i ocena promptów: LangChain/LangSmith nie mają SDK dla Rusta, a orkiestracja agentów nie jest tu potrzebna (jedno wywołanie na zdjęcie lub arkusz). Śledzenie wywołań przez OpenTelemetry (`tracing` + `opentelemetry-otlp`). LangSmith i Langfuse przyjmują OTLP, więc LangSmith zostaje do podpięcia później bez zmian w kodzie. Na start: własna tabela `ai_runs` (prompt, model, koszt, odpowiedź, decyzja użytkownika) jako zbiór do ewaluacji.

**Ryzyko przepisania logiki zapisu:** dwa wiersze metadanych, dwie kolejności słów kluczowych, drzewo słów kluczowych (nested set). Ograniczamy je tak:
- Pythonowy `c1cat` zostaje jako **wzorzec (oracle)**. Testy różnicowe wykonują tę samą operację w Pythonie i w Rust na dwóch kopiach katalogu i porównują zrzut tabel `ZKEYWORD` i `ZIMAGEMETADATA`.
- Port idzie od odczytu do zapisu. Zapis jest włączany dopiero po zielonych testach różnicowych.
- Efekt uboczny: CLI `c1cat` w Rust działa natywnie na Windows, więc plugin Claude Code przestaje potrzebować pośrednictwa WSL. Pythonowy `c1cat` zostaje wycofany po zakończeniu portu.

Interfejs: Svelte (TS) w webview Tauri. Wirtualizowana siatka 100 tys. miniatur i ciemny motyw w stylu C1 (CSS); Rust serwuje obrazy przez własny protokół `c1img://` (miniatury `.cot` przekazywane bez dekodowania). Jeśli chcesz mieć 100% Rusta: Leptos (Rust kompilowany do WASM). Kosztem jest mniejszy ekosystem komponentów, dlatego Svelte jest domyślne.

**Repozytorium: monorepo `c1-studio` w `C:\projects` (zatwierdzone).** Python `c1cat` jest przenoszony z historią git (`git subtree add`) do `legacy/`. Jedno źródło prawdy dla rdzenia, CLI, pluginu, aplikacji i testów różnicowych.

Struktura (workspace Cargo):
```
c1-studio/
  crates/c1model/   # model kanoniczny, trait PhotoSource, Change, testy zgodności
  crates/source-c1/ # implementacja dla Capture One: catalog, safety, keywords, collections, images (.cot/.cop)
  crates/c1core/    # analiza, wyprawy, dziennik zdarzeń, geo (klastrowanie, geokodowanie, GPX)
  crates/c1index/   # tantivy (schemat, facety, zapytania, Filter) + redb (magazyn, kolejka) + rstar
  crates/c1ai/      # VisionProvider: claude, gemini_agy; prompty, ai_runs
  crates/c1cat/     # CLI (clap): zastępuje Pythonowy c1cat
  app/src-tauri/    # okno, komendy Tauri, protokół c1img://
  app/ui/           # Svelte
  plugin/           # skills/ + .claude-plugin/ (wywołuje c1cat.exe)
  legacy/c1cat-py/  # przeniesiony Python, wzorzec do testów różnicowych
  tests/fixtures/   # schema.sql z prawdziwego katalogu
```

## Architektura danych: bazy nie-SQL, wbudowane w aplikację
Cel: filtr z licznikami (facetami) poniżej 10 ms, wyszukiwanie tekstu poniżej 30 ms na 100 tys.+ zdjęć. W aplikacji nie ma żadnego SQL. SQLite dotyka tylko kod synchronizacji, bo katalog C1 jest w tym formacie.

**1. Indeks: `tantivy`.** Silnik wyszukiwania typu Lucene/Elasticsearch, w czystym Rust i wbudowany w aplikację, z własnym trwałym formatem segmentów na dysku. Jeden dokument to jeden wariant zdjęcia:
- **Pola tekstowe:** nazwa pliku, opis, nagłówek, opis AI, miejsce, słowa kluczowe. Polski stemming i usuwanie znaków diakrytycznych, wyszukiwanie przybliżone i po prefiksach.
- **Facety hierarchiczne:** `/kw/miejsca/polska/wroclaw`, `/album/...`, `/folder/...`, `/date/2026/05`, `/camera/...`, `/lens/...`, `/geo/kraj/miasto`, `/category/...`, `/ai/status`. Liczniki dla całego drzewa słów kluczowych i albumów przychodzą w jednym zapytaniu.
- **Fast fields (kolumnowe):** rating, kolor, data, ISO, ogniskowa, przysłona, orientacja, wymiary, ostrość, odcień dominującego koloru, lat/lon. Służą do filtrów zakresowych, sortowania i agregacji (histogramy dat, rozkład ocen).
- **Aktualizacje:** zmiana gwiazdek czy tagu to usunięcie i dodanie dokumentu, potem commit w paczkach (~co 200 ms) i odświeżenie czytnika. Interfejs widzi zmianę od razu, bo trzyma ją lokalnie, zanim commit się zakończy.
- **Geo:** zapytanie zakresowe po lat/lon (fast fields) plus `rstar` w pamięci do widoku mapy.

**2. Magazyn danych: `redb`.** Wbudowana baza klucz–wartość z transakcjami ACID, w czystym Rust. Tabele (klucz, potem wartość serializowana przez `serde`/`postcard`):
- `photo/<vpk>`: pełny rekord zdjęcia (metadane, EXIF, ścieżki, wartości z C1 w chwili ostatniej synchronizacji). To źródło, z którego budowany jest indeks `tantivy`, więc indeks zawsze da się odbudować bez katalogu C1.
- `analysis/<vpk>`: paleta, histogram, ostrość, hash percepcyjny, motyw.
- `ai/<vpk>/<run>` i `ai_runs/<id>`: sugestie i statusy, prompty, model, koszt.
- `queue/<seq>`: kolejka zmian do C1.
- `kwtree`, `albums`: kopie struktur C1.
- `sync_state`: znacznik ostatniej synchronizacji i hashe wierszy.

**3. Opcjonalnie: bitmapy roaring w RAM**, budowane z `redb` przy starcie. Tylko jeśli benchmark pokaże, że liczniki facetów w `tantivy` przekraczają 10 ms.

**4. Katalog C1:** wyłącznie źródło i cel synchronizacji (`rusqlite`, tylko w `c1core::catalog`). Przy aktualizacji przyrostowej czytane są tylko wiersze zmienione od ostatniej synchronizacji (`ZDATEMODIFIED` i porównanie hashy). Zmiany trafiają do `redb`, a stamtąd do `tantivy`.

Odrzucone: Meilisearch, Typesense i Elasticsearch (osobne serwery do zainstalowania obok aplikacji desktopowej) oraz MongoDB i SurrealDB (słabsze w facetach i wyszukiwaniu tekstu niż `tantivy`).

**Bramka w etapie 0 (benchmark):** syntetyczne 250 tys. dokumentów × 2000 słów kluczowych. 95. percentyl poniżej 10 ms dla 3–5 filtrów z licznikami i poniżej 30 ms dla tekstu z filtrami. Start aplikacji (otwarcie indeksu) poniżej 1 s.

**Stan aplikacji w `redb`, szczegóły:**
  - wyniki analizy (kolory, ostrość, histogram, motyw),
  - sugestie AI (status: zaproponowane, przyjęte, odrzucone),
  - **kolejka zmian** (rating, kolor, słowa kluczowe, opis, album),
  - opcjonalnie embeddingi.
- **Synchronizacja do C1.** Edycje w aplikacji trafiają natychmiast do kolejki, więc działają też przy otwartym C1. Przycisk „Sync do C1” jest aktywny tylko przy zamkniętym C1 i używa `safety.writer` (backup, `BEGIN IMMEDIATE`, sprawdzenie drzewa słów kluczowych). Przed zapisem pokazuje podgląd różnic.
- **Synchronizacja z C1.** Po aktualizacji indeksu z katalogu aplikacja porównuje wartości z kolejką. Gdy to samo pole zmieniło się po obu stronach, pokazuje konflikt do rozstrzygnięcia.
- **Dodatkowo w `redb`:** `trips/<id>` (wyprawy), `events/<ts>/<seq>` (dziennik zmian, potrzebny do time travel), `geocache/<cell>` (wyniki geokodowania).

## Kontrakt źródła danych (`c1model` + trait `PhotoSource`)
Aplikacja, indeks, mapa, AI i synchronizacja znają tylko **model kanoniczny i trait**. Capture One to jedna z implementacji (`crates/source-c1`). Kolejne źródło dodaje się jako nowy crate, bez zmian w aplikacji.

**Model kanoniczny** (`crates/c1model`, `serde`, wersjonowany przez `MODEL_VERSION`):
- `PhotoId { source: SourceId, ext: String }`: globalny identyfikator. `ext` to identyfikator w źródle, np. `variant_pk` w C1.
- `Photo`: `id`, `asset_id` (wspólny dla wariantów jednego pliku), `file` (nazwa, ścieżka, rozmiar, `online: bool`), `captured_at` (strefa czasowa opcjonalna), `imported_at`, `exif` (aparat, obiektyw, ISO, czas, przysłona, ogniskowa, wymiary, orientacja), `rating: Option<u8>`, `color: Option<ColorLabel>`, `keywords: Vec<KeywordPath>`, `iptc` (tytuł, opis, nagłówek, autor, copyright), `place` (miasto, region, kraj, lokalizacja), `geo: Option<GeoPoint{lat, lon, alt, source: Exif|Manual|Gpx|Geocoded}>`, `collections: Vec<CollectionId>`, `edited: bool`, `fingerprint: u64` (hash rekordu, do synchronizacji przyrostowej).
- `KeywordNode`, `Collection { id, name, kind: Album|Group|Smart|Folder|Project, parent }`.
- `Change`: zbiór operacji `SetRating`, `SetColor`, `AddKeywords`, `RemoveKeywords`, `SetIptc`, `SetPlace`, `SetGeo`, `AddToCollection`, `RemoveFromCollection`, `CreateCollection`. Każda z polem `base` (wartość oczekiwana w źródle, do wykrywania konfliktów).
- `ImageTier`: `Thumb` / `Preview` / `Original`. `ImageRef` to bajty albo ścieżka plus `ColorTransform` (np. balans bieli dla `.cop`).

**Trait** (async, `Send + Sync`):
```rust
trait PhotoSource {
  fn info(&self) -> SourceInfo;                     // id, nazwa, wersja, Capabilities
  fn capabilities(&self) -> Capabilities;           // które pola i Change są zapisywalne, czy są podglądy, geo, kolekcje
  async fn snapshot(&self, since: Option<Cursor>) -> Result<ChangeStream>; // Upsert(Photo) | Delete(PhotoId) | Tree(..) | Collections(..) + nowy Cursor
  async fn image(&self, id: &PhotoId, tier: ImageTier) -> Result<Option<ImageRef>>;
  async fn write_status(&self) -> WriteStatus;      // Ready | Locked{by: "CaptureOne.exe"} | ReadOnly
  async fn plan(&self, changes: &[Change]) -> Result<ApplyPlan>;    // dry run: różnice i konflikty z `base`
  async fn apply(&self, plan: ApplyPlan) -> Result<ApplyReport>;    // atomowo, z backupem, gdy źródło to wspiera
}
```

- **Testy zgodności** (`crates/c1model/conformance`): jeden zestaw testów, który musi przejść każda implementacja. Obejmuje: przyrostowe `snapshot` po `apply`, idempotencję, wykrywanie konfliktów, odmowę przy `Locked`, możliwości zgodne z rzeczywistością.
- **Kandydaci na kolejne źródła:**
  - `FolderSource`: folder plików z EXIF i XMP sidecar, zapis do XMP.
  - Lightroom Classic (`.lrcat`).
  - digiKam.
  - Immich lub PhotoPrism (REST).
  - Google Photos (tylko odczyt).
- **Kilka źródeł naraz:** indeks i UI obsługują je od początku; `source` jest facetem.

## Mapa
MapLibre GL JS w Svelte. Kafelki wektorowe przez OpenFreeMap albo Protomaps PMTiles (jeden plik offline). Rust serwuje dane geo przez komendę Tauri, z tym samym `Filter` co siatka miniatur.

**Widok zależny od oddalenia (jak punkty InPost):**

| Zoom | Widok |
|---|---|
| 0–5 (świat, kraje) | Okręgi klastrów z liczbą zdjęć, wielkość ~ log(liczby). Kolor wg wyprawy albo gęstości. Opcjonalnie mapa cieplna. |
| 6–11 (region, miasto) | Klastry jako kafelek miniatury najlepszego zdjęcia (najwyższy rating, potem ostrość) z liczbą w rogu. Najechanie pokazuje 4–9 miniatur. |
| 12–16 (ulica) | Pojedyncze pinezki z miniaturą. Zdjęcia z tej samej pozycji (seria) rozkładają się po kliknięciu w spiralę. |
| 17+ | Pinezki z kierunkiem zdjęcia, jeśli EXIF go ma (`GPSImgDirection`). |

- **Klastrowanie w Rust**, dzięki czemu szybkie przy 100 tys.+: siatka geohash lub kafelki zależne od zoomu, liczone z `rstar` na zbiorze po filtrach. Zwracane są tylko klastry widoczne w kadrze (~1–3 ms). Odpowiednik algorytmu `supercluster`.
- **Mapa jako filtr:** zaznaczony prostokąt lub lasso staje się filtrem geo dla siatki. Kliknięcie klastra filtruje siatkę do jego zdjęć.
- **Edycja lokalizacji:**
  - przeciągnięcie pinezki (jednej lub zaznaczonych);
  - przeciągnięcie zdjęć z paska miniatur na mapę nadaje im współrzędne;
  - pole „Szukaj miejsca” (geokodowanie przez Photon/Nominatim, z cache) ustawia współrzędne i pola miejsca;
  - przycisk „Uzupełnij miasto/kraj” robi geokodowanie odwrotne, offline z bazy miast GeoNames;
  - import śladu GPX z wyprawy przypisuje współrzędne po czasie wykonania zdjęcia, z przesunięciem strefy czasowej.

  Każda edycja to `Change::SetGeo` i `SetPlace` w kolejce, z cofaniem. Uwaga dla C1: współrzędne są zapisane na obrazie (`ZIMAGE`), nie na wariancie, więc zmiana dotyczy wszystkich wariantów danego zdjęcia. UI to sygnalizuje.

## Time travel: oś czasu i wyprawy
Dwa wymiary czasu.

**1. Czas wykonania zdjęć: wyprawy.**
- **Automatyczne wykrywanie wypraw:** grupowanie zdjęć po czasie i odległości. Nowa wyprawa zaczyna się po przerwie dłuższej niż X godzin albo po skoku dalej niż Y km od „domu” (domyślnie 36 h i 150 km, do ustawienia). Nazwa jest proponowana z geokodowania i dat, np. „Gruzja · 3–14.05.2026”.
- **Edycja wypraw:** łączenie, dzielenie, zmiana nazwy. Wyprawy zapisane w `redb` są facetem `/trip/...`. Opcjonalnie eksport jako album do C1.
- **UI:** pozioma oś czasu nad siatką i mapą, z histogramem zdjęć na dzień, tydzień lub miesiąc. Zaznaczenie zakresu działa jako filtr, a kolorowe pasy oznaczają wyprawy.
- **Odtwarzanie:** suwak przewija czas, na mapie rysuje się trasa (linia między kolejnymi pozycjami), a zdjęcia pojawiają się w miarę przesuwania suwaka.

**2. Czas w katalogu: jak zmieniała się kolekcja.**
- **Dziennik zdarzeń:** każda synchronizacja zapisuje różnice jako zdarzenia (dodane, usunięte, zmiana gwiazdek, słowa kluczowego, albumu, lokalizacji), ze znacznikiem czasu i źródłem (C1, aplikacja, AI).
- **Historia wstecz:** od daty importu (`imported_at`) da się odtworzyć, co i kiedy doszło. Dokładna historia edycji jest dostępna od pierwszego uruchomienia aplikacji.
- **Widoki:**
  - „Stan na dzień X” (ile zdjęć, jakie oceny, słowa kluczowe);
  - „Co doszło z wyprawy” (zdjęcia z wyprawy podzielone na import, ocenę i publikację);
  - wykres przyrostu kolekcji;
  - historia pojedynczego zdjęcia (kto i kiedy zmienił ocenę lub tagi);
  - porównanie dwóch dat.
- **Historia jako wartość, nie tylko log:** w razie pomyłki dowolny stan z historii da się zamienić w zbiór `Change` i przywrócić.

## Etapy
0. **Szkielet, kontrakt `c1model` i odczyt w `source-c1`** (dobry start nauki Rusta; kontrakt powstaje najpierw, a `source-c1` jest jego pierwszą implementacją).
   - Workspace, Tauri 2 + Svelte, CI (`cargo test`, `clippy`).
   - Python przeniesiony do `legacy/` z prostą konfiguracją `uv`. Python dostaje warstwę platformy (Windows natywnie lub WSL), żeby działał jako wzorzec testów na Windows.
   - Port na Rust części tylko do odczytu: znajdowanie katalogu, ekstrakcja z katalogu C1 do `redb` (logika zapytań z `mirror.py`, które budowały tabelę `v`), `c1index` (schemat `tantivy`, facety, budowa z `redb`), filtry (struct `Filter` zamiast 45 flag), trzy poziomy obrazów (`.cot`, `.cop` JPEG-XL z balansem bieli, osadzony JPEG z RAF/DNG).
   - Benchmark wydajności (bramka opisana wyżej).
   - Testy różnicowe odczytu: ten sam filtr daje te same `variant_pk` w Pythonie (mirror) i w Rust (`tantivy`).
1. **Przeglądarka (tylko odczyt).**
   - Drzewo po lewej: albumy, foldery, słowa kluczowe, smart albumy.
   - Siatka miniatur z wirtualizacją; podgląd: miniatura, potem podgląd JPEG-XL z balansem bieli, potem oryginał z H: przy 100%.
   - Pasek filtrów (gwiazdki, kolor, rok, aparat, słowa kluczowe), wyszukiwanie pełnotekstowe.
   - Panel metadanych i EXIF.
   - Cel wydajności: przewijanie 99 tys. miniatur bez przycięć, wyszukiwanie poniżej 150 ms.
2. **Edycja i synchronizacja.** Wymaga portu zapisu: safety (`tasklist`, wyłączny dostęp do pliku przez `CreateFile` z `share=0`, backup przez SQLite backup API), dwa wiersze metadanych, słowa kluczowe i drzewo (nested set), albumy, każde z testami różnicowymi względem Pythona.
   - Skróty jak w C1 (0–5 gwiazdki, klawisze kolorów), wielokrotne zaznaczenie.
   - Słowa kluczowe z autouzupełnianiem z drzewa, albumy i kategorie.
   - Kolejka zmian, podgląd różnic, sync w obie strony, konflikty, lista backupów z przywracaniem.
3. **Analiza lokalna (`rayon`, `image`, `imageproc`)**, liczona w tle na podglądach:
   - orientacja pion/poziom,
   - dominujące kolory (k-means i paleta),
   - histogram i ekspozycja,
   - ostrość (wariancja laplasjanu),
   - duplikaty i serie (hash percepcyjny),
   - filtry po tych cechach.
4. **Mapa i time travel.**
   - Mapa z klastrami zależnymi od zoomu, mapa jako filtr.
   - Edycja geo: przeciąganie, geokodowanie, GPX.
   - Wykrywanie i edycja wypraw, oś czasu z odtwarzaniem trasy.
   - Dziennik zdarzeń zapisywany od etapu 2 (żeby historia się zbierała), widoki historii tutaj.
5. **AI (`c1ai`).** Wspólny trait `VisionProvider`:
   - **Gemini/agy:** port `vision.py` (arkusze 6×6, prompty, parsowanie JSON), tanio dla dużych partii.
   - **Claude API:** pojedyncze zdjęcie (podgląd 1568 px), ustrukturyzowana odpowiedź. Zwraca słowa kluczowe (mapowane na istniejące drzewo), opis po polsku i angielsku, główny motyw, ocenę kompozycji i jakości, propozycję gwiazdek.
   - Ekran przeglądu sugestii: przyjmij lub odrzuć pojedynczo albo hurtem. Przyjęte trafiają do kolejki zmian.
6. **Dopracowanie.** Instalator, ustawienia (ścieżka katalogu, klucze API), obsługa wielu katalogów, opcjonalnie wyszukiwanie semantyczne (embeddingi CLIP/SigLIP), drugie źródło danych (`FolderSource`) jako sprawdzian kontraktu.

## Interfejs (wzorowany na C1)
- Ciemny motyw (#1e1e1e).
- Lewy panel z zakładkami narzędzi: Biblioteka, Metadane, Słowa kluczowe, AI, Analiza, Wyprawy, Historia.
- Na środku podgląd, pod nim pasek miniatur lub pełna siatka (przełączane jak w C1). Trzeci tryb to **Mapa** z paskiem miniatur, w którym siatka i mapa pokazują ten sam wynik filtra.
- Oś czasu jako zwijany pasek nad obszarem środkowym.
- U góry pasek narzędzi z filtrami i wyszukiwaniem; na dole pasek statusu (stan C1, liczba zmian w kolejce, przycisk Sync).

## Weryfikacja
- **Testy jednostkowe `c1core`** (`cargo test`): port 48 testów z Pythona plus nowe dla filtrów, kolejki i konfliktów, na fikstura schematu.
- **Testy różnicowe Rust vs Python** na kopiach katalogu dla każdej operacji zapisu (porównanie zrzutów tabel).
- **Test synchronizacji:** sync do kopii katalogu, potem `PRAGMA integrity_check` i `check_tree`. Na koniec ręcznie: otwarcie kopii katalogu w C1 i sprawdzenie, czy widać gwiazdki i słowa kluczowe.
- **Wydajność:** pomiar zimnego startu, przewijania siatki i czasu wyszukiwania na prawdziwym katalogu.
- **AI:** 1 arkusz w Gemini i 10 zdjęć w Claude, potem przegląd sugestii, kolejka i sync do kopii katalogu.
- **Kontrakt:** testy zgodności przechodzą dla `source-c1` i dla testowego źródła w pamięci (`MemorySource`).
- **Mapa:** klastry przy 100 tys. punktów poniżej 5 ms na każdy ruch mapy. Przeciągnięcie pinezki trafia do kolejki, a po sync współrzędne są w kopii katalogu.
- **Wyprawy:** wykrywanie na prawdziwym katalogu, ręczny przegląd 2026 roku (wyprawy wyglądają sensownie i da się je poprawić).
