#!/usr/bin/env python3
"""Fill empty translations in all .po files for RustConn 0.9.4 strings."""

import re
import os
import sys

# Translations for each language keyed by msgid
TRANSLATIONS = {
    "uk": {
        "{} backend unavailable. Using fallback.": "Бекенд {} недоступний. Використовується запасний.",
        "Key file not found": "Файл ключа не знайдено",
        "Conflicts with: {}": "Конфліктує з: {}",
        "Backup & Restore": "Резервне копіювання та відновлення",
        "Export or import all settings as a ZIP archive": "Експортуй або імпортуй усі налаштування як ZIP-архів",
        "Backup Settings…": "Резервне копіювання налаштувань…",
        "Save all configuration files to a ZIP archive": "Збережи всі файли конфігурації в ZIP-архів",
        "Restore Settings…": "Відновлення налаштувань…",
        "Load configuration from a ZIP archive (restart required)": "Завантаж конфігурацію з ZIP-архіву (потрібен перезапуск)",
        "Save Backup": "Збережи резервну копію",
        "Backup saved ({} files)": "Резервну копію збережено ({} файлів)",
        "Backup Error": "Помилка резервного копіювання",
        "Open Backup": "Відкрий резервну копію",
        "Restore Settings?": "Відновити налаштування?",
        "This will overwrite current settings. A restart is required to apply changes.": "Це перезапише поточні налаштування. Для застосування змін потрібен перезапуск.",
        "Restore": "Відновити",
        "Restored {} files. Restart to apply.": "Відновлено {} файлів. Перезапусти для застосування.",
        "Restore Error": "Помилка відновлення",
        "Create Cluster": "Створи кластер",
        "Create a cluster from selected connections": "Створи кластер із вибраних з'єднанків",
        "Reconnect to this session": "Перез'єднайся з цим сеансом",
        "Session disconnected": "Сеанс від'єднано",
        "Warning": "Попередження",
        "Info": "Інформація",
        "Retry": "Повторити",
        "Connection failed. Host unreachable.": "З'єднання не вдалося. Гост недосяжний.",
        "This group contains {} connection(s).": "Ця група містить {} з'єднанків.",
        "Delete group '{}'?": "Вилучити групу «{}»?",
        "Keep Connections": "Зберегти з'єднанки",
    },
    "de": {
        "{} backend unavailable. Using fallback.": "Backend {} nicht verfügbar. Verwende Ersatz.",
        "Key file not found": "Schlüsseldatei nicht gefunden",
        "Conflicts with: {}": "Konflikt mit: {}",
        "Backup & Restore": "Sicherung & Wiederherstellung",
        "Export or import all settings as a ZIP archive": "Alle Einstellungen als ZIP-Archiv exportieren oder importieren",
        "Backup Settings…": "Einstellungen sichern…",
        "Save all configuration files to a ZIP archive": "Alle Konfigurationsdateien in ein ZIP-Archiv speichern",
        "Restore Settings…": "Einstellungen wiederherstellen…",
        "Load configuration from a ZIP archive (restart required)": "Konfiguration aus ZIP-Archiv laden (Neustart erforderlich)",
        "Save Backup": "Sicherung speichern",
        "Backup saved ({} files)": "Sicherung gespeichert ({} Dateien)",
        "Backup Error": "Sicherungsfehler",
        "Open Backup": "Sicherung öffnen",
        "Restore Settings?": "Einstellungen wiederherstellen?",
        "This will overwrite current settings. A restart is required to apply changes.": "Dies überschreibt die aktuellen Einstellungen. Ein Neustart ist erforderlich.",
        "Restore": "Wiederherstellen",
        "Restored {} files. Restart to apply.": "{} Dateien wiederhergestellt. Neustart zum Anwenden.",
        "Restore Error": "Wiederherstellungsfehler",
        "Create Cluster": "Cluster erstellen",
        "Create a cluster from selected connections": "Cluster aus ausgewählten Verbindungen erstellen",
        "Reconnect to this session": "Erneut mit dieser Sitzung verbinden",
        "Session disconnected": "Sitzung getrennt",
        "Warning": "Warnung",
        "Info": "Info",
        "Retry": "Wiederholen",
        "Connection failed. Host unreachable.": "Verbindung fehlgeschlagen. Host nicht erreichbar.",
        "This group contains {} connection(s).": "Diese Gruppe enthält {} Verbindung(en).",
        "Delete group '{}'?": "Gruppe «{}» löschen?",
        "Keep Connections": "Verbindungen behalten",
    },
    "fr": {
        "{} backend unavailable. Using fallback.": "Backend {} indisponible. Utilisation du secours.",
        "Key file not found": "Fichier de clé introuvable",
        "Conflicts with: {}": "En conflit avec : {}",
        "Backup & Restore": "Sauvegarde et restauration",
        "Export or import all settings as a ZIP archive": "Exporter ou importer tous les paramètres en archive ZIP",
        "Backup Settings…": "Sauvegarder les paramètres…",
        "Save all configuration files to a ZIP archive": "Enregistrer tous les fichiers de configuration dans une archive ZIP",
        "Restore Settings…": "Restaurer les paramètres…",
        "Load configuration from a ZIP archive (restart required)": "Charger la configuration depuis une archive ZIP (redémarrage requis)",
        "Save Backup": "Enregistrer la sauvegarde",
        "Backup saved ({} files)": "Sauvegarde enregistrée ({} fichiers)",
        "Backup Error": "Erreur de sauvegarde",
        "Open Backup": "Ouvrir la sauvegarde",
        "Restore Settings?": "Restaurer les paramètres ?",
        "This will overwrite current settings. A restart is required to apply changes.": "Cela écrasera les paramètres actuels. Un redémarrage est nécessaire.",
        "Restore": "Restaurer",
        "Restored {} files. Restart to apply.": "{} fichiers restaurés. Redémarrez pour appliquer.",
        "Restore Error": "Erreur de restauration",
        "Create Cluster": "Créer un cluster",
        "Create a cluster from selected connections": "Créer un cluster à partir des connexions sélectionnées",
        "Reconnect to this session": "Se reconnecter à cette session",
        "Session disconnected": "Session déconnectée",
        "Warning": "Avertissement",
        "Info": "Info",
        "Retry": "Réessayer",
        "Connection failed. Host unreachable.": "Connexion échouée. Hôte injoignable.",
        "This group contains {} connection(s).": "Ce groupe contient {} connexion(s).",
        "Delete group '{}'?": "Supprimer le groupe « {} » ?",
        "Keep Connections": "Conserver les connexions",
    },
    "es": {
        "{} backend unavailable. Using fallback.": "Backend {} no disponible. Usando alternativa.",
        "Key file not found": "Archivo de clave no encontrado",
        "Conflicts with: {}": "Conflicto con: {}",
        "Backup & Restore": "Copia de seguridad y restauración",
        "Export or import all settings as a ZIP archive": "Exportar o importar todos los ajustes como archivo ZIP",
        "Backup Settings…": "Copia de seguridad…",
        "Save all configuration files to a ZIP archive": "Guardar todos los archivos de configuración en un archivo ZIP",
        "Restore Settings…": "Restaurar ajustes…",
        "Load configuration from a ZIP archive (restart required)": "Cargar configuración desde un archivo ZIP (requiere reinicio)",
        "Save Backup": "Guardar copia",
        "Backup saved ({} files)": "Copia guardada ({} archivos)",
        "Backup Error": "Error de copia de seguridad",
        "Open Backup": "Abrir copia",
        "Restore Settings?": "¿Restaurar ajustes?",
        "This will overwrite current settings. A restart is required to apply changes.": "Esto sobrescribirá los ajustes actuales. Se requiere un reinicio para aplicar los cambios.",
        "Restore": "Restaurar",
        "Restored {} files. Restart to apply.": "{} archivos restaurados. Reinicie para aplicar.",
        "Restore Error": "Error de restauración",
        "Create Cluster": "Crear clúster",
        "Create a cluster from selected connections": "Crear un clúster a partir de las conexiones seleccionadas",
        "Reconnect to this session": "Reconectar a esta sesión",
        "Session disconnected": "Sesión desconectada",
        "Warning": "Advertencia",
        "Info": "Información",
        "Retry": "Reintentar",
        "Connection failed. Host unreachable.": "Conexión fallida. Host inaccesible.",
        "This group contains {} connection(s).": "Este grupo contiene {} conexión(es).",
        "Delete group '{}'?": "¿Eliminar grupo «{}»?",
        "Keep Connections": "Conservar conexiones",
    },
    "it": {
        "{} backend unavailable. Using fallback.": "Backend {} non disponibile. Uso del fallback.",
        "Key file not found": "File chiave non trovato",
        "Conflicts with: {}": "In conflitto con: {}",
        "Backup & Restore": "Backup e ripristino",
        "Export or import all settings as a ZIP archive": "Esporta o importa tutte le impostazioni come archivio ZIP",
        "Backup Settings…": "Backup impostazioni…",
        "Save all configuration files to a ZIP archive": "Salva tutti i file di configurazione in un archivio ZIP",
        "Restore Settings…": "Ripristina impostazioni…",
        "Load configuration from a ZIP archive (restart required)": "Carica configurazione da un archivio ZIP (riavvio necessario)",
        "Save Backup": "Salva backup",
        "Backup saved ({} files)": "Backup salvato ({} file)",
        "Backup Error": "Errore di backup",
        "Open Backup": "Apri backup",
        "Restore Settings?": "Ripristinare le impostazioni?",
        "This will overwrite current settings. A restart is required to apply changes.": "Questo sovrascriverà le impostazioni attuali. È necessario un riavvio per applicare le modifiche.",
        "Restore": "Ripristina",
        "Restored {} files. Restart to apply.": "{} file ripristinati. Riavvia per applicare.",
        "Restore Error": "Errore di ripristino",
        "Create Cluster": "Crea cluster",
        "Create a cluster from selected connections": "Crea un cluster dalle connessioni selezionate",
        "Reconnect to this session": "Riconnetti a questa sessione",
        "Session disconnected": "Sessione disconnessa",
        "Warning": "Avviso",
        "Info": "Info",
        "Retry": "Riprova",
        "Connection failed. Host unreachable.": "Connessione fallita. Host irraggiungibile.",
        "This group contains {} connection(s).": "Questo gruppo contiene {} connessione/i.",
        "Delete group '{}'?": "Eliminare il gruppo «{}»?",
        "Keep Connections": "Mantieni connessioni",
    },
    "pl": {
        "{} backend unavailable. Using fallback.": "Backend {} niedostępny. Używam zapasowego.",
        "Key file not found": "Nie znaleziono pliku klucza",
        "Conflicts with: {}": "Konflikt z: {}",
        "Backup & Restore": "Kopia zapasowa i przywracanie",
        "Export or import all settings as a ZIP archive": "Eksportuj lub importuj wszystkie ustawienia jako archiwum ZIP",
        "Backup Settings…": "Kopia zapasowa ustawień…",
        "Save all configuration files to a ZIP archive": "Zapisz wszystkie pliki konfiguracji do archiwum ZIP",
        "Restore Settings…": "Przywróć ustawienia…",
        "Load configuration from a ZIP archive (restart required)": "Wczytaj konfigurację z archiwum ZIP (wymagany restart)",
        "Save Backup": "Zapisz kopię",
        "Backup saved ({} files)": "Kopia zapisana ({} plików)",
        "Backup Error": "Błąd kopii zapasowej",
        "Open Backup": "Otwórz kopię",
        "Restore Settings?": "Przywrócić ustawienia?",
        "This will overwrite current settings. A restart is required to apply changes.": "To nadpisze bieżące ustawienia. Wymagany jest restart, aby zastosować zmiany.",
        "Restore": "Przywróć",
        "Restored {} files. Restart to apply.": "Przywrócono {} plików. Uruchom ponownie, aby zastosować.",
        "Restore Error": "Błąd przywracania",
        "Create Cluster": "Utwórz klaster",
        "Create a cluster from selected connections": "Utwórz klaster z wybranych połączeń",
        "Reconnect to this session": "Połącz ponownie z tą sesją",
        "Session disconnected": "Sesja rozłączona",
        "Warning": "Ostrzeżenie",
        "Info": "Informacja",
        "Retry": "Ponów",
        "Connection failed. Host unreachable.": "Połączenie nieudane. Host nieosiągalny.",
        "This group contains {} connection(s).": "Ta grupa zawiera {} połączenie/ń.",
        "Delete group '{}'?": "Usunąć grupę «{}»?",
        "Keep Connections": "Zachowaj połączenia",
    },
    "cs": {
        "{} backend unavailable. Using fallback.": "Backend {} nedostupný. Používám záložní.",
        "Key file not found": "Soubor klíče nenalezen",
        "Conflicts with: {}": "Konflikt s: {}",
        "Backup & Restore": "Záloha a obnovení",
        "Export or import all settings as a ZIP archive": "Exportovat nebo importovat všechna nastavení jako ZIP archiv",
        "Backup Settings…": "Zálohovat nastavení…",
        "Save all configuration files to a ZIP archive": "Uložit všechny konfigurační soubory do ZIP archivu",
        "Restore Settings…": "Obnovit nastavení…",
        "Load configuration from a ZIP archive (restart required)": "Načíst konfiguraci ze ZIP archivu (vyžadován restart)",
        "Save Backup": "Uložit zálohu",
        "Backup saved ({} files)": "Záloha uložena ({} souborů)",
        "Backup Error": "Chyba zálohy",
        "Open Backup": "Otevřít zálohu",
        "Restore Settings?": "Obnovit nastavení?",
        "This will overwrite current settings. A restart is required to apply changes.": "Toto přepíše aktuální nastavení. Pro použití změn je nutný restart.",
        "Restore": "Obnovit",
        "Restored {} files. Restart to apply.": "Obnoveno {} souborů. Restartujte pro použití.",
        "Restore Error": "Chyba obnovení",
        "Create Cluster": "Vytvořit cluster",
        "Create a cluster from selected connections": "Vytvořit cluster z vybraných připojení",
        "Reconnect to this session": "Znovu připojit k této relaci",
        "Session disconnected": "Relace odpojena",
        "Warning": "Varování",
        "Info": "Informace",
        "Retry": "Opakovat",
        "Connection failed. Host unreachable.": "Připojení selhalo. Hostitel nedostupný.",
        "This group contains {} connection(s).": "Tato skupina obsahuje {} připojení.",
        "Delete group '{}'?": "Smazat skupinu «{}»?",
        "Keep Connections": "Ponechat připojení",
    },
    "sk": {
        "{} backend unavailable. Using fallback.": "Backend {} nedostupný. Používam záložný.",
        "Key file not found": "Súbor kľúča nenájdený",
        "Conflicts with: {}": "Konflikt s: {}",
        "Backup & Restore": "Záloha a obnovenie",
        "Export or import all settings as a ZIP archive": "Exportovať alebo importovať všetky nastavenia ako ZIP archív",
        "Backup Settings…": "Zálohovať nastavenia…",
        "Save all configuration files to a ZIP archive": "Uložiť všetky konfiguračné súbory do ZIP archívu",
        "Restore Settings…": "Obnoviť nastavenia…",
        "Load configuration from a ZIP archive (restart required)": "Načítať konfiguráciu zo ZIP archívu (vyžadovaný reštart)",
        "Save Backup": "Uložiť zálohu",
        "Backup saved ({} files)": "Záloha uložená ({} súborov)",
        "Backup Error": "Chyba zálohy",
        "Open Backup": "Otvoriť zálohu",
        "Restore Settings?": "Obnoviť nastavenia?",
        "This will overwrite current settings. A restart is required to apply changes.": "Toto prepíše aktuálne nastavenia. Na použitie zmien je potrebný reštart.",
        "Restore": "Obnoviť",
        "Restored {} files. Restart to apply.": "Obnovených {} súborov. Reštartujte na použitie.",
        "Restore Error": "Chyba obnovenia",
        "Create Cluster": "Vytvoriť cluster",
        "Create a cluster from selected connections": "Vytvoriť cluster z vybraných pripojení",
        "Reconnect to this session": "Znovu pripojiť k tejto relácii",
        "Session disconnected": "Relácia odpojená",
        "Warning": "Varovanie",
        "Info": "Informácia",
        "Retry": "Opakovať",
        "Connection failed. Host unreachable.": "Pripojenie zlyhalo. Hostiteľ nedostupný.",
        "This group contains {} connection(s).": "Táto skupina obsahuje {} pripojení.",
        "Delete group '{}'?": "Odstrániť skupinu «{}»?",
        "Keep Connections": "Ponechať pripojenia",
    },
    "da": {
        "{} backend unavailable. Using fallback.": "Backend {} utilgængelig. Bruger reserve.",
        "Key file not found": "Nøglefil ikke fundet",
        "Conflicts with: {}": "Konflikt med: {}",
        "Backup & Restore": "Sikkerhedskopiering og gendannelse",
        "Export or import all settings as a ZIP archive": "Eksportér eller importér alle indstillinger som ZIP-arkiv",
        "Backup Settings…": "Sikkerhedskopiér indstillinger…",
        "Save all configuration files to a ZIP archive": "Gem alle konfigurationsfiler i et ZIP-arkiv",
        "Restore Settings…": "Gendan indstillinger…",
        "Load configuration from a ZIP archive (restart required)": "Indlæs konfiguration fra et ZIP-arkiv (genstart påkrævet)",
        "Save Backup": "Gem sikkerhedskopi",
        "Backup saved ({} files)": "Sikkerhedskopi gemt ({} filer)",
        "Backup Error": "Sikkerhedskopieringsfejl",
        "Open Backup": "Åbn sikkerhedskopi",
        "Restore Settings?": "Gendan indstillinger?",
        "This will overwrite current settings. A restart is required to apply changes.": "Dette overskriver de aktuelle indstillinger. En genstart er påkrævet.",
        "Restore": "Gendan",
        "Restored {} files. Restart to apply.": "{} filer gendannet. Genstart for at anvende.",
        "Restore Error": "Gendannelsesfejl",
        "Create Cluster": "Opret klynge",
        "Create a cluster from selected connections": "Opret en klynge fra valgte forbindelser",
        "Reconnect to this session": "Genopret forbindelse til denne session",
        "Session disconnected": "Session afbrudt",
        "Warning": "Advarsel",
        "Info": "Info",
        "Retry": "Prøv igen",
        "Connection failed. Host unreachable.": "Forbindelse mislykkedes. Vært utilgængelig.",
        "This group contains {} connection(s).": "Denne gruppe indeholder {} forbindelse(r).",
        "Delete group '{}'?": "Slet gruppen «{}»?",
        "Keep Connections": "Behold forbindelser",
    },
    "sv": {
        "{} backend unavailable. Using fallback.": "Backend {} otillgänglig. Använder reserv.",
        "Key file not found": "Nyckelfil hittades inte",
        "Conflicts with: {}": "Konflikt med: {}",
        "Backup & Restore": "Säkerhetskopiering och återställning",
        "Export or import all settings as a ZIP archive": "Exportera eller importera alla inställningar som ZIP-arkiv",
        "Backup Settings…": "Säkerhetskopiera inställningar…",
        "Save all configuration files to a ZIP archive": "Spara alla konfigurationsfiler i ett ZIP-arkiv",
        "Restore Settings…": "Återställ inställningar…",
        "Load configuration from a ZIP archive (restart required)": "Läs in konfiguration från ett ZIP-arkiv (omstart krävs)",
        "Save Backup": "Spara säkerhetskopia",
        "Backup saved ({} files)": "Säkerhetskopia sparad ({} filer)",
        "Backup Error": "Säkerhetskopieringsfel",
        "Open Backup": "Öppna säkerhetskopia",
        "Restore Settings?": "Återställa inställningar?",
        "This will overwrite current settings. A restart is required to apply changes.": "Detta skriver över aktuella inställningar. En omstart krävs för att tillämpa ändringar.",
        "Restore": "Återställ",
        "Restored {} files. Restart to apply.": "{} filer återställda. Starta om för att tillämpa.",
        "Restore Error": "Återställningsfel",
        "Create Cluster": "Skapa kluster",
        "Create a cluster from selected connections": "Skapa ett kluster från valda anslutningar",
        "Reconnect to this session": "Återanslut till denna session",
        "Session disconnected": "Session frånkopplad",
        "Warning": "Varning",
        "Info": "Info",
        "Retry": "Försök igen",
        "Connection failed. Host unreachable.": "Anslutning misslyckades. Värd onåbar.",
        "This group contains {} connection(s).": "Denna grupp innehåller {} anslutning(ar).",
        "Delete group '{}'?": "Ta bort gruppen «{}»?",
        "Keep Connections": "Behåll anslutningar",
    },
    "nl": {
        "{} backend unavailable. Using fallback.": "Backend {} niet beschikbaar. Terugvaloptie wordt gebruikt.",
        "Key file not found": "Sleutelbestand niet gevonden",
        "Conflicts with: {}": "Conflict met: {}",
        "Backup & Restore": "Back-up en herstel",
        "Export or import all settings as a ZIP archive": "Exporteer of importeer alle instellingen als ZIP-archief",
        "Backup Settings…": "Instellingen back-uppen…",
        "Save all configuration files to a ZIP archive": "Sla alle configuratiebestanden op in een ZIP-archief",
        "Restore Settings…": "Instellingen herstellen…",
        "Load configuration from a ZIP archive (restart required)": "Configuratie laden uit een ZIP-archief (herstart vereist)",
        "Save Backup": "Back-up opslaan",
        "Backup saved ({} files)": "Back-up opgeslagen ({} bestanden)",
        "Backup Error": "Back-upfout",
        "Open Backup": "Back-up openen",
        "Restore Settings?": "Instellingen herstellen?",
        "This will overwrite current settings. A restart is required to apply changes.": "Dit overschrijft de huidige instellingen. Een herstart is vereist om wijzigingen toe te passen.",
        "Restore": "Herstellen",
        "Restored {} files. Restart to apply.": "{} bestanden hersteld. Herstart om toe te passen.",
        "Restore Error": "Herstelfout",
        "Create Cluster": "Cluster aanmaken",
        "Create a cluster from selected connections": "Maak een cluster aan van geselecteerde verbindingen",
        "Reconnect to this session": "Opnieuw verbinden met deze sessie",
        "Session disconnected": "Sessie verbroken",
        "Warning": "Waarschuwing",
        "Info": "Info",
        "Retry": "Opnieuw proberen",
        "Connection failed. Host unreachable.": "Verbinding mislukt. Host onbereikbaar.",
        "This group contains {} connection(s).": "Deze groep bevat {} verbinding(en).",
        "Delete group '{}'?": "Groep «{}» verwijderen?",
        "Keep Connections": "Verbindingen behouden",
    },
    "pt": {
        "{} backend unavailable. Using fallback.": "Backend {} indisponível. Usando alternativa.",
        "Key file not found": "Ficheiro de chave não encontrado",
        "Conflicts with: {}": "Conflito com: {}",
        "Backup & Restore": "Cópia de segurança e restauro",
        "Export or import all settings as a ZIP archive": "Exportar ou importar todas as definições como arquivo ZIP",
        "Backup Settings…": "Cópia de segurança…",
        "Save all configuration files to a ZIP archive": "Guardar todos os ficheiros de configuração num arquivo ZIP",
        "Restore Settings…": "Restaurar definições…",
        "Load configuration from a ZIP archive (restart required)": "Carregar configuração de um arquivo ZIP (reinício necessário)",
        "Save Backup": "Guardar cópia",
        "Backup saved ({} files)": "Cópia guardada ({} ficheiros)",
        "Backup Error": "Erro de cópia de segurança",
        "Open Backup": "Abrir cópia",
        "Restore Settings?": "Restaurar definições?",
        "This will overwrite current settings. A restart is required to apply changes.": "Isto irá substituir as definições atuais. É necessário reiniciar para aplicar as alterações.",
        "Restore": "Restaurar",
        "Restored {} files. Restart to apply.": "{} ficheiros restaurados. Reinicie para aplicar.",
        "Restore Error": "Erro de restauro",
        "Create Cluster": "Criar cluster",
        "Create a cluster from selected connections": "Criar um cluster a partir das ligações selecionadas",
        "Reconnect to this session": "Reconectar a esta sessão",
        "Session disconnected": "Sessão desligada",
        "Warning": "Aviso",
        "Info": "Informação",
        "Retry": "Tentar novamente",
        "Connection failed. Host unreachable.": "Ligação falhada. Anfitrião inacessível.",
        "This group contains {} connection(s).": "Este grupo contém {} ligação(ões).",
        "Delete group '{}'?": "Eliminar grupo «{}»?",
        "Keep Connections": "Manter ligações",
    },
    "be": {
        "{} backend unavailable. Using fallback.": "Бэкенд {} недаступны. Выкарыстоўваецца запасны.",
        "Key file not found": "Файл ключа не знойдзены",
        "Conflicts with: {}": "Канфлікт з: {}",
        "Backup & Restore": "Рэзервовае капіраванне і аднаўленне",
        "Export or import all settings as a ZIP archive": "Экспартаваць або імпартаваць усе налады як ZIP-архіў",
        "Backup Settings…": "Рэзервовае капіраванне налад…",
        "Save all configuration files to a ZIP archive": "Захаваць усе файлы канфігурацыі ў ZIP-архіў",
        "Restore Settings…": "Аднавіць налады…",
        "Load configuration from a ZIP archive (restart required)": "Загрузіць канфігурацыю з ZIP-архіва (патрабуецца перазапуск)",
        "Save Backup": "Захаваць рэзервовую копію",
        "Backup saved ({} files)": "Рэзервовую копію захавана ({} файлаў)",
        "Backup Error": "Памылка рэзервовага капіравання",
        "Open Backup": "Адкрыць рэзервовую копію",
        "Restore Settings?": "Аднавіць налады?",
        "This will overwrite current settings. A restart is required to apply changes.": "Гэта перазапіша бягучыя налады. Для прымянення змен патрабуецца перазапуск.",
        "Restore": "Аднавіць",
        "Restored {} files. Restart to apply.": "Адноўлена {} файлаў. Перазапусціце для прымянення.",
        "Restore Error": "Памылка аднаўлення",
        "Create Cluster": "Стварыць кластар",
        "Create a cluster from selected connections": "Стварыць кластар з абраных злучэнняў",
        "Reconnect to this session": "Перазлучыцца з гэтым сеансам",
        "Session disconnected": "Сеанс адлучаны",
        "Warning": "Папярэджанне",
        "Info": "Інфармацыя",
        "Retry": "Паўтарыць",
        "Connection failed. Host unreachable.": "Злучэнне не ўдалося. Хост недасяжны.",
        "This group contains {} connection(s).": "Гэтая група змяшчае {} злучэнняў.",
        "Delete group '{}'?": "Выдаліць групу «{}»?",
        "Keep Connections": "Захаваць злучэнні",
    },
    "kk": {
        "{} backend unavailable. Using fallback.": "{} бэкенді қолжетімсіз. Қосалқы пайдаланылуда.",
        "Key file not found": "Кілт файлы табылмады",
        "Conflicts with: {}": "Қайшылық: {}",
        "Backup & Restore": "Сақтық көшірме және қалпына келтіру",
        "Export or import all settings as a ZIP archive": "Барлық параметрлерді ZIP мұрағаты ретінде экспорттау немесе импорттау",
        "Backup Settings…": "Параметрлерді сақтық көшірмелеу…",
        "Save all configuration files to a ZIP archive": "Барлық конфигурация файлдарын ZIP мұрағатына сақтау",
        "Restore Settings…": "Параметрлерді қалпына келтіру…",
        "Load configuration from a ZIP archive (restart required)": "ZIP мұрағатынан конфигурацияны жүктеу (қайта іске қосу қажет)",
        "Save Backup": "Сақтық көшірмені сақтау",
        "Backup saved ({} files)": "Сақтық көшірме сақталды ({} файл)",
        "Backup Error": "Сақтық көшірме қатесі",
        "Open Backup": "Сақтық көшірмені ашу",
        "Restore Settings?": "Параметрлерді қалпына келтіру керек пе?",
        "This will overwrite current settings. A restart is required to apply changes.": "Бұл ағымдағы параметрлерді қайта жазады. Өзгерістерді қолдану үшін қайта іске қосу қажет.",
        "Restore": "Қалпына келтіру",
        "Restored {} files. Restart to apply.": "{} файл қалпына келтірілді. Қолдану үшін қайта іске қосыңыз.",
        "Restore Error": "Қалпына келтіру қатесі",
        "Create Cluster": "Кластер құру",
        "Create a cluster from selected connections": "Таңдалған қосылымдардан кластер құру",
        "Reconnect to this session": "Бұл сеансқа қайта қосылу",
        "Session disconnected": "Сеанс ажыратылды",
        "Warning": "Ескерту",
        "Info": "Ақпарат",
        "Retry": "Қайталау",
        "Connection failed. Host unreachable.": "Қосылу сәтсіз. Хост қолжетімсіз.",
        "This group contains {} connection(s).": "Бұл топта {} қосылым бар.",
        "Delete group '{}'?": "«{}» тобын жою керек пе?",
        "Keep Connections": "Қосылымдарды сақтау",
    },
    "uz": {
        "{} backend unavailable. Using fallback.": "{} backend mavjud emas. Zaxira ishlatilmoqda.",
        "Key file not found": "Kalit fayli topilmadi",
        "Conflicts with: {}": "Ziddiyat: {}",
        "Backup & Restore": "Zaxira nusxa va tiklash",
        "Export or import all settings as a ZIP archive": "Barcha sozlamalarni ZIP arxiv sifatida eksport yoki import qilish",
        "Backup Settings…": "Sozlamalarni zaxiralash…",
        "Save all configuration files to a ZIP archive": "Barcha konfiguratsiya fayllarini ZIP arxivga saqlash",
        "Restore Settings…": "Sozlamalarni tiklash…",
        "Load configuration from a ZIP archive (restart required)": "ZIP arxivdan konfiguratsiyani yuklash (qayta ishga tushirish kerak)",
        "Save Backup": "Zaxirani saqlash",
        "Backup saved ({} files)": "Zaxira saqlandi ({} fayl)",
        "Backup Error": "Zaxira xatosi",
        "Open Backup": "Zaxirani ochish",
        "Restore Settings?": "Sozlamalarni tiklashmi?",
        "This will overwrite current settings. A restart is required to apply changes.": "Bu joriy sozlamalarni qayta yozadi. O'zgarishlarni qo'llash uchun qayta ishga tushirish kerak.",
        "Restore": "Tiklash",
        "Restored {} files. Restart to apply.": "{} fayl tiklandi. Qo'llash uchun qayta ishga tushiring.",
        "Restore Error": "Tiklash xatosi",
        "Create Cluster": "Klaster yaratish",
        "Create a cluster from selected connections": "Tanlangan ulanishlardan klaster yaratish",
        "Reconnect to this session": "Bu seansga qayta ulanish",
        "Session disconnected": "Seans uzildi",
        "Warning": "Ogohlantirish",
        "Info": "Ma'lumot",
        "Retry": "Qayta urinish",
        "Connection failed. Host unreachable.": "Ulanish muvaffaqiyatsiz. Xost mavjud emas.",
        "This group contains {} connection(s).": "Bu guruhda {} ta ulanish bor.",
        "Delete group '{}'?": "«{}» guruhini o'chirishmi?",
        "Keep Connections": "Ulanishlarni saqlash",
    },
}


def parse_po_file(filepath):
    """Parse a .po file into a list of entries preserving structure."""
    with open(filepath, 'r', encoding='utf-8') as f:
        content = f.read()

    # Split into entries by double newline (entries are separated by blank lines)
    # But we need to be smarter - parse line by line
    entries = []
    current_comments = []
    current_msgid_lines = []
    current_msgstr_lines = []
    in_msgid = False
    in_msgstr = False

    for line in content.split('\n'):
        if line.startswith('#'):
            if in_msgstr and current_msgid_lines:
                # Save previous entry
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
            # Extract string after msgid
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
