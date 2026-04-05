#!/usr/bin/env python3
"""Fill empty translations in all .po files for RustConn 0.10.11 strings."""

import re
import os
import subprocess
import sys

# Translations for each language keyed by msgid
TRANSLATIONS = {
    "uk": {
        "Mouse Jiggler": "Рух миші",
        "Prevent idle disconnect by simulating mouse movement": "Запобігає від'єднанню через бездіяльність, імітуючи рух миші",
        "Jiggler Interval": "Інтервал руху",
        "Seconds between mouse movements": "Секунди між рухами миші",
        "Connect All": "З'єднати все",
        "Check if Online": "Перевірити доступність",
        "Username copied": "Ім'я користувача скопійовано",
        "No username configured": "Ім'я користувача не налаштовано",
        "Password copied (auto-clears in 30s)": "Пароль скопійовано (автоочищення через 30 с)",
        "Cached password is empty": "Кешований пароль порожній",
        "Connect first to cache credentials": "Спочатку з'єднайтесь для кешування облікових даних",
    },
    "de": {
        "Mouse Jiggler": "Mausbeweger",
        "Prevent idle disconnect by simulating mouse movement": "Leerlauftrennung durch simulierte Mausbewegung verhindern",
        "Jiggler Interval": "Bewegungsintervall",
        "Seconds between mouse movements": "Sekunden zwischen Mausbewegungen",
        "Connect All": "Alle verbinden",
        "Check if Online": "Online-Status prüfen",
        "Username copied": "Benutzername kopiert",
        "No username configured": "Kein Benutzername konfiguriert",
        "Password copied (auto-clears in 30s)": "Passwort kopiert (wird nach 30 s gelöscht)",
        "Cached password is empty": "Zwischengespeichertes Passwort ist leer",
        "Connect first to cache credentials": "Zuerst verbinden, um Anmeldedaten zwischenzuspeichern",
    },
    "fr": {
        "Mouse Jiggler": "Simulateur de souris",
        "Prevent idle disconnect by simulating mouse movement": "Empêche la déconnexion par inactivité en simulant un mouvement de souris",
        "Jiggler Interval": "Intervalle de mouvement",
        "Seconds between mouse movements": "Secondes entre les mouvements de souris",
        "Connect All": "Tout connecter",
        "Check if Online": "Vérifier la disponibilité",
        "Username copied": "Nom d'utilisateur copié",
        "No username configured": "Aucun nom d'utilisateur configuré",
        "Password copied (auto-clears in 30s)": "Mot de passe copié (effacement auto dans 30 s)",
        "Cached password is empty": "Le mot de passe en cache est vide",
        "Connect first to cache credentials": "Connectez-vous d'abord pour mettre en cache les identifiants",
    },
    "es": {
        "Mouse Jiggler": "Simulador de ratón",
        "Prevent idle disconnect by simulating mouse movement": "Evita la desconexión por inactividad simulando movimiento del ratón",
        "Jiggler Interval": "Intervalo de movimiento",
        "Seconds between mouse movements": "Segundos entre movimientos del ratón",
        "Connect All": "Conectar todo",
        "Check if Online": "Comprobar disponibilidad",
        "Username copied": "Nombre de usuario copiado",
        "No username configured": "Nombre de usuario no configurado",
        "Password copied (auto-clears in 30s)": "Contraseña copiada (se borra en 30 s)",
        "Cached password is empty": "La contraseña en caché está vacía",
        "Connect first to cache credentials": "Conéctese primero para almacenar credenciales",
    },
    "it": {
        "Mouse Jiggler": "Simulatore mouse",
        "Prevent idle disconnect by simulating mouse movement": "Impedisce la disconnessione per inattività simulando il movimento del mouse",
        "Jiggler Interval": "Intervallo di movimento",
        "Seconds between mouse movements": "Secondi tra i movimenti del mouse",
        "Connect All": "Connetti tutto",
        "Check if Online": "Verifica disponibilità",
        "Username copied": "Nome utente copiato",
        "No username configured": "Nessun nome utente configurato",
        "Password copied (auto-clears in 30s)": "Password copiata (cancellazione auto in 30 s)",
        "Cached password is empty": "La password memorizzata è vuota",
        "Connect first to cache credentials": "Connettiti prima per memorizzare le credenziali",
    },
    "pl": {
        "Mouse Jiggler": "Symulator ruchu myszy",
        "Prevent idle disconnect by simulating mouse movement": "Zapobiega rozłączeniu z powodu bezczynności, symulując ruch myszy",
        "Jiggler Interval": "Interwał ruchu",
        "Seconds between mouse movements": "Sekundy między ruchami myszy",
        "Connect All": "Połącz wszystkie",
        "Check if Online": "Sprawdź dostępność",
        "Username copied": "Nazwa użytkownika skopiowana",
        "No username configured": "Nie skonfigurowano nazwy użytkownika",
        "Password copied (auto-clears in 30s)": "Hasło skopiowane (automatyczne czyszczenie po 30 s)",
        "Cached password is empty": "Zapisane hasło jest puste",
        "Connect first to cache credentials": "Najpierw połącz się, aby zapisać dane logowania",
    },
    "cs": {
        "Mouse Jiggler": "Simulátor pohybu myši",
        "Prevent idle disconnect by simulating mouse movement": "Zabraňuje odpojení při nečinnosti simulací pohybu myši",
        "Jiggler Interval": "Interval pohybu",
        "Seconds between mouse movements": "Sekundy mezi pohyby myši",
        "Connect All": "Připojit vše",
        "Check if Online": "Zkontrolovat dostupnost",
        "Username copied": "Uživatelské jméno zkopírováno",
        "No username configured": "Uživatelské jméno není nastaveno",
        "Password copied (auto-clears in 30s)": "Heslo zkopírováno (automatické smazání za 30 s)",
        "Cached password is empty": "Uložené heslo je prázdné",
        "Connect first to cache credentials": "Nejprve se připojte pro uložení přihlašovacích údajů",
    },
    "sk": {
        "Mouse Jiggler": "Simulátor pohybu myši",
        "Prevent idle disconnect by simulating mouse movement": "Zabraňuje odpojeniu pri nečinnosti simuláciou pohybu myši",
        "Jiggler Interval": "Interval pohybu",
        "Seconds between mouse movements": "Sekundy medzi pohybmi myši",
        "Connect All": "Pripojiť všetko",
        "Check if Online": "Skontrolovať dostupnosť",
        "Username copied": "Používateľské meno skopírované",
        "No username configured": "Používateľské meno nie je nastavené",
        "Password copied (auto-clears in 30s)": "Heslo skopírované (automatické vymazanie za 30 s)",
        "Cached password is empty": "Uložené heslo je prázdne",
        "Connect first to cache credentials": "Najprv sa pripojte pre uloženie prihlasovacích údajov",
    },
    "da": {
        "Mouse Jiggler": "Musebevæger",
        "Prevent idle disconnect by simulating mouse movement": "Forhindrer afbrydelse ved inaktivitet ved at simulere musebevægelse",
        "Jiggler Interval": "Bevægelsesinterval",
        "Seconds between mouse movements": "Sekunder mellem musebevægelser",
        "Connect All": "Forbind alle",
        "Check if Online": "Tjek tilgængelighed",
        "Username copied": "Brugernavn kopieret",
        "No username configured": "Intet brugernavn konfigureret",
        "Password copied (auto-clears in 30s)": "Adgangskode kopieret (ryddes automatisk efter 30 s)",
        "Cached password is empty": "Gemt adgangskode er tom",
        "Connect first to cache credentials": "Opret forbindelse først for at gemme legitimationsoplysninger",
    },
    "sv": {
        "Mouse Jiggler": "Musrörare",
        "Prevent idle disconnect by simulating mouse movement": "Förhindrar frånkoppling vid inaktivitet genom att simulera musrörelse",
        "Jiggler Interval": "Rörelseintervall",
        "Seconds between mouse movements": "Sekunder mellan musrörelser",
        "Connect All": "Anslut alla",
        "Check if Online": "Kontrollera tillgänglighet",
        "Username copied": "Användarnamn kopierat",
        "No username configured": "Inget användarnamn konfigurerat",
        "Password copied (auto-clears in 30s)": "Lösenord kopierat (rensas automatiskt efter 30 s)",
        "Cached password is empty": "Sparat lösenord är tomt",
        "Connect first to cache credentials": "Anslut först för att spara inloggningsuppgifter",
    },
    "nl": {
        "Mouse Jiggler": "Muisbeweger",
        "Prevent idle disconnect by simulating mouse movement": "Voorkomt verbreken bij inactiviteit door muisbeweging te simuleren",
        "Jiggler Interval": "Bewegingsinterval",
        "Seconds between mouse movements": "Seconden tussen muisbewegingen",
        "Connect All": "Alles verbinden",
        "Check if Online": "Beschikbaarheid controleren",
        "Username copied": "Gebruikersnaam gekopieerd",
        "No username configured": "Geen gebruikersnaam geconfigureerd",
        "Password copied (auto-clears in 30s)": "Wachtwoord gekopieerd (wordt na 30 s gewist)",
        "Cached password is empty": "Opgeslagen wachtwoord is leeg",
        "Connect first to cache credentials": "Maak eerst verbinding om inloggegevens op te slaan",
    },
    "pt": {
        "Mouse Jiggler": "Simulador de rato",
        "Prevent idle disconnect by simulating mouse movement": "Impede a desconexão por inatividade simulando movimento do rato",
        "Jiggler Interval": "Intervalo de movimento",
        "Seconds between mouse movements": "Segundos entre movimentos do rato",
        "Connect All": "Ligar tudo",
        "Check if Online": "Verificar disponibilidade",
        "Username copied": "Nome de utilizador copiado",
        "No username configured": "Nome de utilizador não configurado",
        "Password copied (auto-clears in 30s)": "Palavra-passe copiada (limpeza automática em 30 s)",
        "Cached password is empty": "A palavra-passe em cache está vazia",
        "Connect first to cache credentials": "Ligue-se primeiro para guardar as credenciais",
    },
    "be": {
        "Mouse Jiggler": "Рух мышы",
        "Prevent idle disconnect by simulating mouse movement": "Прадухіляе адлучэнне праз бяздзейнасць, імітуючы рух мышы",
        "Jiggler Interval": "Інтэрвал руху",
        "Seconds between mouse movements": "Секунды паміж рухамі мышы",
        "Connect All": "Злучыць усё",
        "Check if Online": "Праверыць даступнасць",
        "Username copied": "Імя карыстальніка скапіравана",
        "No username configured": "Імя карыстальніка не наладжана",
        "Password copied (auto-clears in 30s)": "Пароль скапіраваны (аўтаачыстка праз 30 с)",
        "Cached password is empty": "Кэшаваны пароль пусты",
        "Connect first to cache credentials": "Спачатку злучыцеся для кэшавання ўліковых даных",
    },
    "kk": {
        "Mouse Jiggler": "Тінтуір қозғалтқыш",
        "Prevent idle disconnect by simulating mouse movement": "Тінтуір қозғалысын имитациялау арқылы бос тұру кезінде ажыратуды болдырмайды",
        "Jiggler Interval": "Қозғалыс аралығы",
        "Seconds between mouse movements": "Тінтуір қозғалыстары арасындағы секундтар",
        "Connect All": "Барлығын қосу",
        "Check if Online": "Қолжетімділікті тексеру",
        "Username copied": "Пайдаланушы аты көшірілді",
        "No username configured": "Пайдаланушы аты бапталмаған",
        "Password copied (auto-clears in 30s)": "Құпия сөз көшірілді (30 с кейін автоматты тазалау)",
        "Cached password is empty": "Сақталған құпия сөз бос",
        "Connect first to cache credentials": "Тіркелгі деректерін сақтау үшін алдымен қосылыңыз",
    },
    "uz": {
        "Mouse Jiggler": "Sichqoncha harakatlantiruvchi",
        "Prevent idle disconnect by simulating mouse movement": "Sichqoncha harakatini simulyatsiya qilib, bo'sh turish uzilishini oldini oladi",
        "Jiggler Interval": "Harakat oralig'i",
        "Seconds between mouse movements": "Sichqoncha harakatlari orasidagi soniyalar",
        "Connect All": "Hammasini ulash",
        "Check if Online": "Mavjudlikni tekshirish",
        "Username copied": "Foydalanuvchi nomi nusxalandi",
        "No username configured": "Foydalanuvchi nomi sozlanmagan",
        "Password copied (auto-clears in 30s)": "Parol nusxalandi (30 s dan keyin avtomatik tozalanadi)",
        "Cached password is empty": "Saqlangan parol bo'sh",
        "Connect first to cache credentials": "Hisob ma'lumotlarini saqlash uchun avval ulaning",
    },
}


