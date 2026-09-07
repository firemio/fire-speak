//! UI language resolution and embedded backend-side translations
//! (tray labels + first-run default modes) for the 12 supported languages.

use crate::settings::Mode;

/// The closed set of supported UI language codes (SPEC v0.2).
pub const SUPPORTED: [&str; 12] = [
    "ja", "en", "zh-CN", "zh-TW", "ko", "es", "fr", "de", "pt-BR", "ru", "vi", "id",
];

/// Resolve the UI language from the OS locale.
/// Always returns one of the 12 supported codes.
pub fn resolve_ui_lang() -> String {
    map_locale(&sys_locale::get_locale().unwrap_or_default())
}

/// Resolve a persisted `ui_lang` setting to a concrete code:
/// a valid stored code wins; `""` (auto) or an unknown code falls back to the
/// OS locale.
pub fn resolve_ui_lang_setting(stored: &str) -> String {
    let stored = stored.trim();
    if SUPPORTED.iter().any(|c| *c == stored) {
        return stored.to_string();
    }
    resolve_ui_lang()
}

/// Map a raw OS locale string (e.g. "ja-JP", "zh_Hant_TW") to a supported code.
/// Rules (SPEC): exact match -> zh-Hant*/zh-TW/zh-HK => zh-TW, zh* => zh-CN,
/// otherwise first-2-letter match, otherwise "en".
pub fn map_locale(raw: &str) -> String {
    let norm = raw.trim().replace('_', "-");
    for code in SUPPORTED {
        if code.eq_ignore_ascii_case(&norm) {
            return code.to_string();
        }
    }
    let lower = norm.to_lowercase();
    if lower.starts_with("zh") {
        if lower.contains("hant") || lower.starts_with("zh-tw") || lower.starts_with("zh-hk") {
            return "zh-TW".to_string();
        }
        return "zh-CN".to_string();
    }
    if let Some(prefix) = lower.get(..2) {
        for code in SUPPORTED {
            if code[..2].eq_ignore_ascii_case(prefix) {
                return code.to_string();
            }
        }
    }
    "en".to_string()
}

/// Fixed tray menu labels: (open settings, quit).
pub fn tray_labels(lang: &str) -> (&'static str, &'static str) {
    match lang {
        "ja" => ("設定を開く", "終了"),
        "zh-CN" => ("打开设置", "退出"),
        "zh-TW" => ("開啟設定", "結束"),
        "ko" => ("설정 열기", "종료"),
        "es" => ("Abrir configuración", "Salir"),
        "fr" => ("Ouvrir les paramètres", "Quitter"),
        "de" => ("Einstellungen öffnen", "Beenden"),
        "pt-BR" => ("Abrir configurações", "Sair"),
        "ru" => ("Открыть настройки", "Выход"),
        "vi" => ("Mở cài đặt", "Thoát"),
        "id" => ("Buka Pengaturan", "Keluar"),
        _ => ("Open Settings", "Quit"),
    }
}

/// Build the 6 default modes (polish, raw, to_en, to_ja, terminal, business)
/// with name and instruction in `lang`. Used only at first run.
pub fn default_modes_for(lang: &str) -> Vec<Mode> {
    const IDS: [&str; 6] = ["polish", "raw", "to_en", "to_ja", "terminal", "business"];
    const USE_LLM: [bool; 6] = [true, false, true, true, true, true];
    let texts = mode_texts(lang);
    IDS.iter()
        .zip(texts.iter())
        .zip(USE_LLM.iter())
        .map(|((id, (name, instruction)), use_llm)| Mode {
            id: (*id).to_string(),
            name: (*name).to_string(),
            instruction: (*instruction).to_string(),
            use_llm: *use_llm,
        })
        .collect()
}

