use crate::diagnostic::DaihonDiagnostic;
use crate::span::Span;
use crate::token::{Token, TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Default,
    Dialogue,
    Function,
    FunctionString,
}

#[derive(Debug, Clone, Copy)]
struct Cursor {
    byte: usize,
    line: usize,
    column: usize,
}

impl Cursor {
    fn span(self, end: Cursor) -> Span {
        Span::new(self.byte, end.byte, self.line, self.column)
    }
}

pub fn lex_source(source: &str) -> Result<Vec<Token>, Vec<DaihonDiagnostic>> {
    let mut lexer = Lexer::new(source);
    lexer.lex();
    if lexer.diagnostics.is_empty() {
        Ok(lexer.tokens)
    } else {
        Err(lexer.diagnostics)
    }
}

struct Lexer<'a> {
    source: &'a str,
    chars: Vec<(usize, char)>,
    index: usize,
    cursor: Cursor,
    tokens: Vec<Token>,
    diagnostics: Vec<DaihonDiagnostic>,
    modes: Vec<(Mode, Cursor)>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.char_indices().collect(),
            index: 0,
            cursor: Cursor {
                byte: 0,
                line: 1,
                column: 1,
            },
            tokens: Vec::new(),
            diagnostics: Vec::new(),
            modes: vec![(
                Mode::Default,
                Cursor {
                    byte: 0,
                    line: 1,
                    column: 1,
                },
            )],
        }
    }

    fn lex(&mut self) {
        while let Some(ch) = self.peek() {
            match self.mode() {
                Mode::Default => self.lex_default(ch),
                Mode::Dialogue => self.lex_dialogue(ch),
                Mode::Function => self.lex_function(ch),
                Mode::FunctionString => self.lex_function_string(ch),
            }
        }

        if let Some((mode, start)) = self.modes.last().copied() {
            match mode {
                Mode::Dialogue | Mode::FunctionString => self.diagnostics.push(
                    DaihonDiagnostic::error(
                        "E-DHN-LEX-001",
                        "セリフが閉じられていません。",
                        start.span(self.cursor),
                    )
                    .with_help("「 で始まったセリフには、対応する 」 が必要です。"),
                ),
                Mode::Function => self.diagnostics.push(
                    DaihonDiagnostic::error(
                        "E-DHN-LEX-002",
                        "関数呼び出しが閉じられていません。",
                        start.span(self.cursor),
                    )
                    .with_help("＜ で始まった関数呼び出しには、対応する ＞ が必要です。"),
                ),
                Mode::Default => {}
            }
        }
    }

    fn mode(&self) -> Mode {
        self.modes
            .last()
            .map(|(mode, _)| *mode)
            .unwrap_or(Mode::Default)
    }

    fn push_mode(&mut self, mode: Mode, start: Cursor) {
        self.modes.push((mode, start));
    }

    fn pop_mode(&mut self) {
        if self.modes.len() > 1 {
            self.modes.pop();
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).map(|(_, ch)| *ch)
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.index + 1).map(|(_, ch)| *ch)
    }

    fn starts_with(&self, needle: &str) -> bool {
        self.source[self.cursor.byte..].starts_with(needle)
    }

    fn advance(&mut self) -> Option<char> {
        let (_, ch) = *self.chars.get(self.index)?;
        self.index += 1;
        self.cursor.byte += ch.len_utf8();
        if ch == '\n' {
            self.cursor.line += 1;
            self.cursor.column = 1;
        } else {
            self.cursor.column += 1;
        }
        Some(ch)
    }

    fn take_while(&mut self, mut predicate: impl FnMut(char) -> bool) -> (String, Span) {
        let start = self.cursor;
        let mut text = String::new();
        while let Some(ch) = self.peek() {
            if !predicate(ch) {
                break;
            }
            text.push(ch);
            self.advance();
        }
        let span = start.span(self.cursor);
        (text, span)
    }

    fn emit(
        &mut self,
        kind: TokenKind,
        original: impl Into<String>,
        normalized: Option<String>,
        span: Span,
    ) {
        self.tokens
            .push(Token::new(kind, original, normalized, span));
    }

    fn emit_char(&mut self, kind: TokenKind, normalized: Option<String>) {
        let start = self.cursor;
        let Some(ch) = self.advance() else {
            return;
        };
        self.emit(kind, ch.to_string(), normalized, start.span(self.cursor));
    }

    fn lex_default(&mut self, ch: char) {
        if is_space(ch) {
            self.advance();
            return;
        }
        if self.starts_with("$$") || self.starts_with("＄＄") {
            self.take_while(|c| c != '\n' && c != '\r');
            return;
        }
        if ch == '\n' || ch == '\r' {
            self.lex_newline();
            return;
        }
        if self.starts_with("###") || self.starts_with("＃＃＃") {
            self.lex_header(TokenKind::SceneHeader, 3);
            return;
        }
        if self.starts_with("##") || self.starts_with("＃＃") {
            self.lex_header(TokenKind::EventHeader, 2);
            return;
        }

        match ch {
            '※' => self.emit_char(TokenKind::ConditionMarker, None),
            '→' => self.emit_char(TokenKind::Arrow, None),
            '「' => {
                let start = self.cursor;
                self.emit_char(TokenKind::DialogueOpen, None);
                self.push_mode(Mode::Dialogue, start);
            }
            '＜' => {
                let start = self.cursor;
                self.emit_char(TokenKind::FunctionOpen, None);
                self.push_mode(Mode::Function, start);
            }
            '@' | '＠' => self.emit_char(TokenKind::At, Some("@".to_owned())),
            ':' | '：' => self.emit_char(TokenKind::Colon, Some(":".to_owned())),
            '.' | '．' => self.emit_char(TokenKind::Dot, Some(".".to_owned())),
            '(' | '（' => self.emit_char(TokenKind::Operator, Some("(".to_owned())),
            ')' | '）' => self.emit_char(TokenKind::Operator, Some(")".to_owned())),
            '=' | '＝' | '!' | '！' | '<' | '>' | '+' | '＋' | '-' | '－' | '*' | '＊' | '/'
            | '／' | '%' | '％' | '~' | '～' | '〜' => self.lex_operator(),
            _ if is_number_start(ch) => self.lex_number_or_time(),
            _ if is_identifier_start(ch) => self.lex_identifier(),
            _ => {
                let start = self.cursor;
                self.advance();
                self.diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-LEX-010",
                    format!("使用できない文字「{ch}」があります。"),
                    start.span(self.cursor),
                ));
            }
        }
    }

    fn lex_header(&mut self, kind: TokenKind, width: usize) {
        let start = self.cursor;
        for _ in 0..width {
            self.advance();
        }
        self.emit(
            kind,
            &self.source[start.byte..self.cursor.byte],
            Some("#".repeat(width)),
            start.span(self.cursor),
        );
        while matches!(self.peek(), Some(c) if is_space(c)) {
            self.advance();
        }
        let name_start = self.cursor;
        let (name, span) = self.take_while(|c| c != '\n' && c != '\r');
        if !name.is_empty() {
            self.emit(
                TokenKind::HeaderName,
                name.clone(),
                Some(name.trim().to_owned()),
                span,
            );
        } else {
            self.emit(
                TokenKind::HeaderName,
                "",
                Some(String::new()),
                name_start.span(self.cursor),
            );
        }
    }

    fn lex_dialogue(&mut self, ch: char) {
        match ch {
            '」' if self.peek_next() == Some('」') => self.lex_escape("」」", "」"),
            '「' if self.peek_next() == Some('「') => self.lex_escape("「「", "「"),
            '＜' if self.peek_next() == Some('＜') => self.lex_escape("＜＜", "＜"),
            '＞' if self.peek_next() == Some('＞') => self.lex_escape("＞＞", "＞"),
            '」' => {
                self.emit_char(TokenKind::DialogueClose, None);
                self.pop_mode();
            }
            '＜' => {
                let start = self.cursor;
                self.emit_char(TokenKind::FunctionOpen, None);
                self.push_mode(Mode::Function, start);
            }
            _ => {
                let (text, span) =
                    self.take_while(|c| c != '」' && c != '「' && c != '＜' && c != '＞');
                if !text.is_empty() {
                    self.emit(TokenKind::DialogueText, text.clone(), Some(text), span);
                }
            }
        }
    }

    fn lex_function(&mut self, ch: char) {
        if is_space(ch) {
            self.advance();
            return;
        }
        match ch {
            '＞' => {
                self.emit_char(TokenKind::FunctionClose, None);
                self.pop_mode();
            }
            '＜' => {
                let start = self.cursor;
                self.emit_char(TokenKind::FunctionOpen, None);
                self.push_mode(Mode::Function, start);
            }
            '「' => {
                let start = self.cursor;
                self.emit_char(TokenKind::DialogueOpen, None);
                self.push_mode(Mode::FunctionString, start);
            }
            '\n' | '\r' => self.lex_newline(),
            '=' | '＝' | '!' | '！' | '<' | '>' | '+' | '＋' | '-' | '－' | '*' | '＊' | '/'
            | '／' | '%' | '％' | '(' | '（' | ')' | '）' => self.lex_operator(),
            _ if is_number_start(ch) => self.lex_number_or_time(),
            _ if is_identifier_start(ch) => self.lex_identifier(),
            _ => {
                let start = self.cursor;
                self.advance();
                self.diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-LEX-011",
                    format!("関数呼び出し内で使用できない文字「{ch}」があります。"),
                    start.span(self.cursor),
                ));
            }
        }
    }

    fn lex_function_string(&mut self, ch: char) {
        match ch {
            '」' if self.peek_next() == Some('」') => self.lex_escape("」」", "」"),
            '「' if self.peek_next() == Some('「') => self.lex_escape("「「", "「"),
            '」' => {
                self.emit_char(TokenKind::DialogueClose, None);
                self.pop_mode();
            }
            _ => {
                let (text, span) = self.take_while(|c| c != '」' && c != '「');
                if !text.is_empty() {
                    self.emit(TokenKind::DialogueText, text.clone(), Some(text), span);
                }
            }
        }
    }

    fn lex_escape(&mut self, original: &str, normalized: &str) {
        let start = self.cursor;
        for _ in original.chars() {
            self.advance();
        }
        self.emit(
            TokenKind::DialogueEscape,
            original,
            Some(normalized.to_owned()),
            start.span(self.cursor),
        );
    }

    fn lex_newline(&mut self) {
        let start = self.cursor;
        if self.peek() == Some('\r') {
            self.advance();
            if self.peek() == Some('\n') {
                self.advance();
            }
        } else {
            self.advance();
        }
        self.emit(
            TokenKind::Newline,
            &self.source[start.byte..self.cursor.byte],
            Some("\n".to_owned()),
            start.span(self.cursor),
        );
    }

    fn lex_number_or_time(&mut self) {
        let start = self.cursor;
        if self.starts_with("0x") || self.starts_with("0o") || self.starts_with("0b") {
            let (text, span) = self.take_while(|c| c.is_ascii_alphanumeric());
            self.emit(
                TokenKind::Number,
                text.clone(),
                Some(normalize_syntax(&text)),
                span,
            );
            return;
        }

        let mut saw_colon = false;
        let mut saw_dot = false;
        while let Some(ch) = self.peek() {
            if is_digit(ch) {
                self.advance();
            } else if matches!(ch, ':' | '：') && !saw_colon {
                saw_colon = true;
                self.advance();
            } else if matches!(ch, '.' | '．') && !saw_dot && !saw_colon {
                saw_dot = true;
                self.advance();
            } else {
                break;
            }
        }
        let text = self.source[start.byte..self.cursor.byte].to_owned();
        let kind = if saw_colon {
            TokenKind::Time
        } else {
            TokenKind::Number
        };
        self.emit(
            kind,
            text.clone(),
            Some(normalize_syntax(&text)),
            start.span(self.cursor),
        );
    }

    fn lex_identifier(&mut self) {
        let (text, span) =
            self.take_while(|c| is_identifier_continue(c) || matches!(c, '#' | '＃'));
        let normalized = normalize_syntax(&text);
        let kind = match normalized.as_str() {
            "はい" | "いいえ" => TokenKind::Boolean,
            "おわり" | "シーンおわり" | "未満" | "以下" | "以上" | "超える" | "かつ" | "または"
            | "でない" | "を含む" | "で始まる" | "で終わる" | "前提条件" | "初期値" | "合図"
            | "条件" | "なら" | "あるいは" | "それ以外" | "優先度" | "重み" | "クールダウン"
            | "頻度" | "話者" => TokenKind::Keyword,
            _ => TokenKind::Identifier,
        };
        self.emit(kind, text, Some(normalized), span);
    }

    fn lex_operator(&mut self) {
        let start = self.cursor;
        let first = self.advance().unwrap_or_default();
        let mut original = first.to_string();
        if matches!(first, '=' | '＝' | '!' | '！' | '<' | '>') && self.peek().is_some() {
            let next = self.peek().unwrap();
            let first_norm = normalize_char(first);
            let next_norm = normalize_char(next);
            if matches!(
                (first_norm, next_norm),
                ('=', '=') | ('!', '=') | ('<', '=') | ('>', '=')
            ) {
                original.push(next);
                self.advance();
            }
        }
        let normalized = normalize_syntax(&original);
        self.emit(
            TokenKind::Operator,
            original,
            Some(normalized),
            start.span(self.cursor),
        );
    }
}