def parse_po_file(filepath):
    """Parse a .po file into a list of entries preserving structure."""
    with open(filepath, 'r', encoding='utf-8') as f:
        content = f.read()

    entries = []
    current_comments = []
    current_msgid_lines = []
    current_msgstr_lines = []
    in_msgid = False
    in_msgstr = False

    for line in content.split('\n'):
        if line.startswith('#'):
            if in_msgstr and current_msgid_lines:
                entries.append({
                    'comments': current_comments,
                    'msgid_lines': current_msgid_lines,
                    'msgstr_lines': current_msgstr_lines,
                })
                current_comments = []
                current_msgid_lines = []
                current_msgstr_lines = []
                in_msgid = False
                in_msgstr = False
            current_comments.append(line)
        elif line.startswith('msgid '):
            if in_msgstr and current_msgid_lines:
                entries.append({
                    'comments': current_comments,
                    'msgid_lines': current_msgid_lines,
                    'msgstr_lines': current_msgstr_lines,
                })
                current_comments = []
                current_msgid_lines = []
                current_msgstr_lines = []
            in_msgid = True
            in_msgstr = False
            current_msgid_lines.append(line)
        elif line.startswith('msgstr '):
            in_msgid = False
            in_msgstr = True
            current_msgstr_lines.append(line)
        elif line.startswith('"') and (in_msgid or in_msgstr):
            if in_msgid:
                current_msgid_lines.append(line)
            else:
                current_msgstr_lines.append(line)
        elif line.strip() == '':
            if in_msgstr and current_msgid_lines:
                entries.append({
                    'comments': current_comments,
                    'msgid_lines': current_msgid_lines,
                    'msgstr_lines': current_msgstr_lines,
                })
                current_comments = []
                current_msgid_lines = []
                current_msgstr_lines = []
                in_msgid = False
                in_msgstr = False

    # Don't forget the last entry
    if current_msgid_lines:
        entries.append({
            'comments': current_comments,
            'msgid_lines': current_msgid_lines,
            'msgstr_lines': current_msgstr_lines,
        })

    return entries


