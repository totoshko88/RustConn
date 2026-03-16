#!/usr/bin/env python3
"""Fill translations for new i18n strings from audit fixes in RustConn 0.10.0.

Covers: RDP file import dialog, statistics (Most Used, Protocol Distribution),
split view toast message.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fill_translations import parse_po_file, extract_msgid, extract_msgstr, rebuild_po_file

TRANSLATIONS = {
    "uk": {
        "RDP File (.rdp)": "Файл RDP (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Імпортувати RDP-з'єднання з файлу Microsoft .rdp",
        "Select RDP File": "Виберіть файл RDP",
        "RDP Files (*.rdp)": "Файли RDP (*.rdp)",
        "Most Used": "Найчастіше використовувані",
        "Top connections by usage": "Найпопулярніші з'єднання",
        "Protocol Distribution": "Розподіл за протоколами",
        "sessions": "сеансів",
        "Split view is available for terminal-based sessions only": "Розділений вигляд доступний лише для термінальних сеансів",
    },
    "de": {
        "RDP File (.rdp)": "RDP-Datei (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "RDP-Verbindung aus einer Microsoft .rdp-Datei importieren",
        "Select RDP File": "RDP-Datei auswählen",
        "RDP Files (*.rdp)": "RDP-Dateien (*.rdp)",
        "Most Used": "Am häufigsten verwendet",
        "Top connections by usage": "Meistgenutzte Verbindungen",
        "Protocol Distribution": "Protokollverteilung",
        "sessions": "Sitzungen",
        "Split view is available for terminal-based sessions only": "Geteilte Ansicht ist nur für terminalbasierte Sitzungen verfügbar",
    },
    "fr": {
        "RDP File (.rdp)": "Fichier RDP (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Importer une connexion RDP depuis un fichier Microsoft .rdp",
        "Select RDP File": "Sélectionner un fichier RDP",
        "RDP Files (*.rdp)": "Fichiers RDP (*.rdp)",
        "Most Used": "Les plus utilisés",
        "Top connections by usage": "Connexions les plus utilisées",
        "Protocol Distribution": "Répartition par protocole",
        "sessions": "sessions",
        "Split view is available for terminal-based sessions only": "La vue divisée est disponible uniquement pour les sessions terminales",
    },
    "es": {
        "RDP File (.rdp)": "Archivo RDP (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Importar conexión RDP desde un archivo Microsoft .rdp",
        "Select RDP File": "Seleccionar archivo RDP",
        "RDP Files (*.rdp)": "Archivos RDP (*.rdp)",
        "Most Used": "Más utilizados",
        "Top connections by usage": "Conexiones más utilizadas",
        "Protocol Distribution": "Distribución por protocolo",
        "sessions": "sesiones",
        "Split view is available for terminal-based sessions only": "La vista dividida solo está disponible para sesiones de terminal",
    },
    "it": {
        "RDP File (.rdp)": "File RDP (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Importa connessione RDP da un file Microsoft .rdp",
        "Select RDP File": "Seleziona file RDP",
        "RDP Files (*.rdp)": "File RDP (*.rdp)",
        "Most Used": "Più utilizzati",
        "Top connections by usage": "Connessioni più utilizzate",
        "Protocol Distribution": "Distribuzione per protocollo",
        "sessions": "sessioni",
        "Split view is available for terminal-based sessions only": "La vista divisa è disponibile solo per le sessioni terminale",
    },
    "pl": {
        "RDP File (.rdp)": "Plik RDP (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Importuj połączenie RDP z pliku Microsoft .rdp",
        "Select RDP File": "Wybierz plik RDP",
        "RDP Files (*.rdp)": "Pliki RDP (*.rdp)",
        "Most Used": "Najczęściej używane",
        "Top connections by usage": "Najpopularniejsze połączenia",
        "Protocol Distribution": "Rozkład protokołów",
        "sessions": "sesji",
        "Split view is available for terminal-based sessions only": "Widok podzielony jest dostępny tylko dla sesji terminalowych",
    },
    "cs": {
        "RDP File (.rdp)": "Soubor RDP (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Importovat připojení RDP ze souboru Microsoft .rdp",
        "Select RDP File": "Vybrat soubor RDP",
        "RDP Files (*.rdp)": "Soubory RDP (*.rdp)",
        "Most Used": "Nejpoužívanější",
        "Top connections by usage": "Nejpoužívanější připojení",
        "Protocol Distribution": "Rozložení protokolů",
        "sessions": "relací",
        "Split view is available for terminal-based sessions only": "Rozdělené zobrazení je dostupné pouze pro terminálové relace",
    },
    "sk": {
        "RDP File (.rdp)": "Súbor RDP (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Importovať pripojenie RDP zo súboru Microsoft .rdp",
        "Select RDP File": "Vybrať súbor RDP",
        "RDP Files (*.rdp)": "Súbory RDP (*.rdp)",
        "Most Used": "Najpoužívanejšie",
        "Top connections by usage": "Najpoužívanejšie pripojenia",
        "Protocol Distribution": "Rozloženie protokolov",
        "sessions": "relácií",
        "Split view is available for terminal-based sessions only": "Rozdelené zobrazenie je dostupné iba pre terminálové relácie",
    },
    "da": {
        "RDP File (.rdp)": "RDP-fil (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Importér RDP-forbindelse fra en Microsoft .rdp-fil",
        "Select RDP File": "Vælg RDP-fil",
        "RDP Files (*.rdp)": "RDP-filer (*.rdp)",
        "Most Used": "Mest brugte",
        "Top connections by usage": "Mest brugte forbindelser",
        "Protocol Distribution": "Protokolfordeling",
        "sessions": "sessioner",
        "Split view is available for terminal-based sessions only": "Delt visning er kun tilgængelig for terminalbaserede sessioner",
    },
    "sv": {
        "RDP File (.rdp)": "RDP-fil (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Importera RDP-anslutning från en Microsoft .rdp-fil",
        "Select RDP File": "Välj RDP-fil",
        "RDP Files (*.rdp)": "RDP-filer (*.rdp)",
        "Most Used": "Mest använda",
        "Top connections by usage": "Mest använda anslutningar",
        "Protocol Distribution": "Protokollfördelning",
        "sessions": "sessioner",
        "Split view is available for terminal-based sessions only": "Delad vy är bara tillgänglig för terminalbaserade sessioner",
    },
    "nl": {
        "RDP File (.rdp)": "RDP-bestand (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "RDP-verbinding importeren uit een Microsoft .rdp-bestand",
        "Select RDP File": "RDP-bestand selecteren",
        "RDP Files (*.rdp)": "RDP-bestanden (*.rdp)",
        "Most Used": "Meest gebruikt",
        "Top connections by usage": "Meest gebruikte verbindingen",
        "Protocol Distribution": "Protocolverdeling",
        "sessions": "sessies",
        "Split view is available for terminal-based sessions only": "Gesplitste weergave is alleen beschikbaar voor terminalgebaseerde sessies",
    },
    "pt": {
        "RDP File (.rdp)": "Ficheiro RDP (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Importar ligação RDP de um ficheiro Microsoft .rdp",
        "Select RDP File": "Selecionar ficheiro RDP",
        "RDP Files (*.rdp)": "Ficheiros RDP (*.rdp)",
        "Most Used": "Mais utilizados",
        "Top connections by usage": "Ligações mais utilizadas",
        "Protocol Distribution": "Distribuição por protocolo",
        "sessions": "sessões",
        "Split view is available for terminal-based sessions only": "A vista dividida está disponível apenas para sessões de terminal",
    },
    "be": {
        "RDP File (.rdp)": "Файл RDP (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Імпартаваць RDP-злучэнне з файла Microsoft .rdp",
        "Select RDP File": "Выберыце файл RDP",
        "RDP Files (*.rdp)": "Файлы RDP (*.rdp)",
        "Most Used": "Найбольш выкарыстоўваныя",
        "Top connections by usage": "Найпапулярнейшыя злучэнні",
        "Protocol Distribution": "Размеркаванне па пратаколах",
        "sessions": "сеансаў",
        "Split view is available for terminal-based sessions only": "Падзелены выгляд даступны толькі для тэрмінальных сеансаў",
    },
    "kk": {
        "RDP File (.rdp)": "RDP файлы (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Microsoft .rdp файлынан RDP қосылымын импорттау",
        "Select RDP File": "RDP файлын таңдаңыз",
        "RDP Files (*.rdp)": "RDP файлдары (*.rdp)",
        "Most Used": "Ең көп қолданылатын",
        "Top connections by usage": "Ең көп қолданылатын қосылымдар",
        "Protocol Distribution": "Хаттама бойынша бөлу",
        "sessions": "сеанстар",
        "Split view is available for terminal-based sessions only": "Бөлінген көрініс тек терминал сеанстары үшін қолжетімді",
    },
    "uz": {
        "RDP File (.rdp)": "RDP fayli (.rdp)",
        "Import RDP connection from a Microsoft .rdp file": "Microsoft .rdp faylidan RDP ulanishni import qilish",
        "Select RDP File": "RDP faylini tanlang",
        "RDP Files (*.rdp)": "RDP fayllar (*.rdp)",
        "Most Used": "Eng ko'p ishlatiladigan",
        "Top connections by usage": "Eng ko'p ishlatiladigan ulanishlar",
        "Protocol Distribution": "Protokol bo'yicha taqsimot",
        "sessions": "seanslar",
        "Split view is available for terminal-based sessions only": "Bo'lingan ko'rinish faqat terminal seanslari uchun mavjud",
    },
}


def fill_translations(filepath, lang):
    """Fill empty translations in a .po file for the given language."""
    if lang not in TRANSLATIONS:
        print(f"  SKIP: No translations defined for '{lang}'")
        return 0

    trans = TRANSLATIONS[lang]
    entries = parse_po_file(filepath)
    filled = 0

    for entry in entries:
        msgid = extract_msgid(entry['msgid_lines'])
        msgstr = extract_msgstr(entry['msgstr_lines'])

        if msgstr == '' and msgid in trans:
            new_msgstr = trans[msgid]
            entry['msgstr_lines'] = [f'msgstr "{new_msgstr}"']
            filled += 1

    if filled > 0:
        content = rebuild_po_file(entries)
        with open(filepath, 'w', encoding='utf-8') as f:
            f.write(content)

    return filled


def main():
    po_dir = os.path.dirname(os.path.abspath(__file__))
    languages = [
        'uk', 'de', 'fr', 'es', 'it', 'pl', 'cs', 'sk',
        'da', 'sv', 'nl', 'pt', 'be', 'kk', 'uz',
    ]

    total_filled = 0
    for lang in languages:
        filepath = os.path.join(po_dir, f'{lang}.po')
        if not os.path.exists(filepath):
            print(f"  SKIP: {filepath} not found")
            continue
        filled = fill_translations(filepath, lang)
        total_filled += filled
        print(f"  {lang}: filled {filled} translations")

    print(f"\nTotal: {total_filled} translations filled across {len(languages)} languages")


if __name__ == '__main__':
    main()
