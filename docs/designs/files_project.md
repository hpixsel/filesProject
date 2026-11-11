# FilesProject — lekki eksplorator plików z wklejaniem przez RDP

* **Autor:** hpixsel (Kacper Adamus)
* **Data:** 2026-11-11
* **Status:** brainstorm, iteracja 1 (po recenzji z 6 perspektyw: architektura, szczegóły techniczne, zakres, zrozumiałość, gotowość do wdrożenia, struktura)

> Zastrzeżenie: recenzja opierała się wyłącznie na rozumowaniu, nic nie zostało przetestowane. Punkty oznaczone „(niezweryfikowane)” pochodzą z pamięci recenzentów i trzeba je sprawdzić w prototypie.

## Przegląd

Eksplorator plików tylko na Windows, pisany w Rust, dla jednego użytkownika (autora). Repozytorium jest publiczne na licencji MIT. Program ma zastąpić FilePilota, który ma dwa problemy:

1. Przy operacjach na plikach zacina cały komputer, a zużycie RAM i dysku gwałtownie rośnie.
2. Nie umie wkleić plików skopiowanych w sesji pulpitu zdalnego (RDP).

Scenariusz RDP: w sesji zdalnej użytkownik robi Ctrl+C w zwykłym Eksploratorze Windows na serwerze, potem Ctrl+V w nowym programie na komputerze lokalnym. Kierunek odwrotny nie jest na razie potrzebny.

Pozostałe funkcje z `CLAUDE.md` (zakładki, wiele paneli, motywy, ulubione, dyski sieciowe, PL/EN, aktualizacje, zastąpienie Eksploratora, CLI) przychodzą stopniowo, po rozwiązaniu tych dwóch problemów.

## Analiza

### 1. Sprzeczność w wymaganiach (blokuje start)
`CLAUDE.md` mówi o kopiowaniu RDP „w obie strony”, a w rozmowie ustaliliśmy tylko kierunek serwer → komputer lokalny. Kierunek odwrotny to znacznie więcej pracy: program musiałby sam wystawiać pliki do schowka z opóźnionym dostarczaniem zawartości. Trzeba poprawić `CLAUDE.md`, bo inaczej kolejna sesja potraktuje oba kierunki jako wymagane.

### 2. Budowa całego eksploratora nie jest jeszcze uzasadniona
- Zwykły Eksplorator Windows **już umie** wkleić pliki ze schowka RDP. Wymaganie nr 1 oznacza więc w praktyce: „FilePilot tego nie umie”.
- Nikt jeszcze nie sprawdził, skąd biorą się zacięcia. Możliwe przyczyny to sam FilePilot, Defender, ścieżki sieciowe albo pamięć podręczna plików Windows (strony „modified/standby”). To ostatnie najlepiej tłumaczy „puchnięcie RAM”: Windows buforuje kopiowane dane w RAM i potem je spłukuje na dysk.
- Test na godzinę: to samo kopiowanie i liczenie rozmiaru folderu w Eksploratorze, z otwartym Monitorem zasobów. Jeśli Eksplorator się nie zacina, to „Eksplorator + mały program pomocniczy do wklejania z RDP” pokrywa oba twarde wymagania.
- Jeśli prawdziwym celem jest „chcę mieć własny eksplorator”, to jest to uczciwy cel. Trzeba go jednak zapisać wprost, żeby było wiadomo, według czego oceniać cięcia zakresu.
- **Decyzja autora (iteracja 1): budujemy własny eksplorator od razu, bez wcześniejszego testu alternatyw.** Konsekwencje:
  - cięcia zakresu oceniamy według celu „własny, używalny na co dzień eksplorator”, a nie „najtańsze obejście problemu”;
  - pomiar zacięć w Eksploratorze nie blokuje już startu, ale nadal warto go zrobić: daje liczby bazowe do limitów z pkt 5 i pokazuje, czy problemem jest pamięć podręczna Windows (wtedy dotyczy też naszego programu).

