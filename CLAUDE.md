# FilesProject
Szybki, stabilny i lekki eksplorator plików dla Windows, napisany w Rust. Zastępuje FilePilota, który:
1. zacina cały komputer przy operacjach na plikach (rośnie zużycie RAM i dysku),
2. nie umie wkleić plików skopiowanych w sesji RDP.

Na razie tylko Windows i jeden użytkownik (autor). Repozytorium jest publiczne, licencja MIT.

Szczegółowa analiza, ryzyka i otwarte decyzje: `docs/designs/files_project.md` (brief) i `docs/designs/files_project_m0.md` (projekt M0).

## Zacznij tutaj
- `docs/HANDOFF.md` — stan obecny, następny krok, wyniki eksperymentów (czytaj jako pierwsze, aktualizuj na koniec każdego zadania).
- `docs/ARCHITECTURE.md` — crate'y i zasady wątków.
- `docs/ROADMAP.md` — kamienie milowe M0–M6.
- `docs/testing-rdp.md` — ręczne testy `rdppaste`.
- Zasada: `unsafe` i wywołania Windows (COM/OLE/Win32) tylko w crate `files-win`; pozostałe crate'y mają `unsafe_code = "forbid"`.
- UI (od M1): GPUI + gpui-kit (decyzja autora); szczegóły w `docs/research/ui-toolkit-2026-11.md`.

## Twarde wymagania (priorytet nad wszystkim innym)
- **Wklejanie z RDP:** Ctrl+C w Eksploratorze Windows na serwerze zdalnym, potem Ctrl+V w tym programie na komputerze lokalnym. Kierunek odwrotny (lokalny → serwer) jest na razie poza zakresem.
- **Program nigdy nie zacina systemu ani UI:**
  - wszystkie operacje na plikach, sieci i COM działają poza wątkiem UI;
  - wszystkie przechodzą przez jedną kolejkę zadań z limitem równoległości, postępem i anulowaniem;
  - zużycie RAM i dysku jest ograniczone, a duże pliki nie zapychają pamięci podręcznej Windows.

## Funkcje (wprowadzane stopniowo, po twardych wymaganiach)
- Liczenie rozmiaru plików i folderów: w tle, z możliwością przerwania
- Wiele paneli (multi pane) do pracy równoległej
- Wiele zakładek
- Ulubione foldery
- Kopiowanie ścieżki pliku bez cudzysłowów (jak Ctrl+Shift+C w Eksploratorze, ale bez `"`)
- Łatwe podpinanie dysków sieciowych
- Języki interfejsu: polski i angielski
- Integracja z CLI
- Łatwe aktualizacje z wydań na GitHubie
- Opcjonalne zastąpienie Eksploratora Windows, z łatwym przywróceniem domyślnego (także bez GUI, np. `--restore-explorer`)

## Design
- Motywy: ciemny, jasny, na podstawie tapety oraz dowolny kolor przewodni, z którego wyliczane są odcienie wszystkich paneli
- Gęstość interfejsu: zwykła, kompaktowa, bardzo kompaktowa
- Nowoczesna biblioteka UI, ale nie kosztem wydajności i integracji z Windows (schowek OLE, przeciąganie, menu kontekstowe)
- UX: płynny, bez zacięć i bez zbędnych kliknięć

## Sposób pracy
- Repozytorium GitHub jest do pełnej dyspozycji.
- Pisz jak najmniej kodu: czytelnego i łatwego w utrzymaniu. Używaj pluginu `ponytail`.
- Dokumentuj pracę dla kolejnych sesji. Na koniec każdej sesji zaktualizuj sekcję „stan obecny / następny krok”.
- Kod, komentarze i dokumentacja techniczna po angielsku (publiczne repo). Rozmowa z autorem po polsku.

## Narzędzia (skille i pluginy) — kiedy czego używać
- **Nowy kamień milowy / większa funkcja:** `/egm:egm-brainstorm` → `/egm:egm-draft` → `/egm:egm-review <plik>`. Dokumenty trafiają do `docs/designs/`, raporty recenzji do `docs/reviews/`. Skille EGM wywołuje autor komendą; agent nie może ich uruchomić narzędziem Skill (może przeczytać ich `SKILL.md` i postępować według niego).
- **Po implementacji kamienia milowego:** `/egm:egm-drift-report docs/designs` sprawdza, czy kod nadal zgadza się z projektami.
- **Pisanie kodu:** plugin `ponytail` (aktywny cały czas). Skróty ze znanym limitem oznaczamy komentarzem `shortcut: <limit>, <kiedy poprawić>`; `/ponytail-debt` wypisuje je jako listę długu.
- **Przed commitem / PR:** `/ponytail-review` (czy każda linia jest potrzebna) i `/code-review` (błędy). Zmiany w `files-win` i w obsłudze danych ze schowka dodatkowo przez `/security-review`.
- **Koniec kamienia milowego:** `/ponytail-audit` (całe repo: błędy, ryzyka, co usunąć) i `/simplify` na zmienionym kodzie.
- **UI (M1, M4):** `ui-ux-pro-max:ui-ux-pro-max` i `ui-ux-pro-max:design-system` tylko do palet, tokenów kolorów (motywy, kolor przewodni), skali odstępów (gęstość) i zasad UX. Ich przykłady stosu (shadcn, Tailwind) nie dotyczą tego projektu.
- **Uruchomienie aplikacji i zrzuty ekranu (M1+):** skill `run`.
- **README dla crate'ów (opcjonalnie, od M2):** `/egm:egm-bootstrap`, jeśli `docs/ARCHITECTURE.md` przestanie wystarczać.
- Nieprzydatne tutaj: skille Office/PDF, ZoriusPro, dataviz, claude-in-chrome.