def extract_msgid(msgid_lines):
    """Extract the actual msgid string from msgid lines."""
    parts = []
    for line in msgid_lines:
        if line.startswith('msgid '):
            match = re.match(r'msgid\s+"(.*)"', line)
            if match:
                parts.append(match.group(1))
        elif line.startswith('"'):
            match = re.match(r'"(.*)"', line)
            if match:
                parts.append(match.group(1))
    return ''.join(parts)


def extract_msgstr(msgstr_lines):
    """Extract the actual msgstr string from msgstr lines."""
    parts = []
    for line in msgstr_lines:
        if line.startswith('msgstr '):
            match = re.match(r'msgstr\s+"(.*)"', line)
            if match:
                parts.append(match.group(1))
        elif line.startswith('"'):
            match = re.match(r'"(.*)"', line)
            if match:
                parts.append(match.group(1))
    return ''.join(parts)


def rebuild_po_file(entries):
    """Rebuild .po file content from entries."""
    lines = []
    for i, entry in enumerate(entries):
        if i > 0:
            lines.append('')
        for comment in entry['comments']:
            lines.append(comment)
        for line in entry['msgid_lines']:
            lines.append(line)
        for line in entry['msgstr_lines']:
            lines.append(line)
    lines.append('')  # trailing newline
    return '\n'.join(lines)