### 3. Model wątków COM to główne źródło zarówno zacięć, jak i błędów wklejania
- Schowek OLE (`OleGetClipboard`) wymaga wątku STA z `OleInitialize`.
- Każdy odczyt `FileContents` (`IStream::Read`) to wywołanie do procesu `mstsc.exe`/`msrdc`, który pobiera dane przez sieć. Może blokować przez kilka sekund. (Drobna korekta: po stronie lokalnej dane wystawia klient RDP, a `rdpclip` działa po stronie serwera.)
- Zasady:
  - na każde wklejanie osobny wątek roboczy STA, który sam pobiera schowek;
  - wątek pompuje komunikaty;
  - czyta kawałkami po 1–4 MiB i między kawałkami sprawdza anulowanie;
  - watchdog, bo pojedynczego zablokowanego odczytu nie da się przerwać, a zerwana sesja może zawiesić odczyt zamiast zwrócić błąd;
  - **nigdy** nie dotykać strumienia z wątku UI.

### 4. Nie ma środowiska testowego na jednym komputerze
- Połączenie RDP do samego siebie (localhost) na Win11 Pro jest podobno blokowane albo zawodne (niezweryfikowane).
- Propozycja: maszyna wirtualna Hyper-V (Win11 w wersji ewaluacyjnej), połączenie przez `mstsc` z włączonym przekierowaniem schowka, plus jeden ręczny test na prawdziwym serwerze z pracy.
- W CI nie ma RDP. Testy automatyczne potrzebują więc sztucznego obiektu schowka w procesie, który podaje `FileGroupDescriptorW`/`FileContents` ze sztucznymi opóźnieniami, błędami i zawieszeniami.
- Zasady grupy (GPO) na serwerze klienta mogą blokować przekierowanie plików przez schowek. Win11 ma osobne zasady dla każdego kierunku. Trzeba to sprawdzić najpierw.

### 5. „Nie zacina się” i „lekki” nie mają liczb
Proponowane limity do potwierdzenia:
- wątek UI nigdy nie jest zablokowany dłużej niż 50 ms;
- RAM w spoczynku poniżej ok. 100 MB;
- bufor kopiowania stały, 1–4 MB;
- duże pliki (powyżej ok. 256 MB) zapisywane bez buforowania lub z `FlushFileBuffers` co N MB, żeby pamięć podręczna plików nie puchła;
- praca w tle w trybie `THREAD_MODE_BACKGROUND_BEGIN` (niski priorytet wejścia/wyjścia). Według dwóch recenzentów to właśnie usuwa „zacinanie całego komputera”;
- liczenie rozmiaru folderów: ok. 2 wątki na dysk fizyczny i 1 na udział sieciowy, anulowanie w ciągu 200 ms, trzymamy tylko sumy częściowe, a nie całe drzewo;
- test zaliczenia: plik 10–20 GB przy płaskim zużyciu RAM oraz drzewo 100 tys. plików bez zacięć systemu.