/// (name, instruction) pairs in the fixed order
/// [polish, raw, to_en, to_ja, terminal, business].
fn mode_texts(lang: &str) -> [(&'static str, &'static str); 6] {
    match lang {
        "ja" => [
            ("整形", "フィラー(「えー」「あの」「um」等)と言い直しを除去し、句読点・改行を整えて自然な文章にしてください。内容・意味は変えないでください。話者が使った言語のまま出力してください。"),
            ("そのまま", ""),
            ("英語に翻訳", "内容を自然で流暢な英語に翻訳してください。フィラーは除去してください。"),
            ("日本語に翻訳", "内容を自然な日本語に翻訳してください。フィラーは除去してください。"),
            ("ターミナルコマンド", "発話内容を Windows PowerShell で実行可能なコマンドに変換してください。コマンドのみを出力し、説明やコードフェンスは付けないでください。"),
            ("ビジネス文体", "内容を丁寧なビジネス日本語(です・ます調)に書き直してください。フィラーは除去し、簡潔で礼儀正しい文章にしてください。"),
        ],
        "zh-CN" => [
            ("润色", "去除填充词(如“呃”“那个”“um”等)和口误重述,整理标点和换行,使其成为自然的文章。不要改变内容和含义。请使用说话者所用的语言输出。"),
            ("原样", ""),
            ("翻译成英语", "请将内容翻译成自然流畅的英语。去除填充词。"),
            ("翻译成日语", "请将内容翻译成自然的日语。去除填充词。"),
            ("终端命令", "请将说话内容转换为可在 Windows PowerShell 中执行的命令。只输出命令,不要附加说明或代码围栏。"),
            ("商务文体", "请将内容改写为礼貌、正式的商务中文。去除填充词,使文章简洁而有礼。"),
        ],
        "zh-TW" => [
            ("潤飾", "去除填充詞(如「呃」「那個」「um」等)和重述,整理標點與換行,使其成為自然的文章。請勿改變內容與含義。請以說話者使用的語言輸出。"),
            ("原樣", ""),
            ("翻譯成英文", "請將內容翻譯成自然流暢的英文。去除填充詞。"),
            ("翻譯成日文", "請將內容翻譯成自然的日文。去除填充詞。"),
            ("終端機命令", "請將說話內容轉換為可在 Windows PowerShell 執行的命令。只輸出命令,不要附加說明或程式碼圍欄。"),
            ("商務文體", "請將內容改寫為有禮、正式的商務中文。去除填充詞,使文章簡潔且有禮貌。"),
        ],
        "ko" => [
            ("다듬기", "필러(\"어\", \"음\", \"um\" 등)와 말 고침을 제거하고, 구두점과 줄바꿈을 정리하여 자연스러운 문장으로 만들어 주세요. 내용과 의미는 바꾸지 마세요. 화자가 사용한 언어 그대로 출력해 주세요."),
            ("그대로", ""),
            ("영어로 번역", "내용을 자연스럽고 유창한 영어로 번역해 주세요. 필러는 제거해 주세요."),
            ("일본어로 번역", "내용을 자연스러운 일본어로 번역해 주세요. 필러는 제거해 주세요."),
            ("터미널 명령", "발화 내용을 Windows PowerShell에서 실행 가능한 명령으로 변환해 주세요. 명령만 출력하고 설명이나 코드 펜스는 붙이지 마세요."),
            ("비즈니스 문체", "내용을 정중한 비즈니스 한국어(격식체)로 다시 써 주세요. 필러는 제거하고 간결하고 예의 바른 문장으로 만들어 주세요."),
        ],
        "es" => [
            ("Pulir", "Elimina las muletillas («eh», «este», «um», etc.) y las reformulaciones, corrige la puntuación y los saltos de línea y convierte el texto en prosa natural. No cambies el contenido ni el significado. Mantén el idioma que usó el hablante."),
            ("Tal cual", ""),
            ("Traducir al inglés", "Traduce el contenido a un inglés natural y fluido. Elimina las muletillas."),
            ("Traducir al japonés", "Traduce el contenido a un japonés natural. Elimina las muletillas."),
            ("Comando de terminal", "Convierte lo dicho en un comando ejecutable en Windows PowerShell. Escribe solo el comando, sin explicaciones ni bloques de código."),
            ("Estilo profesional", "Reescribe el contenido en un español profesional y cortés. Elimina las muletillas y redacta de forma concisa y educada."),
        ],
        "fr" => [
            ("Peaufiner", "Supprimez les mots de remplissage (« euh », « ben », « um », etc.) et les reformulations, corrigez la ponctuation et les retours à la ligne pour obtenir un texte naturel. Ne changez ni le contenu ni le sens. Conservez la langue utilisée par le locuteur."),
            ("Tel quel", ""),
            ("Traduire en anglais", "Traduisez le contenu en anglais naturel et fluide. Supprimez les mots de remplissage."),
            ("Traduire en japonais", "Traduisez le contenu en japonais naturel. Supprimez les mots de remplissage."),
            ("Commande de terminal", "Convertissez le propos en une commande exécutable dans Windows PowerShell. N'affichez que la commande, sans explication ni bloc de code."),
            ("Style professionnel", "Réécrivez le contenu dans un français professionnel et courtois. Supprimez les mots de remplissage et rédigez de manière concise et polie."),
        ],
        "de" => [
            ("Überarbeiten", "Entferne Füllwörter („äh“, „ähm“, „um“ usw.) und Selbstkorrekturen, bringe Zeichensetzung und Zeilenumbrüche in Ordnung und forme einen natürlichen Text. Ändere weder Inhalt noch Bedeutung. Gib den Text in der Sprache des Sprechers aus."),
            ("Unverändert", ""),
            ("Ins Englische übersetzen", "Übersetze den Inhalt in natürliches, flüssiges Englisch. Entferne Füllwörter."),
            ("Ins Japanische übersetzen", "Übersetze den Inhalt in natürliches Japanisch. Entferne Füllwörter."),
            ("Terminalbefehl", "Wandle das Gesagte in einen in Windows PowerShell ausführbaren Befehl um. Gib nur den Befehl aus, ohne Erklärungen oder Codeblöcke."),
            ("Geschäftsstil", "Formuliere den Inhalt in höflichem, professionellem Geschäftsdeutsch um. Entferne Füllwörter und schreibe knapp und höflich."),
        ],
        "pt-BR" => [
            ("Aprimorar", "Remova vícios de linguagem («é», «tipo», «um», etc.) e reformulações, ajuste a pontuação e as quebras de linha e transforme o texto em prosa natural. Não altere o conteúdo nem o significado. Mantenha o idioma usado pelo falante."),
            ("Como está", ""),
            ("Traduzir para o inglês", "Traduza o conteúdo para um inglês natural e fluente. Remova os vícios de linguagem."),
            ("Traduzir para o japonês", "Traduza o conteúdo para um japonês natural. Remova os vícios de linguagem."),
            ("Comando de terminal", "Converta a fala em um comando executável no Windows PowerShell. Escreva apenas o comando, sem explicações nem blocos de código."),
            ("Estilo profissional", "Reescreva o conteúdo em um português profissional e cortês. Remova os vícios de linguagem e escreva de forma concisa e educada."),
        ],
        "ru" => [
            ("Отшлифовать", "Удалите слова-паразиты («э-э», «ну», «um» и т. п.) и самоисправления, приведите в порядок пунктуацию и переносы строк, чтобы получился естественный текст. Не меняйте содержание и смысл. Выводите текст на языке говорящего."),
            ("Как есть", ""),
            ("Перевести на английский", "Переведите содержание на естественный, беглый английский язык. Удалите слова-паразиты."),
            ("Перевести на японский", "Переведите содержание на естественный японский язык. Удалите слова-паразиты."),
            ("Команда терминала", "Преобразуйте сказанное в команду, выполнимую в Windows PowerShell. Выведите только команду, без пояснений и блоков кода."),
            ("Деловой стиль", "Перепишите содержание в вежливом деловом русском стиле. Удалите слова-паразиты, пишите кратко и учтиво."),
        ],
        "vi" => [
            ("Trau chuốt", "Loại bỏ các từ đệm («ờ», «à», «um», v.v.) và những câu nói lại, chỉnh sửa dấu câu và xuống dòng để tạo thành văn bản tự nhiên. Không thay đổi nội dung và ý nghĩa. Giữ nguyên ngôn ngữ mà người nói đã dùng."),
            ("Giữ nguyên", ""),
            ("Dịch sang tiếng Anh", "Hãy dịch nội dung sang tiếng Anh tự nhiên, trôi chảy. Loại bỏ các từ đệm."),
            ("Dịch sang tiếng Nhật", "Hãy dịch nội dung sang tiếng Nhật tự nhiên. Loại bỏ các từ đệm."),
            ("Lệnh terminal", "Hãy chuyển nội dung nói thành lệnh có thể chạy trong Windows PowerShell. Chỉ xuất lệnh, không kèm giải thích hay khối mã."),
            ("Văn phong công việc", "Hãy viết lại nội dung bằng tiếng Việt trang trọng, lịch sự trong công việc. Loại bỏ các từ đệm, viết ngắn gọn và nhã nhặn."),
        ],
        "id" => [
            ("Rapikan", "Hapus kata pengisi («eh», «anu», «um», dll.) dan pengulangan, rapikan tanda baca dan pemisah baris agar menjadi teks yang alami. Jangan mengubah isi dan maknanya. Keluarkan teks dalam bahasa yang digunakan pembicara."),
            ("Apa adanya", ""),
            ("Terjemahkan ke bahasa Inggris", "Terjemahkan isi ke dalam bahasa Inggris yang alami dan lancar. Hapus kata pengisi."),
            ("Terjemahkan ke bahasa Jepang", "Terjemahkan isi ke dalam bahasa Jepang yang alami. Hapus kata pengisi."),
            ("Perintah terminal", "Ubah ucapan menjadi perintah yang dapat dijalankan di Windows PowerShell. Keluarkan hanya perintahnya, tanpa penjelasan atau pagar kode."),
            ("Gaya bisnis", "Tulis ulang isi dalam bahasa Indonesia bisnis yang sopan dan formal. Hapus kata pengisi, tulis secara ringkas dan santun."),
        ],
        // "en" and any unexpected code
        _ => [
            ("Polish", "Remove fillers (\"um\", \"uh\", etc.) and false starts, fix punctuation and line breaks, and turn the text into natural prose. Do not change the content or meaning. Keep the output in the language the speaker used."),
            ("As-is", ""),
            ("Translate to English", "Translate the content into natural, fluent English. Remove fillers."),
            ("Translate to Japanese", "Translate the content into natural Japanese. Remove fillers."),
            ("Terminal command", "Convert the utterance into a command that can run in Windows PowerShell. Output only the command, with no explanations or code fences."),
            ("Business style", "Rewrite the content in polite, professional business English. Remove fillers and keep it concise and courteous."),
        ],
    }
}