pub(crate) fn normalize_syntax(text: &str) -> String {
    text.chars().map(normalize_char).collect()
}

pub(crate) fn normalize_char(ch: char) -> char {
    match ch {
        '＝' => '=',
        '＃' => '#',
        '：' => ':',
        '＄' => '$',
        '＿' => '_',
        '！' => '!',
        '～' | '〜' => '~',
        '＋' => '+',
        '－' => '-',
        '＊' => '*',
        '／' => '/',
        '％' => '%',
        '＠' => '@',
        '．' => '.',
        '（' => '(',
        '）' => ')',
        '　' => ' ',
        '０'..='９' => char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap_or(ch),
        'Ａ'..='Ｚ' => char::from_u32(ch as u32 - 'Ａ' as u32 + 'A' as u32).unwrap_or(ch),
        'ａ'..='ｚ' => char::from_u32(ch as u32 - 'ａ' as u32 + 'a' as u32).unwrap_or(ch),
        _ => ch,
    }
}

fn is_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '　')
}

fn is_digit(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '０'..='９')
}

fn is_number_start(ch: char) -> bool {
    is_digit(ch)
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_'
        || ch == '＿'
        || ch.is_ascii_alphabetic()
        || matches!(
            ch,
            'ぁ'..='ん' | 'ァ'..='ン' | '一'..='龯' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ' | 'ー'
        )
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || is_digit(ch) || matches!(ch, '‥' | '…' | '.' | 'ー')
}