### 6. Dane ze schowka RDP to niezaufane dane z zewnątrz
- `cFileName` zawiera ścieżki względne, np. `dir\sub\plik`. Trzeba odrzucać `..`, ścieżki bezwzględne, litery dysków, `:` (alternatywne strumienie danych) i zarezerwowane nazwy (`CON`, `NUL`).
- Foldery rozpoznaje się tylko po `FILE_ATTRIBUTE_DIRECTORY` i nie mają zawartości. Rozmiar (`FD_FILESIZE`) może nie być podany, więc pasek postępu musi mieć tryb „nieznany rozmiar”. Pliki powyżej 4 GB używają `nFileSizeHigh`.
- Dane mogą przyjść jako `HGLOBAL` zamiast `IStream`. Nazwa jest ograniczona do 260 znaków, więc bardzo głębokie drzewa mogą się nie zmieścić.
- Zapis do pliku `.partial`, zmiana nazwy po zakończeniu, usunięcie po anulowaniu. Ścieżki `\\?\` oraz `longPathAware` w manifeście.

### 7. Bez wspólnego systemu zadań zacięcia wrócą
Wklejanie, kopiowanie, przenoszenie, usuwanie, liczenie rozmiaru i operacje sieciowe przechodzą przez **jedną kolejkę zadań**. Kolejka ma limity równoległości, ograniczone kanały, raportowanie postępu, tokeny anulowania i jedną politykę konfliktów nazw. Jeśli każda funkcja odpala własne wątki, wraca problem FilePilota.

### 8. Ścieżki sieciowe blokują UI
SMB potrafi zawiesić się na 30 sekund i dłużej. Listowanie, odczyt atrybutów i ikon zawsze poza wątkiem UI. Ikony przez mały pul wątków STA, z pamięcią podręczną według rozszerzenia. Lista musi być wirtualizowana, bo foldery mogą mieć ponad 100 tys. pozycji.

### 9. Biblioteka UI i integracja z powłoką Windows
Samo wklejanie z RDP **nie** wymusza wyboru biblioteki UI, bo Ctrl+V tylko uruchamia wątek roboczy. Wybór wymuszają:
- przeciąganie plików do i z Eksploratora: trzeba wyłączyć obsługę w `winit` i samemu zarejestrować `RegisterDragDrop`;
- natywne menu kontekstowe (`IContextMenu`): wymaga uchwytu okna i przekazywania komunikatów menu (podklasowanie okna).

Oba da się zrobić w eframe/egui + `windows-rs`, ale to ręcznie pisany COM, czyli dodatkowy kod.

### 10. Zastąpienie Eksploratora psuje powłokę
Przejęcie otwierania folderów psuje „Ten komputer”, Kosz, Panel sterowania, telefony MTP, foldery zip i `search-ms`. Wymagania:
- wszystko, co nie jest zwykłą ścieżką na dysku, przekazywać do `explorer.exe`;
- zapisywać tylko w `HKCU` i robić kopię poprzednich wartości;
- przywracanie bez GUI: `--restore-explorer` i osobny plik `.reg`.

### 11. Aktualizacje i podpisywanie
- Niepodpisany plik `.exe` wywoła ostrzeżenie SmartScreen. Opcje: SignPath Foundation, Azure Trusted Signing (sprawdzić, czy dostępne dla osoby prywatnej w Polsce) albo zaakceptować ostrzeżenie.
- Przed podmianą pliku sprawdzić SHA-256 i podpis minisign, bo repo jest publiczne.
- Działający plik `.exe` nie może nadpisać sam siebie: najpierw zmiana nazwy, potem podmiana, poprzednia wersja zostaje do wycofania.

### 12. Drobne
- Wklejanie do folderów chronionych wymaga podniesienia uprawnień. Albo przez `IFileOperation`, albo poza zakresem.
- Pole wersji w pliku konfiguracyjnym od pierwszego dnia.
- Tokeny kolorów i klucze tekstów (i18n) zdefiniować wcześnie, żeby motywy i tłumaczenia nie rozlały się po całym kodzie widoków.

## Cele i nie-cele

**W zakresie (kolejność kamieni milowych):**
- **M0:** prototyp bez UI: biblioteka plus CLI `rdppaste <folder_docelowy>`. Czyta schowek, zapisuje strumieniowo na dysk, odtwarza strukturę folderów, pokazuje postęp, pozwala anulować. Do tego polecenie diagnostyczne „pokaż formaty w schowku” i log `tracing` do pliku. Ten kod przyda się niezależnie od tego, czy skończy się na programie pomocniczym, czy na pełnym eksploratorze.
- **M1:** kolejka zadań plus lista plików w jednym panelu z wklejaniem; wybór biblioteki UI.
- **M2:** lokalne kopiowanie, przenoszenie i usuwanie; rozmiar folderów w tle.
- **M3:** zakładki, wiele paneli, ulubione, kopiowanie ścieżki bez cudzysłowów.
- **M4:** motywy, gęstość interfejsu, PL/EN.
- **M5:** dyski sieciowe, menu kontekstowe.
- **M6:** CLI, aktualizacje, zastąpienie Eksploratora. Każde z nich niezależne i do wyłączenia.

**Poza zakresem v1:**
- kopiowanie RDP w kierunku komputer lokalny → serwer (do decyzji, patrz Analiza pkt 1);
- motyw z tapety: w v1 tylko jasny/ciemny według ustawień systemu, kolor przewodni może później;
- więcej niż jeden poziom gęstości;
- przeglądanie wirtualnych folderów powłoki (MTP, zip, „Ten komputer”);
- własne cofanie operacji i Kosz: zrobi to `IFileOperation`;
- przeciąganie plików z programu na zewnątrz;
- auto-aktualizacje: jeden użytkownik może pobrać wydanie ręcznie.

## Rozważone alternatywy

| Alternatywa | Decyzja | Powód |
| :-- | :-- | :-- |
| **Przekazanie schowka do mechanizmu kopiowania Windows** (`IShellItem::BindToHandler(BHID_SFUIObject, IID_IDropTarget)`, potem `DragEnter`/`Drop`, albo czasownik „paste” z menu kontekstowego folderu) | **Sprawdzić jako pierwsze w M0** | Ok. 50 linii. Windows sam obsługuje strumieniowanie, postęp, anulowanie, konflikty i długie ścieżki. Wada: okno postępu jest Windowsa, a nie programu. Największe możliwe cięcie zakresu. Niezweryfikowane, czy działa z obiektem schowka klienta RDP poza Eksploratorem. |
| Własny odczyt `IStream` | Zapasowy | Gdy powyższe nie działa albo program musi mieć własny pasek postępu. |
| `SHFileOperation` / `IFileOperation` do wklejania z RDP | Odrzucone | `SHFileOperation` przyjmuje tylko ścieżki, a `IFileOperation` raczej nie przyjmie danych wirtualnych (niezweryfikowane). |
| `CopyFileExW` (bez buforowania dla dużych plików) lub `IFileOperation` do kopiowania lokalnego | Przyjęte | Bez pisania własnego silnika kopiowania. |
| Przekierowanie dysków `\\tsclient` | Odrzucone jako rozwiązanie | Działa w drugą stronę (serwer widzi dyski lokalne). Przydatne najwyżej jako obejście ręczne. |
| **Eksplorator + mały program pomocniczy** (skrót klawiszowy, pozycja „Wklej z RDP tutaj” w menu kontekstowym albo własna komenda w FilePilocie, jeśli ją obsługuje) | Odrzucone przez autora | Możliwe w jeden dzień, ale autor chce mieć własny eksplorator. CLI `rdppaste` z M0 i tak spełnia tę rolę jako produkt uboczny. |
| Directory Opus | Do przetestowania | Podobno obsługuje `FileGroupDescriptor` (niezweryfikowane). Płatny, zamknięty kod. |
| Files (C#/WinUI, open source) | Do przetestowania | Ciężki program, wklejanie plików wirtualnych działa w nim różnie. Poprawka w Files może być mniejszą pracą niż nowy program. |
| OneCommander, Total Commander | Godzina testów | Odrzucić, jeśli zawiodą w RDP lub się zacinają. |
| UI: eframe/egui + `windows-rs` | Rekomendowane | Najmniej kodu, dostęp do uchwytu okna; integrację przeciągania i menu trzeba dopisać ręcznie. |
| UI: Slint | Możliwe | Sprawdzić zgodność licencji z MIT (podobno nie jest licencją OSI; niezweryfikowane). |
| UI: iced | Odrzucone na teraz | Jeszcze niedojrzałe. |
| UI: Tauri/WebView2 | Odrzucone | Ponad 100 MB RAM na okno, co łamie limit z pkt 5. Przeciąganie obsługuje tylko ścieżki, a kod COM i tak trzeba napisać. |
| UI: WinUI3 z Rust | Odrzucone | Brak oficjalnego wsparcia. |
| Przechodzenie po folderach przez `jwalk` | Odrzucone | Rayon utrudnia ograniczenie równoległości. Zamiast tego `walkdir` lub `FindFirstFileExW` (`FIND_FIRST_EX_LARGE_FETCH`) na małej stałej puli wątków, z pomijaniem punktów ponownej analizy (reparse points). |

## Kluczowe decyzje i przygotowanie do kolejnych kroków

Do zapisania jako ADR (`docs/adr/NNNN-*.md`) przed pisaniem kodu:

1. **Kierunek RDP:** tylko serwer → lokalny, czy w obie strony. Poprawić `CLAUDE.md`.
2. ~~**Budować czy nie**~~ **Rozstrzygnięte:** budujemy własny eksplorator (decyzja autora, 2026-11-11). Alternatywy z tabeli zostają jako punkt odniesienia, a nie warunek startu.
3. **Mechanizm wklejania:** cel upuszczania powłoki (drop target) albo własny czytnik `IStream`. Czy okno postępu musi być własne?
4. **Silnik kopiowania lokalnego:** `CopyFileExW` albo `IFileOperation`.
5. **Model wątków:** wątek UI, osobny wątek STA na każde wklejanie, ograniczona pula do skanowania, komunikacja kanałami, watchdog, anulowanie na granicy kawałka.
6. **Limity zasobów z liczbami** (pkt 5) oraz sposób pomiaru: narzędzie i zestaw danych.
7. **Polityka konfliktów:** nadpisz, pomiń, zmień nazwę albo zapytaj (z „zastosuj do wszystkich”), plus obsługa plików częściowych.
8. **Biblioteka UI:** wybrana po M0, z zasadą, że da się ją wymienić.
9. **Struktura repozytorium:** `core` (czysty model, bez wywołań Windows) / `jobs` / `win` (jedyne miejsce z `unsafe`: COM, OLE, powłoka, rejestr) / `app` (logika widoków bez wiedzy o UI) / `ui-<biblioteka>`. W M0 wystarczy jeden plik binarny z modułami.
10. **Narzędzia:** przypięta wersja Rust (`rust-toolchain.toml`) i dokładna wersja crate'a `windows`; cel `x86_64-pc-windows-msvc`; CI na `windows-latest` (fmt, clippy, testy ze sztucznym schowkiem); wydanie po tagu.
11. **Dokumentacja dla kolejnych sesji:** `README.md`, `docs/ROADMAP.md`, `docs/ARCHITECTURE.md`, `docs/testing-rdp.md`, `docs/adr/`, a w `HANDOFF.md` sekcja „stan obecny / następny krok”. `CLAUDE.md` wskazuje te pliki, zamiast je powielać. Kod i dokumentacja po angielsku (publiczne repo), interfejs PL/EN.
12. **Podpisywanie:** zaakceptować SmartScreen albo skonfigurować podpis. Od tego zależy sposób aktualizacji.

### Otwarte pytania do autora
- Czy zacięcia występują też w Eksploratorze? Tylko na ścieżkach sieciowych, czy też na dyskach lokalnych? Czy wykluczono antywirusa?
- Jaki klient RDP (`mstsc` czy Windows App/MSRDC) i jakie wersje Windows na serwerach? Czy GPO na serwerach klientów pozwala przenosić pliki przez schowek?
- Czy FilePilot obsługuje własne komendy lub przyciski? Jeśli tak, program pomocniczy jest bardzo tani.
- Kopiowanie ścieżki: zachowanie jak Ctrl+Shift+C w Eksploratorze, tylko bez cudzysłowów? Jak rozdzielać wiele zaznaczonych plików (nowa linia)?
- „Zastąpienie Eksploratora”: co dokładnie? Win+E, otwieranie folderów, „pokaż w folderze” z innych programów? Okien dialogowych Otwórz/Zapisz nie da się realnie zastąpić.
- Rozmiar folderów: na żądanie dla wybranego folderu, czy jako automatyczna kolumna?
- Co, jeśli w trakcie przesyłania ktoś na serwerze skopiuje coś innego albo sesja się zerwie? Przerwać? Pokazać listę ukończonych plików i zaproponować ponowienie?
- Wklejania równolegle czy w kolejce? Minimalna wersja Windows? Jeden plik `.exe` czy instalator?