def run_msgmerge(po_file, pot_file):
    """Run msgmerge to add new empty entries from the .pot file."""
    try:
        subprocess.run(
            ['msgmerge', '--update', '--no-fuzzy-matching', '--backup=none',
             po_file, pot_file],
            check=True,
            capture_output=True,
            text=True,
        )
    except FileNotFoundError:
        print("WARNING: msgmerge not found, skipping merge step")
        return False
    except subprocess.CalledProcessError as e:
        print(f"WARNING: msgmerge failed for {po_file}: {e.stderr}")
        return False
    return True


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

        # Only fill if msgstr is empty and we have a translation
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
    pot_file = os.path.join(po_dir, 'rustconn.pot')
    languages = [
        'uk', 'de', 'it', 'fr', 'es', 'pl', 'cs', 'sk',
        'da', 'sv', 'nl', 'pt', 'be', 'kk', 'uz',
    ]

    # Step 1: Run msgmerge to add new entries from .pot
    print("Step 1: Running msgmerge to add new entries from .pot...")
    for lang in languages:
        filepath = os.path.join(po_dir, f'{lang}.po')
        if not os.path.exists(filepath):
            print(f"  SKIP: {filepath} not found")
            continue
        if run_msgmerge(filepath, pot_file):
            print(f"  {lang}: msgmerge OK")
        else:
            print(f"  {lang}: msgmerge failed")

    # Step 2: Fill in translations
    print("\nStep 2: Filling translations...")
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
